//! The API key an agent is billed to, read at the moment it starts.
//!
//! An agent entry may name a command that prints a key (see
//! `AgentConfig::api_key_cmd`). Workbench runs it for each spawn and hands
//! the result to that one child as `ANTHROPIC_API_KEY`. The key is never
//! stored, logged or put in an error: what is kept in config and in saved
//! state is the command.
//!
//! Every failure here stops the spawn. An agent that was asked to run on an
//! API key and cannot get it must not start on the CLI's own login instead,
//! because the whole point of asking was which account pays.

use anyhow::{anyhow, Result};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The variable Claude Code reads its key from.
pub(super) const ENV: &str = "ANTHROPIC_API_KEY";

/// Long enough for a keychain to answer, short enough that a command waiting
/// on something nobody will give it does not hold the interface for good.
const TIMEOUT: Duration = Duration::from_secs(20);

pub(super) fn read(command: &str) -> Result<String> {
    read_within(command, TIMEOUT)
}

fn read_within(command: &str, timeout: Duration) -> Result<String> {
    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    let mut child = Command::new(shell)
        .arg(flag)
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| refused(format!("its API key command could not be run ({err})")))?;

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(refused(format!(
                    "its API key command gave no answer in {}s",
                    timeout.as_secs()
                )));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(refused(format!("its API key command was lost ({err})")));
            }
        }
    };

    // The command has exited, so both pipes are closed at the far end and
    // these reads cannot wait.
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }

    if !status.success() {
        // Its stderr says why (a missing keychain item, say). Its stdout is
        // never quoted: on a half-working command that is where a key is.
        let why = stderr
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| line.chars().take(160).collect::<String>());
        return Err(refused(match why {
            Some(why) => format!("its API key command failed: {why}"),
            None => "its API key command failed".to_string(),
        }));
    }

    let key = stdout.trim();
    if key.is_empty() {
        return Err(refused("its API key command printed nothing".to_string()));
    }
    if key.chars().any(char::is_whitespace) {
        return Err(refused(
            "its API key command printed more than a key".to_string(),
        ));
    }
    Ok(key.to_string())
}

/// One self-contained message: the session-start notice shows an error's root
/// cause, so the reason and the consequence have to be in the same sentence.
fn refused(why: String) -> anyhow::Error {
    anyhow!("{why}, so the agent was not started (it is never moved to the subscription login)")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_what_the_command_prints_without_its_newline() {
        assert_eq!(read("printf 'sk-test-123\\n'").unwrap(), "sk-test-123");
    }

    #[test]
    fn a_command_that_prints_nothing_is_not_a_key() {
        let err = read("true").unwrap_err().to_string();
        assert!(err.contains("printed nothing"), "{err}");
        assert!(err.contains("not started"), "{err}");
    }

    /// The failure a missing keychain item produces: the reason is shown, and
    /// whatever the command wrote to stdout is not.
    #[test]
    fn a_failing_command_reports_its_stderr_and_never_its_stdout() {
        let err = read("echo sk-partial-secret; echo 'item not found' >&2; exit 44")
            .unwrap_err()
            .to_string();
        assert!(err.contains("item not found"), "{err}");
        assert!(!err.contains("sk-partial-secret"), "{err}");
    }

    #[test]
    fn output_that_is_not_one_token_is_refused() {
        let err = read("printf 'Enter password:\\nsk-test'")
            .unwrap_err()
            .to_string();
        assert!(err.contains("more than a key"), "{err}");
        assert!(!err.contains("sk-test"), "{err}");
    }

    #[test]
    fn a_command_that_never_answers_is_given_up_on() {
        let started = Instant::now();
        let err = read_within("sleep 30", Duration::from_millis(150))
            .unwrap_err()
            .to_string();
        assert!(err.contains("gave no answer"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    mod spawn {
        use crate::models::AgentType;
        use crate::pty::{PtyManager, Resume, SessionSpawnConfig};
        use std::os::unix::fs::PermissionsExt;
        use std::path::Path;
        use std::time::{Duration, Instant};
        use tokio::sync::mpsc;
        use uuid::Uuid;

        /// A stand-in agent that records the key it was started with.
        fn agent(dir: &Path, api_key_cmd: Option<&str>) -> AgentType {
            let script = dir.join("agent");
            std::fs::write(
                &script,
                "#!/bin/sh\nprintf '%s' \"${ANTHROPIC_API_KEY-unset}\" > started\nexec sleep 20\n",
            )
            .unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
            AgentType::Custom {
                command: script.to_string_lossy().into(),
                display_name: "Fixture API".into(),
                badge: "A".into(),
                api_key_cmd: api_key_cmd.map(str::to_string),
            }
        }

        fn spawn(dir: &Path, agent_type: AgentType) -> anyhow::Result<crate::pty::PtyHandle> {
            let (pty_tx, _rx) = mpsc::channel(32);
            PtyManager::new().spawn_session(SessionSpawnConfig {
                session_id: Uuid::new_v4(),
                workspace_id: Uuid::new_v4(),
                agent_type,
                working_dir: dir,
                rows: 24,
                cols: 80,
                pty_tx,
                resume: Resume::No,
                dangerously_skip_permissions: false,
                use_alternate_screen: false,
                scrollback_rows: 200,
                // Something else trying to name the key must not win.
                extra_env: vec![(super::ENV.to_string(), "from-extra-env".to_string())],
            })
        }

        fn started_with(dir: &Path) -> String {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match std::fs::read_to_string(dir.join("started")) {
                    Ok(seen) if !seen.is_empty() => return seen,
                    _ => assert!(Instant::now() < deadline, "the agent never started"),
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        #[test]
        fn the_agent_starts_with_the_key_its_command_printed() {
            let dir = tempfile::tempdir().unwrap();
            let mut handle = spawn(
                dir.path(),
                agent(dir.path(), Some("printf 'sk-from-the-keychain\\n'")),
            )
            .unwrap();
            assert_eq!(started_with(dir.path()), "sk-from-the-keychain");
            let _ = handle.kill();
        }

        /// The rule that matters: no key, no agent. The process is never
        /// created, so there is nothing that could be running on the login.
        #[test]
        fn an_agent_whose_key_cannot_be_read_is_not_started_at_all() {
            let dir = tempfile::tempdir().unwrap();
            let err = spawn(
                dir.path(),
                agent(dir.path(), Some("echo 'no such item' >&2; exit 44")),
            )
            .err()
            .expect("the spawn must be refused");
            // What the session-start notice shows is the root cause.
            let shown = err.root_cause().to_string();
            assert!(shown.contains("no such item"), "{shown}");
            assert!(shown.contains("not started"), "{shown}");
            std::thread::sleep(Duration::from_millis(200));
            assert!(!dir.path().join("started").exists());
        }
    }
}
