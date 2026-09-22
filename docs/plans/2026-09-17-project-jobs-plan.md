# Project Jobs Implementation Plan

> **Decision, 2026-09-17:** implemented as a full-screen **Jobs window**
> opened with `F4` (like the F1 help window), not as a Sessions tab as first
> written below nor as the small left-column pane tried in between. The
> window has the job list on the left and a tabbed detail on the right:
> Overview (stats, instruction hashes against the last run), Runs (the
> ledger with a cursor), Lessons, Prompt. Keys, ledger, CLI and phone view
> are as described; read "Jobs tab" as "Jobs window".

**Goal:** A Jobs tab in the Sessions pane that lists a project's repeatable
agent jobs from `.workbench/jobs.toml`, runs one on `Enter`, records every run
in a git-versioned ledger inside the repo, and gives each job a lessons file
and an "improve" action.

**Design:** `docs/plans/2026-09-17-project-jobs-design.md`. Read it first.

**Architecture:** A pure `src/jobs/` module owns the manifest, ledger, lessons
and prompt composition (no `AppState`). `src/app/jobs.rs` mirrors
`src/app/servers.rs` for tab state and actions. `src/tui/components/jobs_pane.rs`
mirrors `servers_pane.rs`. The CLI gains a `jobs` subcommand whose file
operations reuse `src/jobs/`; only `jobs run` talks to the control socket.

**Tech stack:** Rust, ratatui, crossterm, serde/toml/serde_json (all present).
`sha2` is already in `Cargo.lock` as a transitive dependency; add it to
`[dependencies]` for the instruction hashes (no new download).

Every task ends with `cargo build` clean and its tests passing via
`cargo test --offline --bin workbench <filter> --quiet`. Test names are
sentences, as in the rest of the codebase.

---

## Phase 1: core (tab, run, ledger, CLI)

### Task 1: `src/jobs/` — manifest, ledger, lessons, prompt

**Files:**
- Create: `src/jobs/mod.rs`, `src/jobs/manifest.rs`, `src/jobs/history.rs`, `src/jobs/prompt.rs`, `src/jobs/tests.rs`
- Modify: `src/main.rs` (add `mod jobs;`), `Cargo.toml` (hash crate if needed)

**Steps:**

1. Types from the design's data-model section: `Manifest`, `JobDef`,
   `PromptSource`, `RunStatus`, `RunRecord`, `Actor { user, host }`,
   `ProjectJobs`. Derive `Serialize`/`Deserialize`; `RunStatus` serializes
   as lowercase strings.
2. `manifest.rs`: `parse(text) -> Result<Manifest, String>` with validation:
   unique ids matching `^[a-z0-9][a-z0-9-]*$`, exactly one of `prompt`/
   `prompt_file`, `every` parsed from `30m|12h|1d|1w` into `Duration`,
   `history.dir` default `.workbench/jobs/history`. Errors are one readable
   line each (the pane shows them).
3. `history.rs`: `read(root, history_dir, job_id) -> Vec<RunRecord>` reads the
   JSONL, folds by `run_id` with last line wins, skips unparsable lines
   (counted, not fatal). `append(root, history_dir, &RunRecord)` opens with
   `create(true).append(true)` and writes one line plus `\n`.
   `append_lesson(root, job_id, text)` appends `- YYYY-MM-DD: text` to
   `.workbench/jobs/lessons/<job_id>.md`, creating it with a heading.
4. `mod.rs`: `find_root(from)` walks up to the first dir containing
   `.workbench/jobs.toml`. `load(root) -> Option<ProjectJobs>`. `instruction_hashes`
   returns the first 16 hex chars of sha256 per listed file (missing file →
   `"missing"`). `new_run_id()` → `YYYYMMDDTHHMMSSZ-<4 hex>`. `actor()` reads
   `git config user.name` (fallback `$USER`) and the hostname.
5. `prompt.rs`: `compose(root, job, run_id) -> Result<String>` resolves
   `PromptSource`, trims, appends the harness footer verbatim from the design.
   Also `improve_prompt(job, ...)` and `new_job_prompt()` constants for phase 2
   (add them now, they are just strings).
6. `init(root)`: writes the template manifest, `README.md` (the contract),
   `history/.gitkeep`, `lessons/.gitkeep`, appends
   `.workbench/jobs/history/*.jsonl merge=union` to `.gitattributes` if absent.
   Refuses when the manifest exists. Returns the created paths.

**Tests (`src/jobs/tests.rs`, all on `tempfile::tempdir`):**
- a manifest with two jobs parses and keeps manifest order
- a duplicate id, a bad id, and a job with both prompt forms are each rejected with a message naming the job
- every accepts 30m 12h 1d 1w and rejects 2x
- history folds two lines for one run into the later status
- an unparsable history line is skipped and the rest still load
- appending twice from two writers yields two independent lines
- compose reads prompt_file relative to the root and ends with the report command
- init creates the files, adds the gitattributes line once, and refuses a second time

---

### Task 2: session link and spawn environment

**Files:**
- Modify: `src/models/session.rs` (`Session.job: Option<JobLink>`, `#[serde(default)]`)
- Modify: `src/pty/manager.rs` (`SessionSpawnConfig.extra_env: Vec<(String, String)>`, applied next to `WORKBENCH_SESSION` at `manager.rs:598`)
- Modify: `src/app/handlers/session.rs` (`create_session_in` and `finish_worktree_session_spawn` take an `extra_env` parameter; add a thin `create_job_session(state, ws, agent, skip, worktree, env) -> Option<Uuid>` wrapper so existing call sites do not change)

**Steps:**
1. Add `JobLink { job_id, run_id }` and the field. Persistence: `PersistedState` needs no migration because of `serde(default)`; add a test that a v1 `state.json` without the field still loads (`src/persistence.rs` tests).
2. Thread `extra_env` through `SessionSpawnConfig`; existing callers pass `Vec::new()`.
3. `create_session_in` returns the session id in both the direct and worktree paths already; keep that.

**Tests:**
- a session persisted without a job link deserializes with `job: None`
- spawn config env includes the job variables when given (unit test on the env assembly helper, factored out so no PTY is needed)

---

### Task 3: tab state, actions and handler (`src/app/jobs.rs`)

**Files:**
- Create: `src/app/jobs.rs`, `src/app/jobs/tests.rs`
- Modify: `src/app/mod.rs` (`pub(crate) mod jobs;`), `src/app/state/types.rs` (`SessionsTab::Jobs`, `toggle()` 4-cycle), `src/app/state/ui.rs` (`UIState.jobs: JobsUi`, init), `src/app/state/system.rs` (`project_jobs: HashMap<Uuid, ProjectJobs>`, `last_jobs_scan`, `jobs_scan_inflight`), `src/app/state/mod.rs` (`session_visual_order` returns empty for `Jobs`, like `Servers` at `state/mod.rs:360`), `src/app/action.rs`

**Actions to add** (next to the server actions at `action.rs:80`):
`SelectJob(Uuid, String)`, `JobRun`, `JobRunForce`, `JobDetails`, `JobImprove`,
`JobNew`, `JobsScope`, `JobsRefresh`, `JobsClose`,
`JobsScanned(HashMap<Uuid, ProjectJobs>)`, `JobSpawned { workspace, job_id, run_id, session: Option<Uuid> }`.

**`JobsUi`:** `scope_all`, `selected: Option<(Uuid, String)>`, `offset`,
`dialog: Option<JobDialog { workspace, job_id }>`, `message: Option<String>`.

**Functions (mirroring `servers.rs`):**
- `rows(state) -> Vec<JobRow { workspace, name, job: JobDef, last: Option<RunRecord>, due: bool, open_session: Option<Uuid>, error: Option<String> }>` — scoped by `scope_all`; `open_session` is any live session whose `job.run_id` is a `running` record.
- `reconcile`, `move_selection` — copy the server versions.
- `handle(state, action, tx)`:
  - `JobRun`: if the row has an `open_session`, set it active and focus; else fall through to `JobRunForce`.
  - `JobRunForce`: `run_id`, `instruction_hashes`, `append(running)`, `compose_prompt`; on compose error set `message` and stop. `create_job_session(..., env=[WORKBENCH_JOB, WORKBENCH_JOB_RUN])`; then `session.alias = Some(job.id)`, `session.job = Some(link)`, `session.todo_queue.add(prompt)`, `save_state`. Update the in-memory `project_jobs` so the row shows running immediately. Toast `Started <title>`.
  - `JobDetails` / `JobsClose`: open/close the dialog.
  - `JobsScope`, `JobsRefresh` (`last_jobs_scan = None`).
  - `JobImprove`, `JobNew`: phase 2, leave `todo!()`-free stubs that set `message = "coming soon"` for now.
- `scan(roots: Vec<(Uuid, PathBuf)>, previous_mtimes) -> HashMap<Uuid, ProjectJobs>` — off-thread; compares manifest + history dir mtimes and only reloads changed projects.

**Closing runs:** in `src/app/todo_dispatch.rs::retire_finished` (after `finish_running()`), if the session has a `job` link whose latest record is `running`, append `unreported` with `ended_utc`. In `src/app/cleanup.rs` (session exit/delete path), append `aborted` likewise. Put the shared logic in `jobs::close_run_if_open(state, session_id, status)`.

**Handler wiring** (`src/app/handler.rs`):
- Pre-dispatch fast path next to the server one at `handler.rs:47`: route all `Job*`/`Jobs*` actions to `jobs::handle`.
- Rename `state.ui.servers.hits` to `state.ui.sessions_hits` (one shared hit list for the Sessions pane) since `handler.rs:29` only inspects that vec; both panes push into it. Small mechanical refactor; keep it in this task.
- `Tick` arm: `scan_jobs(state, tx)` with `JOBS_SCAN_EVERY = 5s`, modelled on `scan_ports` at `handler.rs:1176`. Apply `JobsScanned` by replacing `project_jobs` and calling `jobs::reconcile`.
- Add the new actions to the "already handled" no-op arm at `handler.rs:545`.
- `src/app/handlers/navigation.rs`: `MoveUp`/`MoveDown` (`:111`), wheel (`:60`) and `ToggleSessionsTab` (`:232`) branches gain the `Jobs` case.

**Tests (`src/app/jobs/tests.rs`):**
- Tab cycles Agents → Terminals → Servers → Jobs → Agents and the cursor lands on a drawn row (extend `navigation.rs:955`)
- pressing Enter on a job starts one session aliased after it with the prompt queued and a running line in the ledger (tempdir workspace; assert `todo_queue.next_pending()` text ends with the report command, and env contains the run id)
- Enter on a job with an open run focuses that session instead of starting another; R starts another
- a retired todo closes the run as unreported; a deleted session closes it as aborted
- a broken manifest renders one error row and Enter does nothing but set a message
- scan skips a project whose files have not changed (mtime test)

---

### Task 4: the pane (`src/tui/components/jobs_pane.rs`)

**Files:**
- Create: `src/tui/components/jobs_pane.rs`
- Modify: `src/tui/components/mod.rs`, `src/tui/components/session_list.rs` (tabs array `:263`, labels `:277`, delegation `:52`, action-bar height `:38`, empty-state match `:202`), `src/tui/ui.rs` (`jobs_pane::dialog` next to `servers_pane::dialog` at `:274`), `src/tui/event/handlers.rs` (early-return key map for `Jobs` next to the Servers one at `:150`, dialog keys next to `:65`)

**Steps:**
1. `render(frame, list_area, actions, state)`: scope header, two-line rows
   as in the design mock, `due`/`● running`/`every` badges, footer message,
   buttons `[Enter run] [d details] [i improve] [n new] [a scope] [r refresh]`.
   All colours via `crate::theme::current()`; no hardcoded chrome colours.
2. `dialog(frame, state)`: centered modal with description, prompt preview
   (first 8 lines), instruction files, last 10 runs (`date status by · summary`),
   lessons count. Footer buttons `[Esc close] [Enter focus session]`.
3. Tab bar: four labels degrade to `["Agents","Terminals","Servers","Jobs"]`,
   `["Agents","Terms","Servers","Jobs"]`, `["A","T","S","J"]`. Check the
   20-column case in `tab_spans`.
4. Keys: `Enter`→`JobRun`, `R`→`JobRunForce`, `d`→`JobDetails`, `i`→`JobImprove`,
   `n`→`JobNew`, `a`→`JobsScope`, `r`→`JobsRefresh`, `j/k/↑/↓`→move,
   `Tab`→`ToggleSessionsTab`; dialog: `Esc`→`JobsClose`, `Enter`→focus.
5. Session rows in the Agents tab: append a `⚙` marker after the alias when
   `session.job.is_some()` (`session_list.rs:471`).

**Tests (inline, render-to-buffer like `session_list.rs:499`):**
- the jobs tab draws every job title and marks the due one
- every registered hit rect is inside the viewport at 120x40, 80x24, 60x18, 20x8
- clicking the Jobs tab header switches tabs; clicking a row selects it
- the details modal lists the last runs newest first

---

### Task 5: CLI `workbench jobs`

**Files:**
- Modify: `src/main.rs` (`Commands::Jobs { cmd: JobsCmd }` with `List`, `Run`, `History`, `Report`, `Init`), `src/cli.rs` (`cmd_jobs_*`)
- Modify: `src/control/mod.rs` (`"jobs.run"` in `dispatch` at `:410` and `schema()` at `:686`), `src/remote/server.rs` (`RemoteCommand::RunJob { project, job_id }`), `src/app/handler.rs::apply_remote` (`:1531` shape: early block dispatching `Action::JobRunForce` with the project's workspace selected; add the variant to the exhaustive matches at `:1576` and `:1598`)

**Steps:**
1. Root resolution helper: `--project <name|path>` → `projects.list` over the socket when available; else `WORKBENCH_WORKSPACE`; else `jobs::find_root(cwd)`. Fail with a one-line message naming what was tried.
2. `list`: table `id  title  every  last(status, when, by)`; `--json`.
3. `history <id> [--limit N] [--json]`: newest first.
4. `report --status S --summary T [--artifacts P] [--lesson L] [--run ID]`:
   run id from `--run` or `WORKBENCH_JOB_RUN`; job id from the run's existing
   record (read the ledger) or `WORKBENCH_JOB`. Appends a record cloned from the
   latest line with the new status, `ended_utc = now`, summary, artifacts,
   lessons; also `append_lesson` for each `--lesson`. Prints the path written.
5. `run <id>`: `Client::connect` → `jobs.run {project, job_id}`; print the reply.
6. `init [path]`: `jobs::init`, print created paths.

**Tests:**
- report without a run id or env fails with a message that names both options
- report appends a line whose status and summary round-trip through history
- jobs.run is listed in api.schema (extend the existing schema test if there is one)

---

### Task 6: zeta-studio pilot manifest

**Files (in `~/code/zeta-studio`, a separate commit there):**
- Create: `.workbench/jobs.toml` with one entry per "Start here" row of `ops/sundream/README.md` (tiktok-creation-questions, instagram-comment-review, reddit-launch-post, reddit-comment-review, signup-email), prompts copied from the README's invocation examples, `instructions` listing the job YAML, the skill's `SKILL.md`, `job-contract.md` and `product-facts.md`.
- Create: `.workbench/jobs/README.md`, `history/.gitkeep`, `lessons/.gitkeep` (via `workbench jobs init` then edit).
- Modify: `.gitattributes` (union line), `ops/sundream/README.md` (one paragraph: how to trigger from workbench, where the ledger is, that `runs/` stays the artifact home).

**Verification:** open zeta-studio in workbench, Tab to Jobs, run
`tiktok-creation-questions` (research mode, read-only), confirm the session is
aliased, the prompt arrives, the agent reports, and the ledger line closes
with `completed`. Then `git diff` in zeta-studio shows only the ledger line.

---

### Task 7: docs and help

- `README.md`: feature bullet after the Servers one (`:17`) and a "Project jobs" section modelled on "Managing local servers" (`:63`): manifest, keys, CLI, ledger, lessons.
- `src/tui/components/status_bar.rs:332`: add `[Tab]` hint text for the Sessions panel if missing.
- `src/tui/components/config_window.rs:169`: Sessions section of the QuickRef gains the Jobs keys.
- `src/tui/components/command_palette.rs`: entries `Jobs: run selected`, `Jobs: refresh`.

---

## Phase 2: lessons, improve, new, phone

### Task 8: `i` improve and `n` new

- `JobImprove`: spawn a session aliased `improve:<id>` with `jobs::prompt::improve_prompt(job, lessons, last_runs)`; no ledger line; toast.
- `JobNew`: if no manifest, run `jobs::init` first (toast the created paths); spawn a session aliased `new-job` with `new_job_prompt()`.
- Tests: improve does not append to the ledger; new on an empty project creates the manifest before spawning.

### Task 9: phone

- `JobView` next to `ServerView` (`src/remote/mod.rs:162`), `ProjectView.jobs` filled where `servers` is (`:448`, `:516`).
- `page.rs:2550`: jobs list in the project card with a Run button; `POST /api/job` via `command_from` (`server.rs:411` is the model) → `RemoteCommand::RunJob`.
- `tests/mobile-page.test.cjs`: a project with jobs renders them and Run posts the job id.

## Phase 3 (optional, off by default)

- `auto = true` per job: when due and no open run, queue it, capped like
  `wake_idle_managers` (`todo_dispatch.rs:231`) with a daily cap in
  `user_config.toml`. Not built until the manual flow has run for a while.
