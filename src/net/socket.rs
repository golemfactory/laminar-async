use crossbeam_channel::{Receiver, Sender, TryRecvError};
use futures::task::{Context, Poll};
use futures::Stream;
#[cfg(feature = "tester")]
use laminar::LinkConditioner;
use laminar::{
    Config, ConnectionManager, DatagramSocket, Packet, Result, SocketEvent, VirtualConnection,
};
use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Instant;

pub struct StreamSocket<DS: DatagramSocket> {
    handler: ConnectionManager<DS, VirtualConnection>,
}

impl<DS: DatagramSocket> StreamSocket<DS> {
    pub fn new(sock: DS, config: Config) -> Self {
        Self {
            handler: ConnectionManager::new(sock, config),
        }
    }

    pub fn socket(&self) -> &DS {
        self.handler.socket()
    }

    pub fn socket_mut(&mut self) -> &mut DS {
        self.handler.socket_mut()
    }

    /// Returns a handle to the packet sender which provides a thread-safe way to enqueue packets
    /// to be processed. This should be used when the socket is busy running its polling loop in a
    /// separate thread.
    pub fn get_packet_sender(&mut self) -> Sender<Packet> {
        self.handler.event_sender().clone()
    }

    /// Returns a handle to the event receiver which provides a thread-safe way to retrieve events
    /// from the socket. This should be used when the socket is busy running its polling loop in
    /// a separate thread.
    pub fn get_event_receiver(&mut self) -> Receiver<SocketEvent> {
        self.handler.event_receiver().clone()
    }

    /// Sends a single packet
    pub fn send(&mut self, packet: Packet) -> Result<()> {
        self.handler
            .event_sender()
            .send(packet)
            .expect("Receiver must exist.");
        Ok(())
    }

    /// Receives a single packet
    pub fn recv(&mut self) -> Option<SocketEvent> {
        match self.handler.event_receiver().try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => panic!["This can never happen"],
        }
    }

    /// Processes any inbound/outbound packets and handle idle clients
    pub fn manual_poll(&mut self, time: Instant) {
        self.handler.manual_poll(time);
    }

    /// Returns the local socket address
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.handler.socket().local_addr()?)
    }

    /// Sets the link conditioner for this socket. See [LinkConditioner] for further details.
    #[cfg(feature = "tester")]
    pub fn set_link_conditioner(&mut self, link_conditioner: Option<LinkConditioner>) {
        self.handler.set_link_conditioner(link_conditioner)
    }
}

impl<DS: DatagramSocket> Stream for StreamSocket<DS> {
    type Item = SocketEvent;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let self_mut = self.get_mut();
        self_mut.manual_poll(Instant::now());
        match self_mut.recv() {
            Some(event) => Poll::Ready(Some(event)),
            None => Poll::Ready(None),
        }
    }
}

impl<DS: DatagramSocket> Unpin for StreamSocket<DS> {}
