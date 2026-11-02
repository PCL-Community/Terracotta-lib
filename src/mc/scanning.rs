use socket2::{Domain, SockAddr, Socket, Type};
use std::borrow::Cow;
use std::io::Result;
use std::mem::MaybeUninit;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};
use std::str::FromStr;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::{mem, thread};

pub struct MinecraftScanner {
    port: Arc<Mutex<Vec<u16>>>,
    _holder: Sender<()>,
}

fn advertised_port(data: &str, filter: fn(&str) -> bool) -> Option<u16> {
    let (_, motd) = data.split_once("[MOTD]")?;
    let (motd, _) = motd.split_once("[/MOTD]")?;
    if motd.is_empty() || !filter(motd) {
        return None;
    }
    let (_, port) = data.split_once("[AD]")?;
    let (port, _) = port.split_once("[/AD]")?;
    port.parse().ok().filter(|port| *port != 0)
}

fn update_ports(ports: &mut Vec<(u16, Instant)>, port: Option<u16>, now: Instant) -> bool {
    let old: Vec<_> = ports.iter().map(|(port, _)| *port).collect();
    ports.retain(|(_, seen)| now.saturating_duration_since(*seen) < Duration::from_secs(5));
    if let Some(port) = port {
        if let Some((_, seen)) = ports.iter_mut().find(|(existing, _)| *existing == port) {
            *seen = now;
        } else {
            ports.push((port, now));
        }
    }
    old != ports.iter().map(|(port, _)| *port).collect::<Vec<_>>()
}

impl MinecraftScanner {
    pub fn create(filter: fn(&str) -> bool) -> MinecraftScanner {
        let (tx, rx) = mpsc::channel::<()>();
        let port = Arc::new(Mutex::new(vec![]));

        let port_cloned = Arc::clone(&port);
        thread::spawn(move || {
            let result = Self::run(rx, port_cloned, filter);

            match result {
                Ok(_) => {}
                Err(err) => {
                    logging!("Server Scanner", "Cannot scan: {}", err);
                }
            }
        });

        return MinecraftScanner { _holder: tx, port };
    }

    fn run(
        signal: Receiver<()>,
        output: Arc<Mutex<Vec<u16>>>,
        filter: fn(&str) -> bool,
    ) -> Result<()> {
        let sockets: Vec<(Socket, &IpAddr)> = crate::ADDRESSES
            .iter()
            .map(|address| match address {
                IpAddr::V4(ip) => {
                    let socket = Socket::new(Domain::IPV4, Type::DGRAM, None)?;
                    socket.set_reuse_address(true)?;
                    socket.bind(&SockAddr::from(SocketAddrV4::new(*ip, 4445)))?;
                    socket.join_multicast_v4(
                        &Ipv4Addr::from_str("224.0.2.60").unwrap(),
                        &Ipv4Addr::UNSPECIFIED,
                    )?;
                    socket.set_read_timeout(Some(Duration::from_millis(500)))?;
                    Ok((socket, address))
                }
                IpAddr::V6(ip) => {
                    let socket = Socket::new(Domain::IPV6, Type::DGRAM, None)?;
                    socket.set_only_v6(true)?;
                    socket.set_reuse_address(true)?;
                    socket.bind(&SockAddr::from(SocketAddrV6::new(*ip, 4445, 0, 0)))?;
                    socket.join_multicast_v6(&Ipv6Addr::from_str("FF75:230::60").unwrap(), 0)?;
                    socket.set_read_timeout(Some(Duration::from_millis(500)))?;
                    Ok((socket, address))
                }
            })
            .filter_map(|r: Result<(Socket, &IpAddr)>| r.ok())
            .collect();

        if sockets.is_empty() {
            return Err(std::io::Error::other("no multicast sockets available"));
        }

        logging!(
            "Server Scanner",
            "Starting server scanner at IP: {:?}",
            sockets.iter().map(|p| p.1).collect::<Vec<_>>()
        );

        let mut buf: [MaybeUninit<u8>; _] = [MaybeUninit::uninit(); 8192];

        let mut ports: Vec<(u16, Instant)> = vec![];

        loop {
            if let Err(mpsc::TryRecvError::Disconnected) = signal.try_recv() {
                return Ok(());
            }

            let mut dirty = update_ports(&mut ports, None, Instant::now());

            for (socket, _) in sockets.iter() {
                if let Ok((length, _)) = socket.recv_from(&mut buf) {
                    // SAFETY: 0..length has been initialized by recv_from.
                    let buf =
                        unsafe { mem::transmute::<&[MaybeUninit<u8>], &[u8]>(&buf[..length]) };

                    let data: Cow<'_, str> = String::from_utf8_lossy(buf);
                    if let Some(port) = advertised_port(&data, filter) {
                        dirty |= update_ports(&mut ports, Some(port), Instant::now());
                        break;
                    }
                }
            }

            if dirty {
                let mut output = output.lock().unwrap();
                *output = ports.iter().map(|(port, _)| *port).collect();
                logging!("Server Scanner", "Updating server list to {:?}", *output);
            }
        }
    }

    pub fn get_ports(&self) -> Vec<u16> {
        return self.port.lock().unwrap().clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_or_filtered_advertisements() {
        let filter = |motd: &str| motd != "self";
        assert_eq!(
            advertised_port("[MOTD]world[/MOTD][AD]25565[/AD]", filter),
            Some(25565)
        );
        for data in [
            "[/MOTD][MOTD]world[AD]1[/AD]",
            "[MOTD]world[/MOTD][/AD][AD]1",
            "[MOTD]self[/MOTD][AD]1[/AD]",
            "[MOTD]world[/MOTD][AD]0[/AD]",
            "[MOTD]world[/MOTD][AD]65536[/AD]",
        ] {
            assert_eq!(advertised_port(data, filter), None, "{data}");
        }
    }

    #[test]
    fn tracks_multiple_ports_refresh_and_expiration() {
        let now = Instant::now();
        let mut ports = Vec::new();
        assert!(update_ports(&mut ports, Some(1000), now));
        assert!(update_ports(&mut ports, Some(2000), now));
        assert_eq!(
            ports.iter().map(|p| p.0).collect::<Vec<_>>(),
            vec![1000, 2000]
        );
        assert!(!update_ports(
            &mut ports,
            Some(1000),
            now + Duration::from_secs(3)
        ));
        assert!(update_ports(&mut ports, None, now + Duration::from_secs(5)));
        assert_eq!(ports.iter().map(|p| p.0).collect::<Vec<_>>(), vec![1000]);
        assert!(update_ports(&mut ports, None, now + Duration::from_secs(8)));
        assert!(ports.is_empty());
    }
}
