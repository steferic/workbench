use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Path to the on-disk log file, alongside other workbench config.
/// `None` if the config dir cannot be located or created.
fn log_path() -> Option<PathBuf> {
    let dir = dirs::config_dir()?.join("workbench");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("workbench.log"))
}

/// Lazily-opened append-only file handle. Held behind a Mutex so concurrent
/// writes from async tasks don't interleave a single log line.
fn handle() -> Option<&'static Mutex<File>> {
    static HANDLE: OnceLock<Option<Mutex<File>>> = OnceLock::new();
    HANDLE
        .get_or_init(|| {
            let path = log_path()?;
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
                .map(Mutex::new)
        })
        .as_ref()
}

fn write(level: &str, msg: &str) {
    let Some(h) = handle() else {
        return;
    };
    let Ok(mut f) = h.lock() else {
        return;
    };
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    let _ = writeln!(f, "[{}] {} {}", stamp, level, msg);
}

pub fn warn(msg: impl AsRef<str>) {
    write("WARN ", msg.as_ref());
}

#[allow(dead_code)]
pub fn error(msg: impl AsRef<str>) {
    write("ERROR", msg.as_ref());
}

#[allow(dead_code)]
pub fn info(msg: impl AsRef<str>) {
    write("INFO ", msg.as_ref());
}
