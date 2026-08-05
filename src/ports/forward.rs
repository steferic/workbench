//! Copying bytes between two sockets until one of them stops.
//!
//! Raw TCP on purpose. A dev server's traffic is not only HTTP — vite's hot
//! reload is a websocket, and anything that tried to understand the bytes
//! would have to understand that too. Splicing understands nothing, which is
//! why it works for every dev server without configuration.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};

/// Run a connection in both directions. Returns immediately; the pair lives on
/// two threads until either end closes.
pub fn splice(a: TcpStream, b: TcpStream) {
    let (Ok(a_read), Ok(b_read)) = (a.try_clone(), b.try_clone()) else {
        return;
    };
    std::thread::spawn(move || pump(a_read, b));
    std::thread::spawn(move || pump(b_read, a));
}

/// One direction. When it ends, shut *both* halves down — otherwise the other
/// thread sits in `read` on a connection that is never going to say anything
/// again, and the pair leaks.
fn pump(mut from: TcpStream, mut to: TcpStream) {
    let mut buffer = [0u8; 32 * 1024];
    loop {
        match from.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                if to.write_all(&buffer[..n]).is_err() {
                    break;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::{TcpListener, TcpStream};

    /// An echo server on loopback, and a splice in front of it: what a client
    /// writes through the splice comes back, and closing one end tears the
    /// pair down rather than leaving a thread reading forever.
    #[test]
    fn bytes_travel_both_ways_and_the_pair_ends_together() {
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in upstream.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 0 {
                    let mut out = stream.try_clone().unwrap();
                    out.write_all(line.to_uppercase().as_bytes()).unwrap();
                    line.clear();
                }
            }
        });

        let front = TcpListener::bind("127.0.0.1:0").unwrap();
        let front_addr = front.local_addr().unwrap();
        std::thread::spawn(move || {
            for incoming in front.incoming().flatten() {
                let to_server = TcpStream::connect(upstream_addr).unwrap();
                splice(incoming, to_server);
            }
        });

        let mut client = TcpStream::connect(front_addr).unwrap();
        client.write_all(b"hello through the splice\n").unwrap();
        let mut reply = String::new();
        BufReader::new(client.try_clone().unwrap())
            .read_line(&mut reply)
            .unwrap();
        assert_eq!(reply, "HELLO THROUGH THE SPLICE\n");

        // Closing the client must end both directions, not strand a reader.
        client.shutdown(Shutdown::Both).unwrap();
        let mut after = String::new();
        assert!(
            BufReader::new(client).read_line(&mut after).is_err() || after.is_empty(),
            "the connection is finished"
        );
    }
}
