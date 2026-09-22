//! What a job's agent is told, beyond the job's own prompt.
//!
//! The footer is the harness: it names the run, points at the lessons and the
//! history, and says exactly how to report. It is the same for every job so
//! an agent that has run one knows how to run them all.

use super::{JobDef, PromptSource};
use anyhow::{Context, Result};
use std::path::Path;

/// The job's own prompt, a `prompt_file` read from disk.
pub fn body(root: &Path, job: &JobDef) -> Result<String> {
    Ok(match &job.prompt {
        PromptSource::Inline(text) => text.trim().to_string(),
        PromptSource::File(path) => std::fs::read_to_string(root.join(path))
            .with_context(|| format!("reading prompt_file {}", path.display()))?
            .trim()
            .to_string(),
    })
}

/// The job's prompt with the harness footer.
pub fn compose(root: &Path, job: &JobDef, run_id: &str) -> Result<String> {
    Ok(format!(
        "{}\n\n{}",
        body(root, job)?,
        footer(&job.id, run_id)
    ))
}

pub fn footer(job_id: &str, run_id: &str) -> String {
    format!(
        "---\n\
Workbench job run `{run_id}` of `{job_id}`.\n\
Before you start: read `{lessons}/{job_id}.md` if it exists, and skim \
`workbench jobs history {job_id} --limit 5`.\n\
When you finish, report once:\n\
  workbench jobs report --status <completed|partial|blocked|failed> --summary \"<one paragraph>\" [--artifacts <relative path>] [--lesson \"<one reusable sentence>\"]\n\
If the job's instructions were wrong or stale, fix the files in this repo and say so in the summary. \
Never change a job's mode, account or authorization wording; that is the user's call.",
        lessons = super::LESSONS_DIR
    )
}

/// The `i` key: tighten a job from what its runs taught, and nothing more.
pub fn improve(job: &JobDef, runs: usize) -> String {
    let files: Vec<String> = job
        .instructions
        .iter()
        .map(|p| format!("`{}`", p.display()))
        .collect();
    let prompt_file = match &job.prompt {
        PromptSource::File(path) => format!(" and its prompt file `{}`", path.display()),
        PromptSource::Inline(_) => String::new(),
    };
    format!(
        "Improve the repeatable job `{id}` (\"{title}\") for this repository. Do not run it.\n\
\n\
Read, in this order:\n\
1. Its entry in `{manifest}`{prompt_file}.\n\
2. Its instruction files: {files}.\n\
3. `{lessons}/{id}.md` if it exists.\n\
4. `workbench jobs history {id} --limit {runs}` — the last runs, with status, summary, \
artifacts path and the instruction hashes each one ran under.\n\
\n\
Then: correlate outcomes with instruction versions, read the artifacts of the runs that went \
badly, and tighten what caused it — prompts, references, queries, limits, stop conditions. \
Fold lessons that are now in the instructions out of the lessons file. Keep the job's scope: \
never change its mode, account, targets' authorization or any wording about what it may publish. \
Leave every edit uncommitted and finish with a short summary of what you changed and why; \
the diff is the review.",
        id = job.id,
        title = job.title,
        manifest = super::MANIFEST,
        files = if files.is_empty() {
            "(none listed)".to_string()
        } else {
            files.join(", ")
        },
        lessons = super::LESSONS_DIR,
    )
}

/// The `n` key: author a job with the repository in front of you.
pub fn new_job() -> String {
    format!(
        "Add a repeatable job to `{manifest}` for this repository.\n\
\n\
First read `.workbench/jobs/README.md` (the contract) and the existing entries in the manifest. \
Then ask the user what the job should do, when it recurs, and which agent should run it. \
Look for instructions that already exist in this repository — playbooks, skills, job configuration \
files — and point the prompt at them rather than duplicating them. List those files under \
`instructions` so each run records their versions. Start with a read-only or draft scope unless \
the user says otherwise. Add the entry, check the file still parses as TOML, and stop: do not \
run the job.",
        manifest = super::MANIFEST
    )
}

/// `.workbench/jobs/README.md`, written by `init`.
pub const CONTRACT: &str = r#"# Repeatable agent jobs

This directory is read by [workbench](https://github.com/steferic/workbench)'s
Jobs tab and by the `workbench jobs` CLI. It is committed so every
contributor sees the same jobs and the same history.

## `jobs.toml`

One `[[job]]` per repeatable task. Fields:

| Field | Meaning |
|---|---|
| `id` | Unique, lowercase letters/digits/dashes. Names the history and lessons files. |
| `title`, `description` | What the row shows. |
| `agent` | Agent command to run it with (`claude`, `codex`, or a configured custom agent). |
| `every` | Cadence hint: `30m`, `12h`, `1d`, `1w`. Marks the job due; never starts it. |
| `prompt` / `prompt_file` | What the agent is sent. `prompt_file` is read at launch. |
| `instructions` | Files whose hashes are recorded on every run. |
| `skip_permissions` | Start the agent with permission prompts off. Default false. |
| `tags` | Free labels. |

Put the job's real configuration (queries, limits, accounts) in the project's
own files and have the prompt name them. Keep this file an index.

## Running

In workbench: `F4` opens the **Jobs** window; `Enter` on a job. A fresh agent
session starts in this project, aliased after the job, with the prompt
queued. From a shell: `workbench jobs run <id>`.

Every run's prompt ends with a footer that names the run and says how to
report. The session's environment carries `WORKBENCH_JOB` and
`WORKBENCH_JOB_RUN`.

## The ledger: `history/<id>.jsonl`

One JSON object per line; a run may have several lines and the last wins.
Append-only, so `.gitattributes` marks it `merge=union`. Each line holds the
run id, job id, status, start/end times, who ran it and where, the agent, the
session, the instruction-file hashes, a summary, an artifacts path and any
lessons. Metadata only: captures, drafts and customer data belong in the
project's own (usually ignored) artifact directories, which `artifacts` points at.

Statuses: `running`, then one of `completed`, `partial`, `blocked`, `failed`
from the agent's report; `unreported` if its turn ended without one;
`aborted` if the session died or was deleted first.

The agent closes a run with:

    workbench jobs report --status completed --summary "..." [--artifacts <path>] [--lesson "..."]

`workbench jobs history <id>` prints the ledger; `workbench jobs` lists jobs.

## Lessons and improvement

`lessons/<id>.md` collects one dated line per `--lesson`. Every run reads it
first. The `i` key in the Jobs window starts an agent that reads the last runs and
lessons and tightens the job's instructions, leaving the edits uncommitted for
review. Neither an agent nor workbench ever changes a job's mode, account or
authorization wording; that stays a human decision made in a reviewed diff.
"#;
