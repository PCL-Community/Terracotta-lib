use socket2::{Domain, SockAddr, Socket, Type};
use std::mem::MaybeUninit;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::thread;
use std::time::{Duration, SystemTime};

pub fn check_mc_conn(port: u16) -> bool {
    let start = SystemTime::now();

    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(64)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(64)))
        .unwrap();
    if let Ok(_) = socket.connect_timeout(
        &SockAddr::from(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), port)),
        Duration::from_secs(64),
    ) && let Ok(_) = socket.send(&[0xFE])
    {
        let mut buf: [MaybeUninit<u8>; _] = [MaybeUninit::uninit(); 1];

        if let Ok(size) = socket.recv(&mut buf) && size >= 1
            // SAFETY: The first byte has been initialized by recv, as size >= 1
            && unsafe { buf[0].assume_init() } == 0xFF
        {
            return true;
        }
    }

    thread::sleep(
        (start + Duration::from_secs(5))
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO),
    );
    false
}