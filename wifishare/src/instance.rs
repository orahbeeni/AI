//! One window per machine: a second launch asks the running one to show itself and exits.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Local-only port next to the sharing port.
fn addr(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port.wrapping_add(1)))
}

/// True if another wifishare window is running here; it has been asked to come forward.
pub fn signal_existing(port: u16) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&addr(port), Duration::from_millis(300)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    if s.write_all(b"wifishare show\n").is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply).is_ok() && reply.trim() == "ok"
}

/// Listens for later launches and forwards their "show" requests.
pub fn listen<T: Send + 'static>(port: u16, tx: Sender<T>, show: T, wake: impl Fn() + Send + 'static)
where
    T: Clone,
{
    let Ok(listener) = TcpListener::bind(addr(port)) else {
        log::debug!("single-instance port {} is busy", port.wrapping_add(1));
        return;
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let mut reader = BufReader::new(&stream);
            if reader.read_line(&mut line).is_ok() && line.trim() == "wifishare show" {
                let _ = (&stream).write_all(b"ok\n");
                if tx.send(show.clone()).is_err() {
                    return;
                }
                wake();
            }
        }
    });
}
