//! How much memory each agent is holding, counted the way macOS counts it.
//!
//! The number is `ri_phys_footprint` — what Activity Monitor calls "Memory"
//! and what the "out of application memory" dialog adds up — rather than the
//! resident size. The two part company exactly when it matters: a Claude
//! process that had grown to 32 GB showed a resident size of 400 MB, because
//! the rest had been compressed or swapped out. Footprint counts both.
//!
//! An agent is its whole process tree. A build, a dev server or a Python job
//! it started is memory it is responsible for, and the dialog bills all of it
//! to the terminal anyway. The agent's own process is kept apart so a reader
//! can tell a leaking agent from a heavy job it is running.
//!
//! Enumerating processes is a syscall per pid, so this runs off the event
//! loop (see `app::handler::scan_memory`).

use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgentMemory {
    /// The agent process itself.
    pub own: u64,
    /// The agent and every process below it.
    pub total: u64,
}

/// `32.1 GB`, `740 MB`: short enough for a pane row, exact enough to act on.
pub fn human(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let mb = bytes as f64 / MB;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{mb:.0} MB")
    }
}

/// Measure each root's process tree. A root that has gone is left out.
pub fn sample(roots: &[(Uuid, u32)]) -> HashMap<Uuid, AgentMemory> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let parents = super::process_tree::parents().unwrap_or_default();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let parents: HashMap<u32, u32> = HashMap::new();
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&pid, &parent) in &parents {
        children.entry(parent).or_default().push(pid);
    }

    let mut out = HashMap::new();
    for &(session, root) in roots {
        let Some(own) = footprint(root) else {
            continue;
        };
        let mut total = own;
        let mut stack: Vec<u32> = children.get(&root).cloned().unwrap_or_default();
        while let Some(pid) = stack.pop() {
            total += footprint(pid).unwrap_or(0);
            if let Some(below) = children.get(&pid) {
                stack.extend(below);
            }
        }
        out.insert(session, AgentMemory { own, total });
    }
    out
}

#[cfg(target_os = "macos")]
fn footprint(pid: u32) -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::uninit();
    // SAFETY: proc_pid_rusage fills the struct for the flavor it is given,
    // and it is only read after the call reports success.
    let ok = unsafe {
        libc::proc_pid_rusage(
            pid as libc::c_int,
            libc::RUSAGE_INFO_V2,
            info.as_mut_ptr() as *mut libc::rusage_info_t,
        )
    } == 0;
    ok.then(|| unsafe { info.assume_init() }.ri_phys_footprint)
}

/// Linux has no footprint; resident plus swapped is the nearest honest sum.
#[cfg(target_os = "linux")]
fn footprint(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let kb = |key: &str| {
        status.lines().find_map(|line| {
            let rest = line.strip_prefix(key)?;
            rest.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()
        })
    };
    Some((kb("VmRSS:")? + kb("VmSwap:").unwrap_or(0)) * 1024)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn footprint(_pid: u32) -> Option<u64> {
    None
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;

    #[test]
    fn a_tree_counts_its_children_and_keeps_its_own_share_apart() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let session = Uuid::new_v4();
        let sampled = sample(&[(session, std::process::id())]);
        let _ = child.kill();
        let _ = child.wait();

        let memory = sampled[&session];
        assert!(memory.own > 0, "{memory:?}");
        assert!(
            memory.total > memory.own,
            "the sleep below this process should be counted: {memory:?}"
        );
    }

    #[test]
    fn a_root_that_has_gone_is_left_out() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(sample(&[(Uuid::new_v4(), pid)]).is_empty());
    }
}
