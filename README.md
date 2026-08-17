# Workbench

[![CI](https://github.com/steferic/workbench/actions/workflows/ci.yml/badge.svg)](https://github.com/steferic/workbench/actions/workflows/ci.yml)

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
