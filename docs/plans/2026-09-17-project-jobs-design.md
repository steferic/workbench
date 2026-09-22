# Project Jobs Design

> **Decision, 2026-09-17:** implemented as a full-screen **Jobs window**
> opened with `F4` (like the F1 help window), not as a Sessions tab as first
> written below nor as the small left-column pane tried in between. The
> window has the job list on the left and a tabbed detail on the right:
> Overview (stats, instruction hashes against the last run), Runs (the
> ledger with a cursor), Lessons, Prompt. Keys, ledger, CLI and phone view
> are as described; read "Jobs tab" as "Jobs window".

## Summary

A **Jobs** tab in the Sessions pane lists the repeatable agent jobs a project
declares in a manifest committed to its repo (`.workbench/jobs.toml`). Pressing
`Enter` on a job spawns a fresh agent session in that workspace, names it after
the job, and queues the job's prompt plus a short harness footer. Every run is
recorded as metadata in an append-only, git-versioned ledger inside the repo, so
teammates who pull the repo see what ran, when, by whom, and how it went. A
lessons file per job and a human-triggered "improve this job" action close the
loop between runs and the instructions that drive them.

The pilot project is `zeta-studio`, whose `ops/sundream/` already has YAML job
configurations, a job contract, repo-scoped skills, per-run artifact directories
and a versioned social-history ledger. Workbench does not replace any of that.
It sits above it: the manifest points at the existing skill + YAML invocations
from the ops README's "Start here" table, and the ledger records launches and
outcomes while the project keeps owning its own artifacts.

## What exists today

| Concern | Today | Where |
|---|---|---|
| Sessions pane tabs | `Agents`, `Terminals`, `Servers`; a hard-coded 3-cycle | `src/app/state/types.rs:123` |
| Tab-specific pane | `servers_pane.rs` + `app/servers.rs` (state, actions, refresh, tests) | added by commit `04f6dd5` |
| Spawning a session in a named workspace | `create_session_in(state, ws, agent, skip_perms, worktree) -> Option<Uuid>` | `src/app/handlers/session.rs:416` |
| Sending a prompt to a session | queue it on `session.todo_queue`; `todo_dispatch` delivers it when the agent is free | `src/app/todo_dispatch.rs:106` |
| Turn-end / blocked detection | `retire_finished` + `agent_status::Activity` | `todo_dispatch.rs:355`, `src/agent_status.rs:59` |
| Naming a session | `Session.alias` (set only by `workbench alias` today) | `src/models/session.rs:41` |
| Per-project files in the repo | none read; only the `CLAUDE.local.md`/`AGENTS.md` brief is written | `src/comms.rs:600` |
| CLI to TUI | control socket (`control/mod.rs::dispatch`) + comms inbox files | `src/control/mod.rs:410` |
| Periodic work | elapsed-time checks in the `Tick` arm; no scheduler | `src/app/handler.rs:118` |
| Phone view | `ProjectView.servers` rendered in the project card | `src/remote/mod.rs:146`, `page.rs:2548` |
| zeta-studio jobs | `ops/sundream/jobs/*.yaml`, `job-contract.md`, `.agents/skills/sundream-*`, `runs/<id>/` (gitignored), `state/social-history.jsonl` (versioned) | `~/code/zeta-studio` |

Two existing rules shape the design:

- Workbench keeps its own state outside repos "so nothing dirties git status"
  (`src/comms.rs:3`). Jobs deliberately break that rule, but only for a project
  that opts in by committing a manifest, and only into `.workbench/`.
- The zeta ops contract says a schedule, a YAML flag or a discovered skill is
  never authorization to publish. Workbench therefore never runs a job on its
  own in v1, and never rewrites a job's `mode`, `account` or authorization text.

## The manifest: `.workbench/jobs.toml`

Workbench's own format, TOML (already a dependency; no YAML crate is added).
The manifest is thin. It says what to show, what to send, and where to record.
Project-specific configuration (queries, limits, accounts) stays in the
project's own files, which the prompt references.

```toml
schema_version = 1

[history]
dir = ".workbench/jobs/history"     # default; zeta-studio may point at ops/sundream/history

[[job]]
id = "tiktok-creation-questions"     # stable, filename-safe, unique
title = "TikTok: creation questions"
description = "Find and qualify creation questions in TikTok comments. Read-only."
agent = "claude"                     # any configured agent command; default: claude
every = "1d"                         # cadence hint only; marks the row due/overdue
tags = ["research", "tiktok"]
prompt = """
Use $sundream-tiktok-research with ops/sundream/jobs/tiktok-creation-questions.yaml.
Inspect up to three videos and return candidates and metrics. Do not post.
"""
# prompt_file = "ops/sundream/prompts/tiktok.md"   # alternative to inline prompt
instructions = [                     # files whose hashes are recorded per run
  "ops/sundream/jobs/tiktok-creation-questions.yaml",
  ".agents/skills/sundream-tiktok-research/SKILL.md",
  "ops/sundream/job-contract.md",
]
worktree = false
skip_permissions = false
```

Rules:

- `id` must be unique and match `^[a-z0-9][a-z0-9-]*$`; it names the history
  and lessons files.
- Exactly one of `prompt` / `prompt_file`. `prompt_file` is relative to the
  repo root and read at launch, so editing the file changes the next run.
- `every` accepts `30m`, `12h`, `1d`, `1w`. Absent means on demand.
- `agent` resolves through the same table as the create-session dialog
  (built-in and `user_config.toml` custom agents). Unknown values render the row
  with an error and refuse to run.
- A malformed manifest renders as a single error row with the TOML message; it
  never panics the pane.

## The ledger: `.workbench/jobs/history/<job-id>.jsonl`

Append-only, one JSON object per line, committed to git. Each line is a full
snapshot of a run record; readers fold lines by `run_id` and the last line
wins. Because nothing is ever edited in place, `merge=union` in `.gitattributes`
makes concurrent appends from two machines merge without conflict.

```json
{"schema_version":1,"run_id":"20260917T141205Z-3f9a","job_id":"tiktok-creation-questions",
 "status":"running","started_utc":"2026-09-17T14:12:05Z","ended_utc":null,
 "by":{"user":"Stefan Le Noach","host":"stefans-mbp"},"agent":"claude","session":"3f9a1c2d",
 "instructions_sha256_16":{"ops/sundream/jobs/tiktok-creation-questions.yaml":"121b1e33b79f371d","...":"..."},
 "summary":null,"artifacts":null,"lessons":[]}
```

Status values: `running`, `completed`, `partial`, `blocked`, `failed`
(reported by the agent, matching the zeta contract's vocabulary), plus two that
workbench sets itself: `unreported` (the agent's turn ended and no report
arrived) and `aborted` (the session died or was deleted with the run open).

What goes in the ledger is metadata only: who, when, where, which instruction
versions, a one-paragraph summary, a relative path to the project's own
artifact directory, and short lessons. Raw captures, drafts and anything with
customer data stay where the project keeps them today (`ops/sundream/runs/`,
which is gitignored). The ledger is safe to commit and safe for teammates to
read.

## The run lifecycle

1. `Enter` on a job (or `workbench jobs run <id>`, or the phone) dispatches
   `Action::JobRun { workspace, job_id }`.
2. The handler allocates `run_id`, computes the instruction hashes, appends a
   `running` line, and calls `create_session_in(...)`. The spawn gets two extra
   environment variables, `WORKBENCH_JOB` and `WORKBENCH_JOB_RUN`, so the
   CLI inside that session knows which run it belongs to without arguments.
3. The new session gets `alias = job.id` and `session.job = Some(JobLink {
   job_id, run_id })` (persisted, so a restart keeps the link).
4. The composed prompt is added to the session's `todo_queue`. The existing
   dispatcher delivers it once the agent is idle, exactly like the phone does
   when it wakes a stopped agent (`handler.rs:1618`).
5. The agent works. When done it runs
   `workbench jobs report --status completed --summary "..." --artifacts ops/sundream/runs/... [--lesson "..."]`.
   The CLI appends the line to the ledger directly (the file is the source of
   truth and works even if the TUI is down); the TUI notices on its next scan.
6. If the queued item retires (`retire_finished`) and the run is still
   `running`, workbench appends `unreported`. If the session exits or is deleted
   first, it appends `aborted`. A late report still wins because it is a later
   line.

The composed prompt is the job's prompt followed by a fixed harness footer:

```
---
Workbench job run `<run_id>` of `<job_id>`.
Before you start: read `.workbench/jobs/lessons/<job_id>.md` if it exists, and
skim `workbench jobs history <job_id> --limit 5`.
When you finish, report once:
  workbench jobs report --status <completed|partial|blocked|failed> --summary "<one paragraph>" [--artifacts <relative path>] [--lesson "<one reusable sentence>"]
If the job's instructions were wrong or stale, fix the files in this repo and
say so in the summary. Never change a job's mode, account or authorization
wording; that is the user's call.
```

## The Jobs tab

Rendering mirrors `servers_pane.rs`: a scope header, two-line rows, a status
footer, and a row of clickable action buttons. Rows are ordered as in the
manifest, with due jobs marked.

```
 Jobs · zeta-studio                                 a:all projects
 ▸ TikTok: creation questions           due   every 1d
   last: 2026-09-16 completed by Stefan · 27 questions, 5 fits
   Reddit: comment review                     every 12h   ● running (3f9a)
   last: running since 14:12 by Stefan
   Signup email                                on demand
   last: never
 [Enter run] [d details] [i improve] [n new] [r refresh]
```

Keys on the tab (early-return key map like the Servers tab, so `1-4`/`t`
do not create sessions):

| Key | Action |
|---|---|
| `Enter` | Run the job. If a run from this workbench is still open, jump to its session instead; `R` forces a new run. |
| `d` | Details modal: description, prompt preview, instruction files, last 10 runs with status/summary/artifacts. `Enter` in the modal focuses the run's session if it is still alive. |
| `i` | Spawn an "improve" session (see below). |
| `n` | Spawn a "new job" session (see below). |
| `a` | Toggle selected project / all projects. |
| `r` | Rescan now. |
| `j`/`k`, arrows, wheel, click | Move selection. |

Sessions started by a job show their alias in the Agents tab as today, plus a
small job marker so the link is visible at a glance.

A project without a manifest shows a one-line hint: `No jobs. n creates
.workbench/jobs.toml with an agent's help, or run: workbench jobs init`.

## Refresh

The manifest, ledger and lessons are files, so the Jobs tab reads them the way
the Servers tab scans ports: a `JOBS_SCAN_EVERY` (5s) check in the `Tick` arm,
guarded by an in-flight flag, on `spawn_blocking`, returning
`Action::JobsScanned(HashMap<Uuid, ProjectJobs>)`. The scan first compares
mtimes of the manifest and history directory per workspace and skips
unchanged projects, so the steady-state cost is a handful of `stat` calls.

## Self-improvement, bounded

The loop is: run → report (summary, lessons, instruction hashes) → lessons file
→ next run reads it. Two things make that recursive without making it
unsupervised:

1. **Lessons file**: `.workbench/jobs/lessons/<job-id>.md`, versioned, appended
   by `--lesson` at report time (one dated bullet per lesson). The harness
   footer makes every run read it first. This is the zeta README's "promote
   concise, anonymized lessons" rule, made mechanical.
2. **Improve action (`i`)**: spawns a session whose prompt is a workbench
   built-in: read the job's manifest entry, instruction files, lessons and the
   last 10 ledger lines; correlate outcomes with instruction hashes; tighten
   prompts, references, queries and limits; never change `mode`, `account` or
   authorization text; do not run the job; leave the edits uncommitted so the
   diff is the review. The session is aliased `improve:<job-id>` and is not a
   ledger run.

Nothing edits instructions automatically. A human presses `i`, and git shows
what changed. That matches the zeta contract and keeps the blast radius of a
bad lesson to one reviewed diff.

## Creating the structure for any repo

- `workbench jobs init [path]` writes `.workbench/jobs.toml` (commented
  template with one example job), `.workbench/jobs/README.md` (the contract
  above: manifest fields, ledger format, report command, lessons and improve
  rules), `.workbench/jobs/history/.gitkeep`, `.workbench/jobs/lessons/.gitkeep`
  and appends the `merge=union` line to `.gitattributes`. It refuses to
  overwrite an existing manifest.
- `n` in the Jobs tab runs `init` if needed and then spawns a session aliased
  `new-job` with a built-in prompt: read the repo's existing docs, ask the user
  what the job should do, add a manifest entry that points at existing
  instructions rather than duplicating them, and stop before running it.

For zeta-studio the first manifest is written by hand as part of this work
(see the plan), with one entry per row of the ops README's "Start here" table.

## CLI

```
workbench jobs                       # list jobs of the repo containing cwd (or --project)
workbench jobs run <id>              # ask the running TUI to start it (control socket)
workbench jobs history <id> [--limit N] [--json]
workbench jobs report --status ... --summary ... [--artifacts ...] [--lesson ...] [--run <id>]
workbench jobs init [path]
```

`list`, `history`, `report` and `init` read and write repo files directly and
need no TUI. `report` finds the run from `WORKBENCH_JOB_RUN` or `--run`, and
the repo from `WORKBENCH_WORKSPACE`, `--project` or a walk up from cwd looking
for `.workbench/jobs.toml`. `run` goes through the control socket's new
`jobs.run` method, which queues `RemoteCommand::RunJob { project, job_id }`,
the same path the phone uses.

## Phone view

`ProjectView` gains `jobs: Vec<JobView { id, title, due, last_status,
last_at, running_session }>`, rendered in the expanded project card next to
servers, each with a Run button posting to `/api/job`. Teammates on the
tailnet can therefore see and trigger jobs without the TUI.

## Data model (Rust)

```rust
// src/jobs/mod.rs — pure file I/O and types; no AppState
pub struct Manifest { pub schema_version: u32, pub history_dir: PathBuf, pub jobs: Vec<JobDef> }
pub struct JobDef { pub id: String, pub title: String, pub description: String,
    pub agent: Option<String>, pub every: Option<Duration>, pub tags: Vec<String>,
    pub prompt: PromptSource, pub instructions: Vec<PathBuf>, pub worktree: bool, pub skip_permissions: bool }
pub enum PromptSource { Inline(String), File(PathBuf) }
pub enum RunStatus { Running, Completed, Partial, Blocked, Failed, Unreported, Aborted }
pub struct RunRecord { pub run_id: String, pub job_id: String, pub status: RunStatus,
    pub started_utc: DateTime<Utc>, pub ended_utc: Option<DateTime<Utc>>, pub by: Actor,
    pub agent: String, pub session: Option<String>, pub instructions_sha256_16: BTreeMap<String, String>,
    pub summary: Option<String>, pub artifacts: Option<String>, pub lessons: Vec<String> }
pub struct ProjectJobs { pub manifest: Result<Manifest, String>, pub runs: HashMap<String, Vec<RunRecord>>, pub lessons: HashMap<String, String> }

pub fn find_root(from: &Path) -> Option<PathBuf>;
pub fn load(root: &Path) -> Option<ProjectJobs>;          // None when no manifest
pub fn append_run(root: &Path, history_dir: &Path, rec: &RunRecord) -> Result<()>;
pub fn append_lesson(root: &Path, job_id: &str, lesson: &str) -> Result<()>;
pub fn compose_prompt(root: &Path, job: &JobDef, run_id: &str) -> Result<String>;
pub fn instruction_hashes(root: &Path, job: &JobDef) -> BTreeMap<String, String>;
pub fn init(root: &Path) -> Result<Vec<PathBuf>>;

// src/models/session.rs
pub struct JobLink { pub job_id: String, pub run_id: String }   // Session.job: Option<JobLink>, serde(default)

// src/app/jobs.rs — tab state + action handling, mirrors app/servers.rs
pub struct JobsUi { pub scope_all: bool, pub selected: Option<(Uuid, String)>, pub offset: usize,
    pub dialog: Option<JobDialog>, pub message: Option<String> }
```

## Out of scope for v1

- Running jobs on a timer. `every` only marks rows due. The automation hub
  already owns unattended runs for zeta-studio, and the ops contract is
  explicit that a schedule is not authorization. A capped auto-queue (like
  `wake_idle_managers`) is a possible phase 3, off by default.
- Parsing the project's own job YAML. The manifest references it; the agent
  reads it.
- A TUI form for authoring jobs. The `n` action delegates authoring to an
  agent, which can read the repo.
