# Workbench

A TUI (Terminal User Interface) for managing AI agent workspaces and sessions.

## Features

- Manage multiple workspaces and sessions
- PTY support for running terminal sessions
- ANSI color rendering
- Audio playback support
- Clipboard integration

## Prerequisites

### Required
- [Rust](https://www.rust-lang.org/tools/install) (1.70 or later)

### AI Agents (install the ones you want to use)
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) - `claude` CLI for Anthropic's Claude
- [Gemini CLI](https://github.com/google-gemini/gemini-cli) - `gemini` CLI for Google's Gemini
- [Codex CLI](https://github.com/openai/codex) - `codex` CLI for OpenAI's Codex
- [Grok CLI](https://github.com/xai-org/grok) - `grok` CLI for xAI's Grok

### Optional Dependencies (for full feature support)

#### Audio/Sounds
- **VLC** - Required for classical radio streaming
  ```bash
  # macOS
  brew install vlc

  # Ubuntu/Debian
  sudo apt install vlc
  ```

- **FFmpeg** - Required for ambient sounds (ocean, chimes, rain, brown noise)
  ```bash
  # macOS
  brew install ffmpeg

  # Ubuntu/Debian
  sudo apt install ffmpeg
  ```

## Installation

### From source

```bash
git clone https://github.com/stefanlenoach/workbench.git
cd workbench
cargo build --release
```

The binary will be at `target/release/workbench`.

### Install globally

```bash
cargo install --path .
```

## Usage

### Run the TUI

```bash
workbench
```

### Start with a specific workspace

```bash
workbench --workspace /path/to/workspace
```

### Add a workspace

```bash
workbench add /path/to/workspace
workbench add /path/to/workspace --name "My Workspace"
```

### List workspaces

```bash
workbench list
```

## License

MIT
