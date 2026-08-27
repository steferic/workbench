//! One forwarded port, as its own process.
//!
//! This existed the moment the killer got a name. Projects free their dev
//! ports with `kill $(lsof -ti:4000)`, and lsof matches every listener on
//! the port number — on any address. While the forwarder lived inside the
//! TUI, that idiom shot workbench dead every time an agent restarted a dev
//! server; the radio-phoenix agent did it on schedule. Now the same bullet
//! hits this process instead: workbench notices on its next scan and simply
//! spawns another one. Dying is part of the job description.
//!
//! Workbench binds the listener itself — so "address in use" still surfaces
//! where the log can say so — and hands it over as fd 3. Stdin is the
//! lifeline: when workbench exits, the pipe closes and this process leaves
//! with it, the same contract the ffmpeg guard uses. No arguments beyond the
//! upstream address, no dependencies, nothing to configure.

#[cfg(unix)]
fn main() {
    use std::io::Read;
    use std::net::{SocketAddr, TcpStream};
    use std::os::unix::io::FromRawFd;

    let Some(upstream) = std::env::args().nth(1) else {
        std::process::exit(64);
    };
    let Ok(upstream) = upstream.parse::<SocketAddr>() else {
        std::process::exit(64);
    };

    // The listener workbench bound and passed down. Nothing else is fd 3.
    // SAFETY: the parent dup2'd a listening socket onto fd 3 before exec.
    let listener = unsafe { std::net::TcpListener::from_raw_fd(3) };

    // The lifeline: workbench holds the write end and never writes. EOF
    // means it is gone, and a forwarder must not outlive the app that owns
    // it — orphaned processes were half of what sank this machine.
    std::thread::spawn(|| {
        let mut sink = [0u8; 64];
        let mut stdin = std::io::stdin();
        loop {
            match stdin.read(&mut sink) {
                Ok(0) | Err(_) => std::process::exit(0),
                Ok(_) => {}
            }
        }
    });

    for incoming in listener.incoming() {
        let Ok(from_phone) = incoming else { continue };
        // Dialled per connection, so a dev server that restarts is picked up
        // with no bookkeeping, and one that is gone refuses as it would
        // locally.
        let Ok(to_server) = TcpStream::connect(upstream) else {
            continue;
        };
        splice(from_phone, to_server);
    }
}

#[cfg(not(unix))]
fn main() {}

/// `ports::forward::splice`, standalone. Duplicated rather than shared: this
/// binary follows wbhook's rule of depending on nothing, including the crate.
#[cfg(unix)]
fn splice(a: std::net::TcpStream, b: std::net::TcpStream) {
    let (Ok(a_read), Ok(b_read)) = (a.try_clone(), b.try_clone()) else {
        return;
    };
    std::thread::spawn(move || pump(a_read, b));
    std::thread::spawn(move || pump(b_read, a));
}

#[cfg(unix)]
fn pump(mut from: std::net::TcpStream, mut to: std::net::TcpStream) {
    use std::io::{Read, Write};
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
    let _ = from.shutdown(std::net::Shutdown::Both);
    let _ = to.shutdown(std::net::Shutdown::Both);
}
