//! Give the TUI room for PTYs, sockets and files without changing system policy.

pub fn prepare() {
    #[cfg(unix)]
    match raise_nofile_limit() {
        Ok((before, after)) => {
            crate::logger::info(format!("file descriptor soft limit: {before} -> {after}"))
        }
        Err(err) => crate::logger::warn(format!(
            "could not raise file descriptor soft limit: {err:#}"
        )),
    }
}

#[cfg(unix)]
fn target_limit(current: libc::rlim_t, hard: libc::rlim_t) -> libc::rlim_t {
    // Four persistent descriptors per PTY plus temporary spawn descriptors,
    // servers, and background file reads. Never reduce a larger allowance or
    // change the hard limit. A finite target also works with macOS's unlimited
    // hard limit (setting its soft limit to RLIM_INFINITY is not supported).
    current.max(4096.min(hard))
}

#[cfg(unix)]
fn raise_nofile_limit() -> anyhow::Result<(libc::rlim_t, libc::rlim_t)> {
    use anyhow::Context;
    let mut limits = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes to a valid, initialized rlimit.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) } != 0 {
        return Err(std::io::Error::last_os_error()).context("getrlimit(RLIMIT_NOFILE)");
    }
    let before = limits.rlim_cur;
    limits.rlim_cur = target_limit(before, limits.rlim_max);
    if limits.rlim_cur != before {
        // SAFETY: a valid rlimit; only the soft limit changes, within the hard limit.
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limits) } != 0 {
            return Err(std::io::Error::last_os_error()).context("setrlimit(RLIMIT_NOFILE)");
        }
    }
    Ok((before, limits.rlim_cur))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn retains_larger_limits_and_respects_hard_limit() {
        assert_eq!(target_limit(8192, 16384), 8192);
        assert_eq!(target_limit(256, 1024), 1024);
        assert_eq!(target_limit(256, libc::RLIM_INFINITY), 4096);
    }

    #[test]
    fn raises_real_limit_in_isolated_processes() {
        // Never change the test runner's process-wide limits. Exercise both
        // the ordinary GUI allowance and a hard cap below our target.
        for hard in [4096, 1024] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "resource_limits::tests::limit_child",
                    "--ignored",
                ])
                .env("WORKBENCH_TEST_NOFILE_HARD", hard.to_string())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    #[ignore = "subprocess fixture; run by raises_real_limit_in_isolated_processes"]
    fn limit_child() {
        let hard: libc::rlim_t = std::env::var("WORKBENCH_TEST_NOFILE_HARD")
            .unwrap()
            .parse()
            .unwrap();
        let limits = libc::rlimit {
            rlim_cur: 256,
            rlim_max: hard,
        };
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limits) }, 0);
        assert_eq!(raise_nofile_limit().unwrap(), (256, hard));
        let mut actual = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut actual) },
            0
        );
        assert_eq!((actual.rlim_cur, actual.rlim_max), (hard, hard));
        let files: Vec<_> = (0..320)
            .map(|_| std::fs::File::open("/dev/null").unwrap())
            .collect();
        assert_eq!(files.len(), 320);
        assert_eq!(raise_nofile_limit().unwrap(), (hard, hard));
    }
}
