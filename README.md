# Workbench

[![CI](https://github.com/steferic/workbench/actions/workflows/ci.yml/badge.svg)](https://github.com/steferic/workbench/actions/workflows/ci.yml)

A TUI for managing AI agent workspaces and sessions. Run Claude, Codex, Gemini, and other coding agents side by side across multiple projects, with per-workspace sessions, pinned terminals, todos, and git worktree isolation.

## Features

- Multiple workspaces, each with its own agents, terminals, and todos
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

Requires a recent stable [Rust](https://www.rust-lang.org/tools/install) toolchain.

```bash
# Linux only: audio build deps
sudo apt install pkg-config libasound2-dev     # Debian/Ubuntu
# sudo dnf install pkgconf-pkg-config alsa-lib-devel   # Fedora

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

Press `h` or `?` in the app for keybindings and settings.

## License

[MIT](LICENSE)
