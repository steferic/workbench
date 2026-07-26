# Workbench

[![CI](https://github.com/steferic/workbench/actions/workflows/ci.yml/badge.svg)](https://github.com/steferic/workbench/actions/workflows/ci.yml)

A TUI for managing AI agent workspaces and sessions. Run Claude, Codex, Gemini, and other coding agents side by side across multiple projects, with per-workspace sessions, pinned terminals, a live view of the selected agent's task list, and git worktree isolation.

## Features

- Multiple workspaces, each with its own agents and terminals
- Tasks pane: the selected agent's own task list, the prompt behind it, and live progress — with keys to ask that agent to add, change, or drop a task (Claude, Codex, opencode, hermes)
- Restart restores each agent's *own* conversation, so several agents in one project keep separate histories
- Run agents in isolated git worktrees and merge their work back with one key
- Parallel tasks: race several agents on the same prompt in separate worktrees
- Pinned terminal panes alongside the agent output
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
```

## Agent-to-agent communication

Agents running inside workbench can discover and talk to each other through
the `workbench` CLI (available on their PATH, with identity injected via
`$WORKBENCH_SESSION`). Workbench maintains a standing instructions block in
each workspace's `CLAUDE.local.md` / `AGENTS.md` (kept out of git via
`.git/info/exclude`), so every agent knows the protocol by default — you can
just tell Claude "ask codex what it thinks" or "read the other claude's
transcript and take over where it left off".

```bash
workbench agents                       # roster: id, provider, alias, branch, idle/busy
workbench transcript <id|alias>        # a peer's recent conversation (exported at each idle)
workbench ask <id|alias> "question"    # queue a question for a live peer; prints a ticket
workbench handoff <id|alias> --wait    # structured take-over summary from a live peer
workbench replies <ticket> --wait      # collect the answer
workbench alias <name>                 # name this session for easy addressing
```

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

## License

[MIT](LICENSE)
