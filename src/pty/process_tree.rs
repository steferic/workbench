//! Ownership survives job control, setsid(), and the original agent exiting.
//! Each launch exports a fresh marker inherited by its children. We also walk
//! parent links for children that clear their environment while still attached.
//! Every signal checks the captured kernel start time; a PID alone is not an
//! identity. Enumeration is only done during cleanup, never on the render loop.
//! A daemon that clears the marker and reparents before cleanup cannot be
//! safely attributed; unrelated processes are always left alone.

use super::proc_identity::{self, ProcStart};
use anyhow::{bail, Result};
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const ENV_OWNER: &str = "WORKBENCH_PROCESS_OWNER";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    pid: u32,
    start: ProcStart,
}

struct Process {
    identity: Identity,
    parent: u32,
    zombie: bool,
}

pub struct ProcessTree {
    root: Option<Identity>,
    marker: Option<Vec<u8>>,
    known: HashMap<u32, Identity>,
    finished: bool,
}

impl ProcessTree {
    pub fn new(pid: Option<u32>, token: &str) -> Self {
        let root =
            pid.and_then(|pid| proc_identity::start_time(pid).map(|start| Identity { pid, start }));
        Self {
            root,
            marker: Some(format!("{ENV_OWNER}={token}").into_bytes()),
            known: HashMap::new(),
            finished: false,
        }
    }

    /// A user-selected server owns only its descendants, not every sibling
    /// carrying the coding agent's environment marker. Keep its scan identity.
    pub fn for_server(pid: u32, start: ProcStart) -> Self {
        Self {
            root: Some(Identity { pid, start }),
            marker: None,
            known: HashMap::new(),
            finished: false,
        }
    }

    pub fn stop_server(&mut self, label: &str) -> Result<()> {
        self.terminate_with_signal(Duration::from_millis(1500), label, libc::SIGTERM)
    }

    fn refresh(&mut self) -> Result<()> {
        let processes = snapshot()?;
        self.known.retain(|pid, identity| {
            processes
                .get(pid)
                .is_some_and(|p| !p.zombie && p.identity == *identity)
        });
        if let Some(root) = self.root {
            if processes
                .get(&root.pid)
                .is_some_and(|p| !p.zombie && p.identity == root)
            {
                self.known.insert(root.pid, root);
            }
        }
        let mut env_buffer = environment_buffer();
        for process in processes.values().filter(|p| !p.zombie) {
            let id = process.identity;
            // A descendant cannot predate its owner. Besides saving syscalls,
            // this avoids reading the environment of older, unrelated apps.
            if self.root.is_some_and(|root| {
                (id.start.sec, id.start.usec) < (root.start.sec, root.start.usec)
            }) || self.known.contains_key(&id.pid)
            {
                continue;
            }
            if self
                .marker
                .as_ref()
                .is_some_and(|marker| has_marker(id.pid, marker, &mut env_buffer))
            {
                self.known.insert(id.pid, id);
            }
        }
        loop {
            let before = self.known.len();
            for process in processes.values().filter(|p| !p.zombie) {
                if self.known.get(&process.parent).is_some_and(|parent| {
                    (process.identity.start.sec, process.identity.start.usec)
                        >= (parent.start.sec, parent.start.usec)
                        && proc_identity::owner(parent.pid, parent.start)
                            == proc_identity::PidOwner::Ours
                }) {
                    self.known.insert(process.identity.pid, process.identity);
                }
            }
            if self.known.len() == before {
                break;
            }
        }
        Ok(())
    }

    fn signal(&self, signal: i32, label: &str) -> Result<()> {
        let mut failure = None;
        for identity in self.known.values() {
            if proc_identity::owner(identity.pid, identity.start) != proc_identity::PidOwner::Ours {
                continue;
            }
            crate::logger::info(format!(
                "kill: signal {signal} pid {} ({label})",
                identity.pid
            ));
            // Positive PIDs only, verified immediately before each signal.
            if unsafe { libc::kill(identity.pid as i32, signal) } == -1 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() != Some(libc::ESRCH) {
                    failure = Some(err);
                }
            }
        }
        match failure {
            Some(err) => Err(err.into()),
            None => Ok(()),
        }
    }

    pub fn terminate(&mut self, grace: Duration, label: &str) -> Result<()> {
        self.terminate_with_signal(grace, label, libc::SIGINT)
    }

    fn terminate_with_signal(&mut self, grace: Duration, label: &str, signal: i32) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.refresh()?;
        if !grace.is_zero() && !self.known.is_empty() {
            self.signal(signal, label)?;
            let deadline = Instant::now() + grace;
            while Instant::now() < deadline {
                if self.known.values().all(|id| !is_live(*id)) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        // Keep the descendants captured before SIGINT even if their parent
        // was reaped during the grace period. Rescan for children forked by
        // shutdown handlers, including those now reparented to init.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            self.refresh()?;
            if self.known.is_empty() {
                self.finished = true;
                return Ok(());
            }
            self.signal(libc::SIGKILL, label)?;
            if Instant::now() >= deadline {
                bail!("processes for {label} did not exit after SIGKILL");
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

fn is_live(identity: Identity) -> bool {
    process(identity.pid).is_some_and(|p| !p.zombie && p.identity == identity)
}

#[cfg(target_os = "macos")]
fn process(pid: u32) -> Option<Process> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    let got = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut _,
            size,
        )
    };
    if got != size || info.pbi_uid != unsafe { libc::geteuid() } {
        return None;
    }
    Some(Process {
        identity: Identity {
            pid,
            start: ProcStart {
                sec: info.pbi_start_tvsec,
                usec: info.pbi_start_tvusec,
            },
        },
        parent: info.pbi_ppid,
        zombie: info.pbi_status == 5, // SZOMB, from sys/proc.h
    })
}

#[cfg(target_os = "macos")]
fn snapshot() -> Result<HashMap<u32, Process>> {
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    if count <= 0 {
        bail!("could not enumerate processes for cleanup");
    }
    let mut pids = vec![0i32; count as usize + 128];
    let got = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * 4) as i32) };
    if got <= 0 {
        bail!("could not enumerate processes for cleanup");
    }
    Ok(pids
        .into_iter()
        .take(got as usize)
        .filter(|pid| *pid > 1 && *pid as u32 != std::process::id())
        .filter_map(|pid| process(pid as u32).map(|p| (pid as u32, p)))
        .collect())
}

#[cfg(target_os = "linux")]
fn process(pid: u32) -> Option<Process> {
    use std::os::unix::fs::MetadataExt;
    let path = format!("/proc/{pid}");
    if std::fs::metadata(&path).ok()?.uid() != unsafe { libc::geteuid() } {
        return None;
    }
    let stat = std::fs::read_to_string(format!("{path}/stat")).ok()?;
    let fields: Vec<_> = stat[stat.rfind(')')? + 1..].split_whitespace().collect();
    Some(Process {
        identity: Identity {
            pid,
            start: ProcStart {
                sec: fields.get(19)?.parse().ok()?,
                usec: 0,
            },
        },
        parent: fields.get(1)?.parse().ok()?,
        zombie: *fields.first()? == "Z",
    })
}

#[cfg(target_os = "linux")]
fn snapshot() -> Result<HashMap<u32, Process>> {
    Ok(std::fs::read_dir("/proc")?
        .filter_map(|entry| {
            let pid: u32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
            if pid <= 1 || pid == std::process::id() {
                return None;
            }
            process(pid).map(|p| (pid, p))
        })
        .collect())
}

#[cfg(target_os = "macos")]
fn environment_buffer() -> Vec<u8> {
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    let mut max = 0i32;
    let mut size = std::mem::size_of_val(&max);
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            &mut max as *mut _ as *mut _,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    vec![
        0;
        if result == 0 && max > 0 {
            max as usize
        } else {
            1024 * 1024
        }
    ]
}

#[cfg(target_os = "macos")]
fn has_marker(pid: u32, marker: &[u8], buffer: &mut Vec<u8>) -> bool {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as i32];
    let mut size = buffer.len();
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return false;
    }
    environment_from_procargs(&buffer[..size])
        .is_some_and(|env| env.split(|b| *b == 0).any(|entry| entry == marker))
}

#[cfg(target_os = "macos")]
fn environment_from_procargs(bytes: &[u8]) -> Option<&[u8]> {
    let argc = i32::from_ne_bytes(bytes.get(..4)?.try_into().ok()?);
    if argc < 0 {
        return None;
    }
    let mut rest = bytes.get(4..)?;
    // Executable path, NUL padding, then exactly argc argument strings.
    rest = rest.get(rest.iter().position(|b| *b == 0)? + 1..)?;
    while rest.first() == Some(&0) {
        rest = &rest[1..];
    }
    for _ in 0..argc {
        rest = rest.get(rest.iter().position(|b| *b == 0)? + 1..)?;
    }
    Some(rest)
}

#[cfg(target_os = "linux")]
fn environment_buffer() -> Vec<u8> {
    Vec::new()
}

#[cfg(target_os = "linux")]
fn has_marker(pid: u32, marker: &[u8], _buffer: &mut Vec<u8>) -> bool {
    std::fs::read(format!("/proc/{pid}/environ"))
        .is_ok_and(|bytes| bytes.split(|b| *b == 0).any(|entry| entry == marker))
}

#[cfg(test)]
pub(crate) fn running(pid: u32) -> bool {
    process(pid).is_some_and(|p| !p.zombie)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recycled_root_identity_does_not_authorize_signals() {
        let mut child = std::process::Command::new("sleep")
            .arg("20")
            .spawn()
            .unwrap();
        let mut tree = ProcessTree::new(Some(child.id()), &uuid::Uuid::new_v4().to_string());
        tree.root.as_mut().unwrap().start.sec = 1;
        let result = tree.terminate(Duration::ZERO, "recycled fixture");
        let survived = child.try_wait().unwrap().is_none();
        let _ = child.kill();
        child.wait().unwrap();
        result.unwrap();
        assert!(survived, "a recycled PID was signalled");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn procargs_marker_must_be_in_the_environment_not_the_arguments() {
        let marker = b"WORKBENCH_PROCESS_OWNER=token";
        let mut args = 2i32.to_ne_bytes().to_vec();
        args.extend_from_slice(
            b"/bin/agent\0\0agent\0WORKBENCH_PROCESS_OWNER=token\0OTHER=value\0",
        );
        let env = environment_from_procargs(&args).unwrap();
        assert!(!env.split(|b| *b == 0).any(|field| field == marker));
        args.extend_from_slice(marker);
        args.push(0);
        let env = environment_from_procargs(&args).unwrap();
        assert!(env.split(|b| *b == 0).any(|field| field == marker));
    }
}
