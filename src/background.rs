//! A detachable client for the existing Workbench runtime. The runtime owns
//! its terminal and keeps processing agent, control socket and phone events
//! while no client is attached.
use anyhow::{Context, Result, bail};
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::{
    fs::{OpenOptionsExt, PermissionsExt},
    net::{UnixListener, UnixStream},
};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RESIZE: u8 = 1;
const INPUT: u8 = 2;
const DETACH: u8 = 0x1c;
const MODES_ON: &[u8] =
    b"\x1b[?1049h\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h\x1b[?2004h\x1b[?25l";
const MODES_OFF: &[u8] = b"\x1b[?2004l\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1049l\x1b[?25h\x1b[0m";

fn runtime_dir() -> Result<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let mut root = dirs::config_dir()
        .context("no config directory")?
        .join("workbench");
    if root.as_os_str().len() > 80 {
        root =
            std::env::temp_dir().join(format!("workbench-runtime-{}", unsafe { libc::geteuid() }));
    }
    let dir = root.join("runtime");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    let info = std::fs::symlink_metadata(&dir)?;
    if !info.is_dir() || info.uid() != unsafe { libc::geteuid() } || info.mode() & 0o077 != 0 {
        bail!(
            "Workbench runtime directory must be private: {}",
            dir.display()
        );
    }
    Ok(dir)
}

pub fn running() -> bool {
    runtime_dir().ok().is_some_and(|dir| {
        dir.join("ui.sock").exists()
            && open_lock(&dir.join("lock"))
                .and_then(|lock| try_lock(&lock))
                .is_ok_and(|available| !available)
    })
}

pub fn lock_state() -> Result<File> {
    let lock = open_lock(&runtime_dir()?.join("state.lock"))?;
    if !try_lock(&lock)? {
        bail!("Workbench already owns this state. Reattach with workbench --persistent.");
    }
    Ok(lock)
}

fn open_lock(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
}

fn try_lock(lock: &File) -> io::Result<bool> {
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let err = io::Error::last_os_error();
    if err.kind() == io::ErrorKind::WouldBlock {
        Ok(false)
    } else {
        Err(err)
    }
}

pub fn attach(workspace: Option<PathBuf>) -> Result<()> {
    let (cols, rows) = crossterm::terminal::size().context("detachable mode needs a terminal")?;
    let path = runtime_dir()?.join("ui.sock");
    let mut socket = match UnixStream::connect(&path) {
        Ok(socket) => {
            if workspace.is_some() {
                // Dropping an accepted socket releases its client slot.
                bail!("Pass --workspace when starting a new background runtime");
            }
            socket
        }
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            let mut command = Command::new(std::env::current_exe()?);
            command.args(["background-runtime", &cols.to_string(), &rows.to_string()]);
            if let Some(workspace) = workspace {
                command.arg("--workspace").arg(workspace);
            }
            let mut child = command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Ok(socket) = UnixStream::connect(&path) {
                    break socket;
                }
                if child.try_wait()?.is_some_and(|status| !status.success())
                    || Instant::now() >= deadline
                {
                    bail!("Workbench background runtime failed to start");
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        Err(err) => return Err(err.into()),
    };
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut accepted = [0];
    socket.read_exact(&mut accepted)?;
    socket.set_read_timeout(None)?;
    if accepted != [1] {
        bail!("Workbench already has an attached client");
    }

    let mut tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    crossterm::terminal::enable_raw_mode()?;
    let _guard = RestoreTerminal;
    tty.write_all(MODES_ON)?;
    send_resize(&mut socket, cols, rows)?;
    relay(&mut socket, &mut tty, (cols, rows))
}

struct RestoreTerminal;
impl Drop for RestoreTerminal {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        if let Ok(mut tty) = OpenOptions::new().write(true).open("/dev/tty") {
            let _ = tty.write_all(MODES_OFF);
        }
    }
}

fn send_resize(socket: &mut UnixStream, cols: u16, rows: u16) -> io::Result<()> {
    let [x1, x2] = cols.to_be_bytes();
    let [y1, y2] = rows.to_be_bytes();
    socket.write_all(&[RESIZE, x1, x2, y1, y2])
}

fn relay(socket: &mut UnixStream, tty: &mut File, mut size: (u16, u16)) -> Result<()> {
    let mut poll = [
        libc::pollfd {
            fd: socket.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: tty.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let mut buf = [0; 8192];
    loop {
        let ready = unsafe { libc::poll(poll.as_mut_ptr(), 2, 100) };
        if ready < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err.into());
        }
        if poll[0].revents != 0 {
            let count = socket.read(&mut buf)?;
            if count == 0 {
                return Ok(());
            }
            tty.write_all(&buf[..count])?;
        }
        if poll[1].revents != 0 {
            let count = tty.read(&mut buf)?;
            if count == 0 {
                return Ok(());
            }
            let input = &buf[..count];
            let before_detach = input
                .iter()
                .position(|byte| *byte == DETACH)
                .unwrap_or(count);
            if before_detach != 0 {
                let [high, low] = (before_detach as u16).to_be_bytes();
                socket.write_all(&[INPUT, high, low])?;
                socket.write_all(&input[..before_detach])?;
            }
            if before_detach != count {
                return Ok(());
            }
        }
        if let Ok(current) = crossterm::terminal::size() {
            if current != size {
                send_resize(socket, current.0, current.1)?;
                size = current;
            }
        }
    }
}

pub fn serve(cols: u16, rows: u16, workspace: Option<&Path>) -> Result<()> {
    if unsafe { libc::setsid() } < 0 {
        return Err(io::Error::last_os_error().into());
    }
    let dir = runtime_dir()?;
    let lock = open_lock(&dir.join("lock"))?;
    if !try_lock(&lock)? {
        return Ok(());
    }
    let path = dir.join("ui.sock");
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let mut command = CommandBuilder::new(std::env::current_exe()?);
    command.arg("--foreground");
    if let Some(path) = workspace {
        command.arg("--workspace");
        command.arg(path);
    }
    command.cwd(std::env::current_dir()?);
    command.env("WORKBENCH_BACKGROUND_TUI", "1");
    let result = serve_terminal(listener, pty_size(cols, rows), command);
    let _ = std::fs::remove_file(&path);
    result
}

fn pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

type Client = Arc<Mutex<Option<UnixStream>>>;

fn serve_terminal(listener: UnixListener, size: PtySize, command: CommandBuilder) -> Result<()> {
    let pair = native_pty_system().openpty(size)?;
    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let mut child = pair.slave.spawn_command(command)?;
    drop(pair.slave);
    let runtime_pid = child.process_id();

    let client = Client::default();
    let output = Arc::clone(&client);
    std::thread::spawn(move || {
        let mut buf = [0; 8192];
        while let Ok(count) = reader.read(&mut buf) {
            if count == 0 {
                break;
            }
            let mut slot = output.lock().unwrap();
            if let Some(socket) = slot.as_mut() {
                if socket.write_all(&buf[..count]).is_err() {
                    let _ = socket.shutdown(std::net::Shutdown::Both);
                }
            }
        }
    });

    let master = Arc::new(Mutex::new(pair.master));
    let writer = Arc::new(Mutex::new(writer));
    std::thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            let _ = socket.set_write_timeout(Some(Duration::from_secs(1)));
            let mut slot = client.lock().unwrap();
            if slot.is_some() {
                let _ = socket.write_all(&[0]);
                continue;
            }
            let Ok(output) = socket.try_clone() else {
                continue;
            };
            if socket.write_all(&[1]).is_err() {
                continue;
            }
            *slot = Some(output);
            let client = Arc::clone(&client);
            let master = Arc::clone(&master);
            let writer = Arc::clone(&writer);
            std::thread::spawn(move || {
                let _ = read_client(&mut socket, &master, &writer, runtime_pid);
                client.lock().unwrap().take();
            });
        }
    });

    child.wait()?;
    Ok(())
}

fn read_client(
    socket: &mut UnixStream,
    master: &Mutex<Box<dyn MasterPty + Send>>,
    writer: &Mutex<Box<dyn Write + Send>>,
    runtime_pid: Option<u32>,
) -> Result<()> {
    let mut first_resize = true;
    let mut kind = [0];
    while socket.read_exact(&mut kind).is_ok() {
        match kind[0] {
            RESIZE => {
                let mut size = [0; 4];
                socket.read_exact(&mut size)?;
                let cols = u16::from_be_bytes([size[0], size[1]]);
                let rows = u16::from_be_bytes([size[2], size[3]]);
                master.lock().unwrap().resize(pty_size(cols, rows))?;
                if first_resize {
                    // Same-size attachments still need a full repaint.
                    if let Some(pid) = runtime_pid {
                        unsafe { libc::kill(pid as libc::pid_t, libc::SIGWINCH) };
                    }
                    first_resize = false;
                }
            }
            INPUT => {
                let mut len = [0; 2];
                socket.read_exact(&mut len)?;
                let mut input = vec![0; u16::from_be_bytes(len) as usize];
                socket.read_exact(&mut input)?;
                writer.lock().unwrap().write_all(&input)?;
            }
            _ => bail!("invalid Workbench client frame"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect(path: &Path) -> UnixStream {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let mut socket = UnixStream::connect(path).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut ack = [0];
            socket.read_exact(&mut ack).unwrap();
            if ack == [1] {
                return socket;
            }
            assert!(
                Instant::now() < deadline,
                "the previous client did not detach"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn input(socket: &mut UnixStream, bytes: &[u8]) {
        let [high, low] = (bytes.len() as u16).to_be_bytes();
        socket.write_all(&[INPUT, high, low]).unwrap();
        socket.write_all(bytes).unwrap();
    }

    fn read_marker(socket: &mut UnixStream, marker: &[u8]) {
        let mut seen = Vec::new();
        while !seen.ends_with(marker) {
            let mut byte = [0];
            socket.read_exact(&mut byte).unwrap();
            seen.push(byte[0]);
        }
    }

    #[test]
    fn same_pty_process_keeps_running_and_accepts_input_after_reattach() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui");
        let listener = UnixListener::bind(&path).unwrap();
        let marker = dir.path().join("detached");
        let mut command = CommandBuilder::new("/bin/sh");
        command.args([
            "-c",
            r#"
            stty -echo
            read first
            printf 'ready'
            while [ ! -e "$1" ]; do sleep 0.02; done
            printf 'continued' > "$2"
            read second
            printf 'result:%s:%s' "$first" "$second"
            read quit
        "#,
            "test",
        ]);
        let advance = dir.path().join("advance");
        command.arg(&advance);
        command.arg(&marker);
        let server =
            std::thread::spawn(move || serve_terminal(listener, pty_size(80, 24), command));
        let mut first = connect(&path);
        send_resize(&mut first, 80, 24).unwrap();
        input(&mut first, b"one\n");
        read_marker(&mut first, b"ready");
        drop(first);
        std::fs::write(advance, []).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !marker.exists() {
            assert!(Instant::now() < deadline, "the detached process stopped");
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut second = connect(&path);
        send_resize(&mut second, 100, 30).unwrap();
        input(&mut second, b"two\n");
        read_marker(&mut second, b"result:one:two");
        input(&mut second, b"quit\n");
        server.join().unwrap().unwrap();
    }
}
