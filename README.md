# Workbench

[![Check](https://github.com/steferic/workbench/actions/workflows/check.yml/badge.svg)](https://github.com/steferic/workbench/actions/workflows/check.yml)

A TUI for managing AI agent workspaces and sessions. Run Claude, Codex, Gemini, and other coding agents side by side across multiple projects, with per-workspace sessions, pinned terminals, a live view of the selected agent's task list, and git worktree isolation.

## Features

- Multiple workspaces, each with its own agents and terminals
- TODO pane: queue up work for an agent and it gets through the list one item at a time, sending the next when a turn ends — unless the agent is blocked on you or you are mid-conversation with it. The agent's own steps show under whatever is running (Claude, Codex, opencode, hermes)
- Phone view over Tailscale: every agent's status across projects, queue work, and approve or deny a blocked agent from your phone — served on the tailnet address only, never a public port, with a scannable QR in Utilities
- Live status reported by the agent itself: a session stopped at a permission prompt is flagged `!` instead of looking idle, in its session row, its project row, and the status bar (Claude; Codex in ⚡ mode, which is what lets its hooks run)
- Restart restores each agent's *own* conversation, so several agents in one project keep separate histories
- Run agents in isolated git worktrees and merge their work back with one key
- Parallel tasks: race several agents on the same prompt in separate worktrees
- Pinned terminal panes alongside the agent output
- Servers tab in Sessions: inspect project servers across all workspaces, open their URLs, and stop unused processes
- Jobs window (`F4`): a project's repeatable agent jobs, declared in a manifest committed to its repo, with their stats and run history; `Enter` runs one in a fresh agent, every run lands in a git-versioned ledger your teammates can read, and `i` starts an agent that tightens the job from what its runs taught
- Local repository map: open any workspace as a searchable, live file tree on a clean light infinite canvas, with read-only highlighted code previews and agent-generated explanations, highlights, notes, connections, groups, and diagrams
- Scrollback reconstruction for full-screen agents (Claude, Codex)
- Dark/light themes, mouse support, clipboard integration

## Install

### One-line install (macOS / Linux)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/steferic/workbench/releases/latest/download/workbench-installer.sh | sh
```

### One-line install (Windows PowerShell)

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/steferic/workbench/releases/latest/download/workbench-installer.ps1 | iex"
```

Prebuilt binaries for macOS (Apple Silicon + Intel), Linux (x86_64 + arm64), and Windows (x86_64) are also on the [releases page](https://github.com/steferic/workbench/releases).

> **Linux note:** the binary needs the ALSA runtime for sounds, which every desktop distro already ships. On minimal/headless systems: `sudo apt install libasound2` (Debian/Ubuntu) or `sudo dnf install alsa-lib` (Fedora).

### From source

Requires a recent stable [Rust](https://www.rust-lang.org/tools/install) toolchain. macOS and Windows need nothing else; Linux needs the audio build deps:

```bash
sudo apt install pkg-config libasound2-dev     # Debian/Ubuntu
# sudo dnf install pkgconf-pkg-config alsa-lib-devel   # Fedora
```

Then, on any OS:

```bash
cargo install --git https://github.com/steferic/workbench
```

Or clone and build:

```bash
git clone https://github.com/steferic/workbench.git
cd workbench
cargo build --release   # binary at target/release/workbench
```

## Managing local servers

Click **Servers** in the Sessions pane, or focus Sessions and press `Tab` to cycle through Agents, Terminals, and Servers. The list refreshes every five seconds and includes servers running inside your open projects and their agent worktrees. It works without phone access enabled.

- `a`: switch between all projects and the selected project
- `↑` / `↓` (or `k` / `j`): select a server
- `Enter`: show its project, process, PID, directory, URL, and ports
- `o`: open the URL in your browser
- `x`: ask to stop the selected server; `Enter` confirms and `Esc` cancels
- `r`: refresh now

Stopping checks that the process still matches the selected row, then terminates it and its children. All ports held by that process close; a supervisor may restart it. Phone forwarders are removed when their backend disappears. Workbench's own listeners and forwarders are excluded from the list. Discovery uses `lsof`; stopping is supported on macOS and Linux.

## Scrollback

Scrolling above live output opens a conversation history for Claude and Codex, read from that session's own log. Markdown headings, lists, tables, code and links are rendered consistently; tool results retain their full text. History stays separate from the live screen so the latest answer is not repeated. The passage you are reading stays anchored as new output arrives or the pane changes width.

Other agents and terminals use the terminal parser's stored cells, preserving colors, links and soft line wraps without replaying old cursor commands. Normal terminal buffers reflow when resized. A full-screen application that erases its output still needs a provider log to recover the erased conversation; terminal cells alone cannot reconstruct it reliably.

## Project jobs

A project can declare repeatable agent jobs — the email triage, the comment review, the weekly research pass — in `.workbench/jobs.toml` at its root. `F4` opens the **Jobs** window over the whole screen: the project's jobs on the left, and on the right the selected job in four tabs. **Overview** is what to read before pressing Enter — the description, the ledger's totals (runs by outcome, success rate, mean duration, who has run it), and each instruction file's hash now against the hash the last run recorded, so an edit nobody has run yet is called out. **Runs** is the ledger newest first with the cursor's run spelled out below it. **Lessons** and **Prompt** are the two things the next run will read, the prompt shown exactly as the agent will get it, footer included. `Enter` runs the job: the window closes, a fresh agent starts in that project aliased after the job, and the output pane shows it begin.

- `Enter`: run the selected job, or jump to its session if a run is already open here; `R` starts another regardless
- `Tab` / `1`-`4`: move between the list and the detail; pick a detail tab. `j`/`k` walk whichever side has the cursor
- `i`: start an agent that reads the job's runs and lessons and tightens its instructions, leaving the diff for review
- `n`: create the manifest if the project has none, then start an agent that adds a job with you
- `a`: all projects or the selected one; `r`: re-read the files now (they are also re-read every five seconds, and when the window opens)
- `Esc`, `q` or `F4`: close

The manifest is an index, not a copy: the job's real configuration stays in the project's own files and the prompt names them. Each entry has an `id`, a `title`, a `prompt` (or `prompt_file`), optional `every` (`30m`, `12h`, `1d`, `1w` — marks the row due, never starts it), `agent`, and `instructions`, the files whose hashes are recorded on every run.

Every run is one JSON line in `.workbench/jobs/history/<id>.jsonl`: who, when, where, the agent, the instruction versions, then the agent's own report — status, a summary, an artifacts path, lessons. It is append-only and merges by union, so it is committed and two machines never conflict. Metadata only; captures and customer data stay in the project's own directories. The agent closes a run with

```sh
workbench jobs report --status completed --summary "..." [--artifacts <path>] [--lesson "..."]
```

and a run whose agent ends its turn without reporting is marked `unreported`; one whose session dies first, `aborted`. `--lesson` appends to `.workbench/jobs/lessons/<id>.md`, which every later run reads first.

```sh
workbench jobs init             # scaffold .workbench/jobs.toml, README, history and lessons dirs
workbench jobs                  # list the jobs of the repo you are in
workbench jobs run <id>         # what Enter does, from a shell or another agent
workbench jobs history <id>     # the ledger, newest first
```

`init`, `list`, `history` and `report` read and write the repo directly and need no running workbench. The phone view lists each project's jobs with a Run button.

## Agents

Install the CLIs for whichever agents you want to drive (workbench spawns them by command name):

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) — `claude`
- [Codex CLI](https://github.com/openai/codex) — `codex`
- [Gemini CLI](https://github.com/google-gemini/gemini-cli) — `gemini`
- [Grok CLI](https://github.com/xai-org/grok) — `grok`

Custom agents (any command) can be added in `~/.config/workbench/user_config.toml` or via the in-app settings (`h`).

### Optional

- **VLC** — only needed for the classical radio stream (`brew install vlc` / `apt install vlc`). All other sounds are built in.

## Usage

```bash
workbench                                  # run the TUI
workbench --workspace /path/to/project    # open a specific workspace
workbench add /path/to/project            # register a workspace
workbench list                            # list workspaces
workbench prompts                         # analyze submitted prompts
workbench prompts --json                  # export recent prompts with metadata
```

## Images, video, and links

Agents can present files directly:

```sh
workbench preview ./screenshot.png
workbench preview ./demo.mp4
# From an ordinary shell, name the agent that owns the preview:
workbench preview ./screenshot.png --agent <id-or-alias>
```

The focused agent's preview opens in a dismissible overlay. **Esc** closes it;
**B** or **Enter** opens the original in a browser, where videos have playback,
seeking, and download controls. **Ctrl+P → Preview latest media** reopens the
focused agent's latest file. Background agents' previews stay available on the
phone and through the URL printed by the command.

Images use the terminal's graphics support (including Ghostty and Kitty), with a
colored text fallback on other terminals. PNG, JPEG, GIF, and WebP are supported;
animated images play in the browser, while the terminal shows a still frame.
MP4/MOV and WebM play in browsers that support the file's codec. Installing
`ffmpeg` enables video thumbnails; playback does not require it.

The phone conversation includes the same previews. Desktop browser previews work
over a private loopback server even without Tailscale. Only explicitly presented
copies are served; originals are never changed. Previews expire when their agent
is deleted or Workbench closes, and older previews are evicted at 16 per agent,
64 overall, or 512 MiB of media. Images are limited to 25 MiB / 32 megapixels;
videos to 256 MiB.

Embedded terminal links are clickable, including links whose visible label hides
the URL. Links survive scrolling and log-derived history. Ordinary HTTP(S) URLs
are clickable too. Dragging still selects text; a click opens the link with the
system browser or the local file's associated application.
Use a normal left-click, with no modifier key. Ghostty, Kitty, and Foot show a
hand pointer while hovering over a recognized link. The pointer resets when you
leave the link, open a dialog, or quit Workbench.

## Agent-to-agent communication

Agents running inside workbench can discover and talk to each other through
the `workbench` CLI (available on their PATH, with identity injected via
`$WORKBENCH_SESSION`). Workbench maintains a standing instructions block in
each workspace's `CLAUDE.local.md` / `AGENTS.md` (kept out of git via
`.git/info/exclude`), so every agent knows the protocol by default — you can
just tell Claude "ask codex what it thinks" or "read the other claude's
transcript and take over where it left off".

If a workspace already **tracks** `AGENTS.md` in git, workbench never writes
to it: that file is project-owned and committed, while this block describes
your local machine (a TUI, live peer sessions, `$WORKBENCH_SESSION`) — and
editing a tracked file would let any agent running `git add -A` commit those
machine instructions into the shared repo. The block goes to an untracked
`AGENTS.local.md` sidecar instead, so agents here still get the protocol.

```bash
workbench agents                       # roster: id, provider, alias, branch, idle/busy
workbench transcript <id|alias>        # a peer's recent conversation (exported at each idle)
workbench ask <id|alias> "question"    # queue a question for a live peer; prints a ticket
workbench handoff <id|alias> --wait    # structured take-over summary from a live peer
workbench replies <ticket> --wait      # collect the answer
workbench alias <name>                 # name this session for easy addressing
workbench wait <id|alias>              # block until a peer stops working
```

An agent or script can address a peer by short id, by alias, or by provider
name — the last resolves when only one such agent runs in *your* project,
and never resolves to the caller itself, so `wait codex` from a codex agent
means the other one. Anything still ambiguous is refused with the candidates
named rather than guessed at; `--project <name>` narrows explicitly from a
plain shell.

`wait` returns as soon as the agent stops working, which by default means
idle, blocked, *or* stopped — an agent parked on a permission prompt has
finished its turn as far as a script is concerned, and `--state idle` alone
would hang there until a human answered. It exits `3` on timeout, distinct
from `1`, so a script can tell "still working" from "no such agent".

## Control socket

Workbench listens on a Unix socket (`WORKBENCH_CONTROL_SOCK`, injected into
every agent pane) speaking newline-delimited JSON — a third way in, after the
TUI's keys and the phone. `wait` is built on it, and so can your own scripts,
editors, or agents.

Inside a pane the path is already in `$WORKBENCH_CONTROL_SOCK`. Elsewhere,
workbench logs it at startup (`control socket on …`) — it lives beside the
rest of workbench's state, or in the temp directory when that path would
overflow the ~104 bytes a Unix socket address can hold.

One JSON object per line, in and out. Ask it what it can do:

```jsonc
→ {"id":1,"method":"api.schema"}
← {"id":1,"result":{"methods":[…],"events":[…]}}

→ {"id":2,"method":"agents.list"}
← {"id":2,"result":[{"id":"a9b5f906","project":"workbench","provider":"Claude",
                    "alias":null,"model":"Opus 5","status":"working",…}]}

→ {"id":3,"method":"agent.prompt","params":{"agent":"a9b5f906","text":"ship it"}}
← {"id":3,"result":{"accepted":true}}

→ {"id":4,"method":"events.subscribe"}
← {"id":4,"result":{"subscribed":true}}
← {"event":"agent.status_changed","data":{"agent":"a9b5f906","from":"working","to":"idle"}}
```

Any client that speaks a Unix stream socket will do — `workbench wait` is the
one that ships.

Reads answer from the snapshot the event loop already publishes each tick, so
they never block the UI and are at most one tick old. Writes (`agent.prompt`,
`agent.todo`, `agent.answer`, `agent.focus`, `agent.new`) are queued for the
event loop and answer `{"accepted":true}` — the loop took it, not that the
agent has replied. Subscribers get `agent.added`, `agent.removed`,
`agent.status_changed` and `agent.model_changed` as they happen, which is why
`wait` costs nothing while it waits. The socket is `0600` and local only.

The instructions block also encodes what multi-agent research says works:
review a peer's *branch diff* with fresh eyes (never its self-report), use
`handoff` from the live author when taking over, prefer cross-provider
opinions, and push back with a better alternative when asked for known
anti-patterns (consensus debates, shared-branch edits).

Consults deliver only when the target is idle, appear visibly in its pane,
and are guarded against cycles (A→B while B→A) and unbounded fan-out (one
outstanding consult per asker). Transcripts and rosters live outside the
repo under the workbench config directory, so nothing pollutes git status.

Press `h` or `?` in the app for keybindings and settings.

For phone access, install and open [Tailscale](https://tailscale.com/download) on
the computer running Workbench and on your phone, then sign in to the same
tailnet on both. Restart Workbench and open **Utilities → Phone QR** to scan the
private link. Phone access is enabled by default; set `remote_port = 0` in
`user_config.toml` to turn it off.

From the workspace list, press `g` to open the selected repository map in your
browser. The map is served on loopback only, respects `.gitignore`, and shows
the full tree in compact folder clusters with search, pan/zoom, fit-to-view, a
minimap, optional folder collapsing, and automatic refresh. Click anywhere in
the minimap to center the canvas there, drag its viewport frame to navigate, or
focus it and use the arrow keys for keyboard panning.

Use **Analyze** and **Categorize** for the built-in repository jobs, or click
**Note** to place an independent agent note on the board. Each note starts a fresh,
read-only Claude Code instance using Claude Sonnet 5. Answers and follow-up turns
appear inside the note; note conversations live only in the open canvas and Claude
session persistence is disabled. Use **Select**, Shift-drag, or Cmd/Ctrl-click
before creating a note to bind it to specific files and folders.

The Categorize job creates a grounded Architecture Lens. Workbench lays out the
agent's categories and relationships as a compact full-canvas graph, and every
concept must reference real repository paths. Select a concept to reveal its files
or generate a deeper subsystem map, then use the back control to move through the
abstraction levels or return to the factual file tree. Generated maps and other AI
drawing layers remain read-only, bounded, validated, undoable, and removable.

## License

[MIT](LICENSE)
