use crate::scaffolding::{PacketResponse, TIMEOUT};
use socket2::{Domain, SockAddr, Socket, Type};
use std::io::{Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};
use std::{io, thread};

struct Packet {
    data: Vec<u8>,
    handle: Box<dyn FnOnce(Option<PacketResponse>) + Send + 'static>,
}

pub struct ClientSession {
    channel: mpsc::Sender<Packet>,
    alive: Arc<OnceLock<()>>,
    socket: Socket,
}

impl ClientSession {
    pub fn open(address: IpAddr, port: u16) -> io::Result<ClientSession> {
        let mut socket = Socket::new(
            match address {
                IpAddr::V4(_) => Domain::IPV4,
                IpAddr::V6(_) => Domain::IPV6,
            },
            Type::STREAM,
            None,
        )?;

        socket.set_read_timeout(Some(TIMEOUT))?;
        socket.set_write_timeout(Some(TIMEOUT))?;
        socket.connect_timeout(&SockAddr::from(SocketAddr::new(address, port)), TIMEOUT)?;
        let control_socket = socket.try_clone()?;

        let (sender, receiver) = mpsc::channel::<Packet>();
        let alive = Arc::new(OnceLock::new());
        let alive2 = alive.clone();

        thread::spawn(move || {
            let handle_packet = &mut move |data: Vec<u8>| -> io::Result<PacketResponse> {
                socket.write_all(data.as_ref())?;
                socket.flush()?;

                drop(data);

                let (status, body_size) = {
                    let mut buf = [0u8; 5];
                    socket.read_exact(&mut buf)?;

                    (
                        buf[0],
                        u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize,
                    )
                };

                if body_size > 65536 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Response too large",
                    ));
                }
                let mut data = vec![0u8; body_size];
                socket.read_exact(&mut data)?;

                Ok(if status == 0 {
                    PacketResponse::Ok { data }
                } else {
                    PacketResponse::Fail { status, data }
                })
            };

            while let Ok(packet) = receiver.recv() {
                let Packet { data, handle } = packet;
                match handle_packet(data) {
                    Ok(response) => handle(Some(response)),
                    Err(e) => {
                        logging!("ScaffoldingClient", "Session is closed: {:?}", e);
                        alive.get_or_init(|| ());

                        handle(None);
                        return;
                    }
                }
            }
        });

        Ok(ClientSession {
            channel: sender,
            alive: alive2,
            socket: control_socket,
        })
    }

    pub fn is_alive(&self) -> bool {
        self.alive.get().is_none()
    }

    pub fn send_sync<P>(
        &mut self,
        kind: (&str, &str),
        encoder: P,
        active: impl Fn() -> bool,
    ) -> Option<PacketResponse>
    where
        P: FnOnce(&mut Vec<u8>),
    {
        if !active() || !self.is_alive() {
            return None;
        }
        let (sender, receiver) = mpsc::channel();
        self.send(kind, encoder, move |response| {
            let _ = sender.send(response);
        });
        let started = Instant::now();

        match loop {
            if !active() || started.elapsed() >= TIMEOUT {
                self.alive.get_or_init(|| ());
                let _ = self.socket.shutdown(Shutdown::Both);
                return None;
            }
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(response) => break response,
                Err(mpsc::RecvTimeoutError::Disconnected) => return None,
                Err(mpsc::RecvTimeoutError::Timeout) if !self.is_alive() => {
                    logging!(
                        "ScaffoldingClient",
                        "API {}:{} invocation failed: Session has been closed.",
                        kind.0,
                        kind.1
                    );
                    return None;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
            }
        } {
            Some(PacketResponse::Ok { data }) => Some(PacketResponse::Ok { data }),
            Some(PacketResponse::Fail { status, data }) => {
                logging!(
                    "ScaffoldingClient",
                    "API {}:{} invocation failed with status {}: {}",
                    kind.0,
                    kind.1,
                    status,
                    String::from_utf8_lossy(&data)
                );
                None
            }
            None => {
                logging!(
                    "ScaffoldingClient",
                    "API {}:{} invocation failed: Session has been closed.",
                    kind.0,
                    kind.1
                );
                None
            }
        }
    }

    fn send<P, R>(&mut self, kind: (&str, &str), encoder: P, receiver: R)
    where
        P: FnOnce(&mut Vec<u8>),
        R: FnOnce(Option<PacketResponse>) + Send + 'static,
    {
        let mut data: Vec<u8> = vec![];

        {
            let length = kind.0.len() + kind.1.len() + 1;
            data.push(length as u8);
            data.reserve(length);

            data.extend_from_slice(kind.0.as_bytes());
            data.push(b':');
            data.extend_from_slice(kind.1.as_bytes());
        }
        {
            let pos = data.len();
            data.resize(pos + 4, 0);

            encoder(&mut data);

            let length = data.len() - pos - size_of::<u32>();
            data[pos..pos + size_of::<u32>()].copy_from_slice(&(length as u32).to_be_bytes());
        }

        if self
            .channel
            .send(Packet {
                data,
                handle: Box::new(receiver),
            })
            .is_err()
        {
            self.alive.get_or_init(|| ());
        }
    }
}

impl Drop for ClientSession {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}
