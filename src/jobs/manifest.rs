//! `.workbench/jobs.toml`: what to show, what to send, where to record.
//!
//! Deliberately thin. A project's own job configuration — queries, limits,
//! accounts — stays in the project's own files, which the prompt names; this
//! file is the index workbench needs to list and launch them.

use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_HISTORY_DIR: &str = ".workbench/jobs/history";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub schema_version: u32,
    /// Where run records go, relative to the repository root.
    pub history_dir: PathBuf,
    pub jobs: Vec<JobDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobDef {
    /// Stable and filename-safe: it names the history and lessons files.
    pub id: String,
    pub title: String,
    pub description: String,
    /// The agent command to run it with; the configured default when absent.
    pub agent: Option<String>,
    /// A cadence hint. Marks the row due; never starts anything.
    pub every: Option<Duration>,
    pub tags: Vec<String>,
    pub prompt: PromptSource,
    /// Files whose hashes are recorded on every run, so an outcome can be
    /// tied to the instruction version that produced it.
    pub instructions: Vec<PathBuf>,
    pub skip_permissions: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptSource {
    Inline(String),
    /// Read at launch, relative to the root, so editing it changes the next run.
    File(PathBuf),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    schema_version: u32,
    #[serde(default)]
    history: RawHistory,
    #[serde(default, rename = "job")]
    jobs: Vec<RawJob>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawHistory {
    dir: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJob {
    id: String,
    title: Option<String>,
    #[serde(default)]
    description: String,
    agent: Option<String>,
    every: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    prompt: Option<String>,
    prompt_file: Option<PathBuf>,
    #[serde(default)]
    instructions: Vec<PathBuf>,
    #[serde(default)]
    skip_permissions: bool,
}

/// Parse and validate. Every error is one readable line naming the job, since
/// the pane shows it in place of the list.
pub fn parse(text: &str) -> Result<Manifest, String> {
    let raw: RawManifest = toml::from_str(text).map_err(|e| {
        // toml's message spans lines; the first is the one that says what.
        e.message()
            .lines()
            .next()
            .unwrap_or("invalid TOML")
            .to_string()
    })?;
    if raw.schema_version != 1 {
        return Err(format!(
            "schema_version {} is not supported (this workbench reads 1)",
            raw.schema_version
        ));
    }
    let history_dir = raw
        .history
        .dir
        .unwrap_or_else(|| PathBuf::from(DEFAULT_HISTORY_DIR));
    if history_dir.is_absolute() {
        return Err("history.dir must be relative to the repository".into());
    }
    let mut jobs = Vec::with_capacity(raw.jobs.len());
    for job in raw.jobs {
        let id = job.id.trim().to_string();
        if !valid_id(&id) {
            return Err(format!(
                "job id `{id}` must be lowercase letters, digits and dashes, starting with a letter or digit"
            ));
        }
        if jobs.iter().any(|j: &JobDef| j.id == id) {
            return Err(format!("job id `{id}` is used twice"));
        }
        let prompt = match (job.prompt, job.prompt_file) {
            (Some(text), None) if !text.trim().is_empty() => PromptSource::Inline(text),
            (None, Some(path)) if !path.is_absolute() => PromptSource::File(path),
            (None, Some(_)) => {
                return Err(format!(
                    "job `{id}`: prompt_file must be relative to the repository"
                ))
            }
            (Some(_), Some(_)) => {
                return Err(format!("job `{id}`: give prompt or prompt_file, not both"))
            }
            _ => return Err(format!("job `{id}` needs a prompt or a prompt_file")),
        };
        let every = match job.every.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(text) => Some(parse_every(text).ok_or_else(|| {
                format!("job `{id}`: every `{text}` is not like 30m, 12h, 1d or 1w")
            })?),
        };
        if let Some(bad) = job.instructions.iter().find(|p| p.is_absolute()) {
            return Err(format!(
                "job `{id}`: instruction path {} must be relative to the repository",
                bad.display()
            ));
        }
        jobs.push(JobDef {
            title: job.title.unwrap_or_else(|| id.clone()),
            id,
            description: job.description.trim().to_string(),
            agent: job
                .agent
                .map(|a| a.trim().to_lowercase())
                .filter(|a| !a.is_empty()),
            every,
            tags: job.tags,
            prompt,
            instructions: job.instructions,
            skip_permissions: job.skip_permissions,
        });
    }
    Ok(Manifest {
        schema_version: raw.schema_version,
        history_dir,
        jobs,
    })
}

fn valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `30m`, `12h`, `1d`, `1w`. Nothing finer than a minute: a job is a session,
/// not a cron tick.
pub fn parse_every(text: &str) -> Option<Duration> {
    let (digits, unit) = text.split_at(
        text.trim_end_matches(|c: char| c.is_ascii_alphabetic())
            .len(),
    );
    let n: u64 = digits.trim().parse().ok().filter(|n| *n > 0)?;
    let seconds = match unit.trim() {
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        "w" => 7 * 86_400,
        _ => return None,
    };
    Some(Duration::from_secs(n * seconds))
}

/// `every` back in the form it was written, for the row.
pub fn describe_every(every: Duration) -> String {
    let secs = every.as_secs();
    if secs % (7 * 86_400) == 0 {
        format!("{}w", secs / (7 * 86_400))
    } else if secs % 86_400 == 0 {
        format!("{}d", secs / 86_400)
    } else if secs % 3600 == 0 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}m", secs / 60)
    }
}

/// What `workbench jobs init` writes: one commented example.
pub const TEMPLATE: &str = r#"# Repeatable agent jobs for this repository, listed in workbench's Jobs window (F4).
# See .workbench/jobs/README.md for the contract, the ledger and the CLI.
schema_version = 1

# [history]
# dir = ".workbench/jobs/history"    # run ledger, one JSONL file per job

[[job]]
id = "example-check"                  # unique; lowercase letters, digits, dashes
title = "Example: check the docs"
description = "Read the README and report anything out of date. Changes nothing."
# agent = "claude"                    # any configured agent command
every = "1w"                          # cadence hint; marks the row due, never runs it
tags = ["example"]
prompt = """
Read README.md and list anything that no longer matches the code.
Do not edit files. Summarize what you found.
"""
# prompt_file = "docs/jobs/example.md"   # instead of prompt: read at launch
instructions = ["README.md"]          # files hashed into every run record
skip_permissions = false
"#;
