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
