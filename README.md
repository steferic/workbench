# Workbench

A TUI (Terminal User Interface) for managing AI agent workspaces and sessions.

## Features

- Manage multiple workspaces and sessions
- PTY support for running terminal sessions
- ANSI color rendering
- Audio playback support
- Clipboard integration

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (1.70 or later)

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
