use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

/// 加入者广播的 MOTD 前缀；世界发现用它排除自己重播的大厅。
pub const LOBBY_MOTD_PREFIX: &str = "PCL.Proto 大厅";

pub struct FakeServer {
    pub port: u16,
    _holder: Sender<()>,
}

impl FakeServer {
    /// `motd` 形如 `PCL.Proto 大厅 - {房主名}`，拿不到房主名时退化为
    /// `PCL.Proto 大厅`。
    pub fn create(port: u16, motd: String) -> FakeServer {
        let (tx, rx) = mpsc::channel::<()>();
        thread::spawn(move || run(port, motd, rx));

        FakeServer { port, _holder: tx }
    }
}

/// 判断一条局域网广播 MOTD 是否来自联机大厅（自身加入者的回放）。
///
/// 世界扫描必须排除这些广播，否则加入者会把自己重播的假服务器
/// 当成一个可联机的世界。
pub fn is_lobby_motd(motd: &str) -> bool {
    let motd = motd.trim();
    motd.starts_with(LOBBY_MOTD_PREFIX) || motd == crate::MOTD
}

fn run(port: u16, motd: String, signal: Receiver<()>) {
    let (socket, target) = match broadcast_socket() {
        Ok(pair) => pair,
        Err(error) => {
            logging!("FakeServer", "Cannot bind loopback socket: {:?}", error);
            return;
        }
    };

    let message = format!("[MOTD]{}[/MOTD][AD]{}[/AD]", motd, port);

    loop {
        if let Err(mpsc::TryRecvError::Disconnected) = signal.try_recv() {
            return;
        }

        if let Err(error) = socket.send_to(message.as_bytes(), target) {
            logging!("FakeServer", "Cannot broadcast lobby: {:?}", error);
        }

        thread::sleep(Duration::from_millis(1500));
    }
}

fn broadcast_socket() -> std::io::Result<(UdpSocket, SocketAddr)> {
    #[cfg(target_os = "windows")]
    {
        Ok((
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))?,
            SocketAddr::from((Ipv4Addr::LOCALHOST, 4445)),
        ))
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Unix delivers unicast to only one of the sockets sharing Minecraft's port.
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        socket.set_multicast_loop_v4(true)?;
        socket.set_multicast_ttl_v4(0)?;
        Ok((
            socket,
            SocketAddr::from((Ipv4Addr::new(224, 0, 2, 60), 4445)),
        ))
    }
}
