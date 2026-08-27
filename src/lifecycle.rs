//! Whether the last run ended on purpose.
//!
//! SIGKILL leaves no trace from the inside: no panic hook, no Drop, no last
//! words. So the trace is left the other way round — a marker written at
//! boot, rewritten at every clean exit. A boot that finds the previous
//! marker still saying "running" knows that instance never got to say
//! goodbye, and writes the accusation into the log where the health
//! heartbeats already are. Between the two, the log now answers on its own
//! when it died and how it was doing at the time.

use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};

/// The last termination signal received, and who sent it. Written by the
/// signal handler, drained by the event loop. Zero / -1 mean "nothing yet".
static TERM_SIGNAL: AtomicI32 = AtomicI32::new(0);
static TERM_SENDER: AtomicI32 = AtomicI32::new(-1);

/// SA_SIGINFO handler: record the signal and the sender's pid, nothing else.
/// Only atomic stores — the async-signal-safe set is tiny, and a handler that
/// allocates or locks can deadlock the very process it is trying to inform.
#[cfg(unix)]
extern "C" fn on_termination(
    signal: libc::c_int,
    info: *mut libc::siginfo_t,
    _context: *mut libc::c_void,
) {
    if !info.is_null() {
        // SAFETY: the kernel hands a valid siginfo_t to an SA_SIGINFO handler.
        TERM_SENDER.store(unsafe { (*info).si_pid }, Ordering::SeqCst);
    }
    TERM_SIGNAL.store(signal, Ordering::SeqCst);
}

/// Catch SIGTERM and SIGHUP so a polite kill becomes a clean shutdown with
/// the sender's name in the log, instead of an abrupt death with no trace.
///
/// This exists because something on this machine SIGTERMs workbench when
/// agents start — established by the launcher's exit-status capture after
/// the kernel, the panic hook, and every journal came up empty. The signal
/// itself says who: SA_SIGINFO carries the sender's pid.
#[cfg(unix)]
pub fn watch_termination() {
    // SAFETY: installing a handler whose body is two atomic stores.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_termination as usize;
        action.sa_flags = libc::SA_SIGINFO;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGHUP, &action, std::ptr::null_mut());
    }
}

#[cfg(not(unix))]
pub fn watch_termination() {}

/// The termination asked of us, if any — with the asker identified while its
/// pid is still warm. Drains the pending signal, so one request is one notice.
pub fn termination_notice() -> Option<String> {
    let signal = TERM_SIGNAL.swap(0, Ordering::SeqCst);
    if signal == 0 {
        return None;
    }
    let sender = TERM_SENDER.swap(-1, Ordering::SeqCst);
    let name = match signal {
        #[cfg(unix)]
        libc::SIGTERM => "SIGTERM",
        #[cfg(unix)]
        libc::SIGHUP => "SIGHUP",
        other => return Some(format!("signal {other} from {}", describe_pid(sender))),
    };
    Some(format!("{name} from {}", describe_pid(sender)))
}

/// Best effort: the sender may be `pkill`, gone before we can ask its name —
/// but even a bare pid places the killer on the timeline.
fn describe_pid(pid: i32) -> String {
    if pid <= 0 {
        return "the kernel or an unknown sender".to_string();
    }
    let looked_up = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "pid=,ppid=,command="])
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|line| !line.is_empty());
    match looked_up {
        Some(line) => format!("pid {pid}: {line}"),
        None => format!("pid {pid} (already exited)"),
    }
}


fn marker_path() -> Option<PathBuf> {
    let dir = dirs::config_dir()?.join("workbench");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("lifecycle"))
}

/// Called once at boot: report on the previous instance, then claim the
/// marker for this one.
pub fn note_boot() {
    let Some(path) = marker_path() else {
        return;
    };
    match std::fs::read_to_string(&path) {
        Ok(prev) if prev.starts_with("running") => crate::logger::warn(format!(
            "previous instance ended abruptly — killed or lost power, since a \
             clean exit rewrites this marker ({})",
            prev.trim()
        )),
        // A clean marker, or no marker at all (first run): nothing to report.
        _ => {}
    }
    // The hazard that has actually been killing instances: running straight
    // out of target/, where every `cargo build` — ours, a peer agent's, the
    // launch alias itself — relinks the file this process is executing. On
    // macOS that invalidates the running binary's code signature, and the
    // kernel answers the next cold page fault with SIGKILL. Under full swap
    // the fault can come hours after the build, which is why the deaths
    // looked random.
    if std::env::current_exe()
        .ok()
        .is_some_and(|exe| exe.components().any(|c| c.as_os_str() == "target"))
    {
        crate::logger::warn(
            "running from a cargo target/ directory: any rebuild will get this              process SIGKILLed by the kernel (in-place relink invalidates the              code signature). Install a copy and run that instead, e.g.              `cargo install --path .` and launch `workbench`.",
        );
    }

    let claim = format!(
        "running pid {} since {}\n",
        std::process::id(),
        chrono::Utc::now().to_rfc3339()
    );
    if let Err(err) = std::fs::write(&path, claim) {
        crate::logger::warn(format!("could not write the lifecycle marker: {err}"));
    }
}

/// Called on the deliberate way out. After this, the next boot has nothing
/// to accuse anyone of.
pub fn note_clean_exit() {
    let Some(path) = marker_path() else {
        return;
    };
    let done = format!(
        "clean exit pid {} at {}\n",
        std::process::id(),
        chrono::Utc::now().to_rfc3339()
    );
    if let Err(err) = std::fs::write(&path, done) {
        crate::logger::warn(format!("could not write the lifecycle marker: {err}"));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// The whole point: a SIGTERM stops being a death and becomes a notice
    /// naming its sender.
    #[test]
    fn a_sigterm_is_survived_and_attributed() {
        watch_termination();
        assert!(termination_notice().is_none(), "nothing pending at first");

        // SAFETY: signalling ourselves, with the handler installed above.
        unsafe { libc::kill(std::process::id() as i32, libc::SIGTERM) };
        std::thread::sleep(std::time::Duration::from_millis(50));

        let notice = termination_notice().expect("the signal was recorded");
        assert!(notice.contains("SIGTERM"), "{notice}");
        assert!(
            notice.contains(&std::process::id().to_string()),
            "the sender (us) is named: {notice}"
        );
        // And it drained: one request, one notice.
        assert!(termination_notice().is_none());
    }
}
