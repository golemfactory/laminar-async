use actix_rt::System;
use anyhow::Result;
use crossbeam_channel::{unbounded, Receiver, SendError, Sender, TryRecvError};
use futures::io::{Error, ErrorKind};
use futures::StreamExt;
use laminar::{Config, DatagramSocket, Packet, SocketEvent};
use laminar_async::net::socket::StreamSocket;
use std::cmp::min;
use std::fmt::Formatter;
use std::net::SocketAddr;
use std::thread;
use std::time::Duration;
use tokio::time::delay_for;

type Data = Vec<u8>;

struct VirtualSocket {
    tx: Sender<Data>,
    rx: Receiver<Data>,
    local_sa: SocketAddr,
    remote_sa: SocketAddr,
    blocking_mode: bool,
}

impl std::fmt::Debug for VirtualSocket {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        let repr = format!("VirtualSocket({:?})", self.local_sa);
        f.write_str(&repr)
    }
}

impl VirtualSocket {
    fn new(
        tx: Sender<Data>,
        rx: Receiver<Data>,
        local_sa: SocketAddr,
        remote_sa: SocketAddr,
        blocking_mode: bool,
    ) -> Self {
        VirtualSocket {
            tx,
            rx,
            local_sa,
            remote_sa,
            blocking_mode,
        }
    }
}

fn map_send_err<T: 'static + Send + Sync>(e: SendError<T>) -> Error {
    Error::new(ErrorKind::Other, e)
}

fn map_recv_err(e: TryRecvError) -> Error {
    match e {
        TryRecvError::Empty => Error::new(ErrorKind::WouldBlock, e),
        _ => Error::new(ErrorKind::Other, e),
    }
}

impl DatagramSocket for VirtualSocket {
    fn send_packet(&mut self, _addr: &SocketAddr, payload: &[u8]) -> Result<usize, Error> {
        let size = payload.len();
        self.tx
            .send(payload.to_vec())
            .map(|_| size)
            .map_err(map_send_err)
            .unwrap();
        Ok(size)
    }

    fn receive_packet<'a>(
        &mut self,
        buffer: &'a mut [u8],
    ) -> Result<(&'a [u8], SocketAddr), Error> {
        let mut received = self.rx.try_recv().map_err(map_recv_err)?;
        let len = min(buffer.len(), received.len());
        let src = &received.as_mut_slice()[..len];

        buffer[..len].copy_from_slice(src);
        Ok((&buffer[..len], self.remote_sa.clone()))
    }

    fn local_addr(&self) -> Result<SocketAddr, Error> {
        Ok(self.local_sa.clone())
    }

    fn is_blocking_mode(&self) -> bool {
        self.blocking_mode
    }
}

fn build_config() -> Config {
    let mut config = Config::default();
    config.idle_connection_timeout = Duration::from_secs(5u64);
    config.heartbeat_interval = Some(Duration::from_secs(1u64));
    config
}

fn build_sa(port: u16) -> SocketAddr {
    let addr = format!("127.0.0.1:{}", port);
    SocketAddr::V4(addr.parse().unwrap())
}

fn build_packet<'a>(name: &'a str, to_addr: &SocketAddr, counter: u32) -> Packet {
    let data = format!("From {}: {}", name, counter).into_bytes();
    Packet::reliable_ordered(to_addr.clone(), data, Some(0))
}

fn poll_socket(mut sock: StreamSocket<VirtualSocket>) {
    let name = format!("{:?}", sock.socket());
    System::new(&name).block_on(async move {
        let mut counter = 0u32;
        log::info!("Spawning {}", name);

        sock.send(build_packet(&name, &sock.socket().remote_sa, counter))
            .unwrap();
        counter += 1;

        loop {
            match sock.into_future().await {
                (Some(event), s) => {
                    sock = s;
                    match event {
                        SocketEvent::Packet(packet) => {
                            counter += 1;
                            let string = std::str::from_utf8(packet.payload()).unwrap();
                            log::info!("[{}] Packet: {:?}", name, string);

                            sock.send(build_packet(&name, &sock.socket().remote_sa, counter))
                                .unwrap();
                        }
                        SocketEvent::Connect(addr) => {
                            log::info!("[{}] Connect: {:?}", name, addr);
                        }
                        SocketEvent::Timeout(addr) => {
                            log::info!("[{}] Timeout: {:?}", name, addr);
                        }
                    }
                    continue;
                }
                (_, s) => {
                    sock = s;
                }
            }

            delay_for(Duration::from_millis(100)).await;
        }
    });
}

#[actix_rt::main]
async fn main() -> Result<()> {
    env_logger::init();
    let (tx_1, rx_1) = unbounded();
    let (tx_2, rx_2) = unbounded();

    let addr_1 = build_sa(1);
    let addr_2 = build_sa(2);

    let vs_1 = VirtualSocket::new(tx_2, rx_1, addr_1.clone(), addr_2.clone(), false);
    let sock_1 = StreamSocket::new(vs_1, build_config());

    let vs_2 = VirtualSocket::new(tx_1, rx_2, addr_2.clone(), addr_1.clone(), false);
    let sock_2 = StreamSocket::new(vs_2, build_config());

    thread::spawn(move || {
        poll_socket(sock_1);
    });
    thread::spawn(move || {
        poll_socket(sock_2);
    });

    delay_for(Duration::from_secs(30)).await;
    Ok(())
}
