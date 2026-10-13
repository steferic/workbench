//! Who says what: word counts for you and every agent, by project and model.
//!
//! Nothing new is recorded for this. Claude Code and Codex already keep every
//! conversation as JSONL — `~/.claude/projects` and `~/.codex/sessions` — with
//! the model on each reply and the working directory that names the project.
//! A scan reads those files and adds up the words into one SQLite table keyed
//! by project, provider, model and speaker.
//!
//! Cost is the reason for the shape. The transcripts run to gigabytes, almost
//! all of it tool output, so a scan remembers how far into each file it has
//! read and only ever reads what was appended since; lines that are plainly
//! tool output are skipped before they are parsed. The counts live on disk,
//! not in Workbench, and a cloud is one `GROUP BY` away. Nothing runs until a
//! cloud is asked for (`workbench words`, or `/words` on the phone server).

pub mod page;
pub mod prune;
mod scan;
pub mod tokens;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub use scan::scan;

/// "you" for what you typed, "agent" for what an agent wrote back.
pub const YOU: &str = "you";
pub const AGENT: &str = "agent";

pub fn db_path() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("no config directory")?
        .join("workbench");
    Ok(dir.join("words.sqlite3"))
}

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    // A scan holds the write lock for one file at a time; a second scanner
    // (the CLI while Workbench scans) waits its turn rather than failing.
    connection.busy_timeout(Duration::from_secs(120))?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS files (
             path TEXT PRIMARY KEY,
             offset INTEGER NOT NULL,
             -- NULL until a line names the directory; '' when it is not one
             -- of your projects, and the file is skipped from then on.
             project TEXT,
             provider TEXT NOT NULL,
             model TEXT NOT NULL DEFAULT '',
             typed INTEGER NOT NULL DEFAULT 1
         );
         CREATE TABLE IF NOT EXISTS counts (
             project TEXT NOT NULL,
             provider TEXT NOT NULL,
             model TEXT NOT NULL,
             speaker TEXT NOT NULL,
             word TEXT NOT NULL,
             n INTEGER NOT NULL,
             PRIMARY KEY (project, provider, model, speaker, word)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS totals (
             project TEXT NOT NULL,
             provider TEXT NOT NULL,
             model TEXT NOT NULL,
             speaker TEXT NOT NULL,
             n INTEGER NOT NULL,
             PRIMARY KEY (project, provider, model, speaker)
         ) WITHOUT ROWID;
         -- Words you took out of the cloud to see what is under them.
         CREATE TABLE IF NOT EXISTS hidden (
             word TEXT PRIMARY KEY,
             at_ms INTEGER NOT NULL
         ) WITHOUT ROWID;",
    )?;
    Ok(connection)
}

/// Which slice of the counts to draw. `None` means all of them.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub project: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub speaker: Option<String>,
    /// Leave out words shorter than this many characters.
    pub min_len: usize,
}

#[derive(Debug, Serialize)]
pub struct Cloud {
    /// The most used words in the slice, stop words left out.
    pub words: Vec<(String, u64)>,
    /// Every word in the slice, stop words included.
    pub total: u64,
    /// What there is to filter by, with how many words each holds.
    pub projects: Vec<(String, u64)>,
    pub providers: Vec<(String, u64)>,
    pub models: Vec<Model>,
    /// The words you hid, most recent first.
    pub hidden: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Model {
    pub provider: String,
    pub model: String,
    pub words: u64,
}

pub fn query(connection: &Connection, filter: &Filter, limit: usize) -> Result<Cloud> {
    const WHERE: &str = "(?1 IS NULL OR project = ?1) AND (?2 IS NULL OR provider = ?2)
         AND (?3 IS NULL OR model = ?3) AND (?4 IS NULL OR speaker = ?4)";
    let args = params![
        filter.project,
        filter.provider,
        filter.model,
        filter.speaker
    ];

    // Stop words are filtered here rather than at count time; ask for enough
    // extra rows that every one of them could be skipped.
    let mut statement = connection.prepare(&format!(
        "SELECT word, SUM(n) AS total FROM counts WHERE {WHERE}
           AND length(word) >= ?6 AND word NOT IN (SELECT word FROM hidden)
         GROUP BY word ORDER BY total DESC LIMIT ?5"
    ))?;
    let fetch = (limit + 400) as i64;
    let words = statement
        .query_map(
            params![
                filter.project,
                filter.provider,
                filter.model,
                filter.speaker,
                fetch,
                filter.min_len as i64
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64)),
        )?
        .filter_map(Result::ok)
        .filter(|(word, _)| !tokens::is_stop_word(word))
        .take(limit)
        .collect();

    let total: i64 = connection.query_row(
        &format!("SELECT COALESCE(SUM(n), 0) FROM totals WHERE {WHERE}"),
        args,
        |row| row.get(0),
    )?;

    // The facets count within the speaker the reader picked, so the menu
    // says how much each project or model holds of what is on screen.
    let speaker = filter.speaker.as_deref();
    let grouped = |column: &str| -> Result<Vec<(String, u64)>> {
        let mut statement = connection.prepare(&format!(
            "SELECT {column}, SUM(n) AS total FROM totals WHERE (?1 IS NULL OR speaker = ?1)
             GROUP BY {column} ORDER BY total DESC"
        ))?;
        let rows = statement
            .query_map([speaker], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    };
    let projects = grouped("project")?;
    let providers = grouped("provider")?;
    let mut statement = connection.prepare(
        "SELECT provider, model, SUM(n) AS total FROM totals
         WHERE (?1 IS NULL OR speaker = ?1) AND model != ''
         GROUP BY provider, model ORDER BY total DESC",
    )?;
    let models = statement
        .query_map([speaker], |row| {
            Ok(Model {
                provider: row.get(0)?,
                model: row.get(1)?,
                words: row.get::<_, i64>(2)? as u64,
            })
        })?
        .filter_map(Result::ok)
        .collect();

    Ok(Cloud {
        words,
        total: total as u64,
        projects,
        providers,
        models,
        hidden: hidden(connection)?,
    })
}

pub fn hidden(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection.prepare("SELECT word FROM hidden ORDER BY at_ms DESC")?;
    let words = statement
        .query_map([], |row| row.get(0))?
        .filter_map(Result::ok)
        .collect();
    Ok(words)
}

/// Take words out of every cloud, or (`hide = false`) put them back.
pub fn set_hidden(connection: &Connection, words: &[String], hide: bool) -> Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    for word in words {
        let word = word.trim().to_lowercase();
        if word.is_empty() {
            continue;
        }
        if hide {
            connection.execute(
                "INSERT INTO hidden (word, at_ms) VALUES (?1, ?2)
                 ON CONFLICT DO UPDATE SET at_ms = excluded.at_ms",
                params![word, now],
            )?;
        } else {
            connection.execute("DELETE FROM hidden WHERE word = ?1", [word])?;
        }
    }
    Ok(())
}

/// Progress of the scan Workbench runs for the phone page.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Progress {
    pub scanning: bool,
    pub done: u64,
    pub of: u64,
}

static SCANNING: AtomicBool = AtomicBool::new(false);
static DONE: AtomicU64 = AtomicU64::new(0);
static OF: AtomicU64 = AtomicU64::new(0);
static LAST: Mutex<Option<Instant>> = Mutex::new(None);

pub fn progress() -> Progress {
    Progress {
        scanning: SCANNING.load(Ordering::Relaxed),
        done: DONE.load(Ordering::Relaxed),
        of: OF.load(Ordering::Relaxed),
    }
}

/// Start a scan on its own thread unless one ran in the last minute or is
/// running now. The page shows whatever is counted so far and asks again.
pub fn refresh_in_background() {
    refresh_unless_within(Duration::from_secs(60));
}

/// Called on every tick of the app. Claude Code deletes a transcript 30
/// days after its last write, and `workbench prune-sessions` sooner, so the
/// counts cannot wait for someone to open the cloud: a file deleted before
/// any scan read it is lost to them. A scan every 6 hours is far inside both.
pub fn keep_current() {
    refresh_unless_within(Duration::from_secs(6 * 60 * 60));
}

fn refresh_unless_within(again_after: Duration) {
    {
        let Ok(mut last) = LAST.lock() else { return };
        if last.is_some_and(|at| at.elapsed() < again_after) {
            return;
        }
        if SCANNING.swap(true, Ordering::SeqCst) {
            return;
        }
        *last = Some(Instant::now());
    }
    std::thread::spawn(|| {
        background_priority();
        let result = db_path().and_then(|path| {
            scan(&path, |done, of| {
                DONE.store(done, Ordering::Relaxed);
                OF.store(of, Ordering::Relaxed);
            })
        });
        if let Err(err) = result {
            crate::logger::warn(format!("word count scan failed: {err:#}"));
        }
        if let Ok(mut last) = LAST.lock() {
            *last = Some(Instant::now());
        }
        SCANNING.store(false, Ordering::SeqCst);
    });
}

/// macOS throttles a background thread's CPU and disk below everything the
/// user is doing, which is exactly the priority a word count deserves.
#[cfg(target_os = "macos")]
fn background_priority() {
    const PRIO_DARWIN_THREAD: libc::c_int = 3;
    const PRIO_DARWIN_BG: libc::c_int = 0x1000;
    // SAFETY: plain syscall on the calling thread; failure leaves the
    // priority as it was.
    unsafe {
        libc::setpriority(PRIO_DARWIN_THREAD, 0, PRIO_DARWIN_BG);
    }
}

#[cfg(not(target_os = "macos"))]
fn background_priority() {}
