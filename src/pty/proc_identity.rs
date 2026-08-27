//! Whether a pid still names the process we spawned.
//!
//! A pid is a loan, not a name: once the child exits, the kernel hands the
//! number to whatever forks next. On this machine that is not theoretical —
//! under agent load it burns through the whole pid space in minutes — so a
//! stored pid can come to mean an innocent process, and `kill(-pid, SIGKILL)`
//! aimed at a dead agent would take down whoever inherited the number.
//!
//! The start time is the disambiguator: two owners of one pid can never share
//! one. Captured when the child is spawned, compared before anything is
//! signalled.

/// A process's kernel start time, opaque beyond equality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcStart {
    pub sec: u64,
    pub usec: u64,
}

/// What a remembered pid means right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PidOwner {
    /// Still the process we spawned.
    Ours,
    /// The number now belongs to someone else. Signalling it would be
    /// friendly fire.
    Recycled,
    /// Nothing holds the pid; the child is gone and reaped.
    Gone,
}

/// The start time of `pid`, or `None` if no such process (or no way to ask).
#[cfg(target_os = "macos")]
pub fn start_time(pid: u32) -> Option<ProcStart> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let got = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        )
    };
    (got == size).then(|| ProcStart {
        sec: info.pbi_start_tvsec,
        usec: info.pbi_start_tvusec,
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn start_time(pid: u32) -> Option<ProcStart> {
    // Field 22 of /proc/<pid>/stat is the start time in clock ticks. The
    // command name in field 2 may contain spaces and parentheses, so parse
    // from after the *last* ')'.
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = &stat[stat.rfind(')')? + 1..];
    let ticks: u64 = after.split_whitespace().nth(19)?.parse().ok()?;
    Some(ProcStart {
        sec: ticks,
        usec: 0,
    })
}

#[cfg(not(unix))]
pub fn start_time(_pid: u32) -> Option<ProcStart> {
    None
}

/// Compare what we remembered against what the pid is now.
///
/// An unreadable current state is `Gone`, not `Ours`: the one thing a kill
/// must never do is proceed on a pid it cannot vouch for.
pub fn owner(pid: u32, spawned: ProcStart) -> PidOwner {
    match start_time(pid) {
        Some(now) if now == spawned => PidOwner::Ours,
        Some(_) => PidOwner::Recycled,
        None => PidOwner::Gone,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn our_own_start_time_is_readable_and_stable() {
        let pid = std::process::id();
        let first = start_time(pid).expect("a live process has a start time");
        let again = start_time(pid).expect("still alive");
        assert_eq!(first, again);
        assert_eq!(owner(pid, first), PidOwner::Ours);
    }

    #[test]
    fn a_wrong_start_time_reads_as_recycled() {
        let pid = std::process::id();
        let real = start_time(pid).unwrap();
        let forged = ProcStart {
            sec: real.sec.wrapping_add(1),
            usec: real.usec,
        };
        assert_eq!(owner(pid, forged), PidOwner::Recycled);
    }

    #[test]
    fn a_dead_pid_reads_as_gone() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        // The pid is reaped; only a (vanishingly unlikely, immediate) reuse
        // could make this read as anything but Gone — and that reuse would
        // read as Recycled, which the guard also refuses to kill.
        let spawned = ProcStart { sec: 1, usec: 1 };
        assert_ne!(owner(pid, spawned), PidOwner::Ours);
    }
}
