# migu — Cross-Shell Command History Manager

命名自《山海经》迷榖："有木焉，其状如谷而黑理，其华四照，其名曰迷穀，佩之不迷。"。**migu** records your shell commands into a local SQLite database and provides an interactive TUI browser for fast recall.

Supports **bash**, **zsh**, **fish**.

## Features

- **Persistent history** — all commands stored in SQLite
- **Interactive TUI** — fuzzy search, keyboard navigation, two sort modes
- **Time mode** — most recent commands (consecutive duplicates folded)
- **Frequency mode** — most used commands (grouped by command + directory)
- **cwd-aware** — commands from current directory shown first
- **Instant record** — WAL-mode SQLite, sub-millisecond writes

## Quick Start

### Install

```bash
cargo install --path .
```

### Setup (bash)

```bash
eval "$(migu init bash)"
```

This sets up:
- `PROMPT_COMMAND` to auto-record every command
- **Ctrl-R** keybinding for the TUI browser

Restart your shell or add to `~/.bashrc`:

```bash
# ~/.bashrc
eval "$(migu init bash)"
```

## Usage

### Browse history

Press **Ctrl-R** to open the TUI browser:

```
> cvbnm                                        │ 跳转：15_
 1/30 ────────────────────────────────────────────────────
   1. git status                  ~/re      （2分钟前）
   2. cargo build --release       ~/re      （5分钟前）
   3. ssh prod-server             ~         （12分钟前）
  30. cat /var/log/syslog         /var      （1天前）
```

### Keys

| Key | Action |
|-----|--------|
| `↑` `↓` | Navigate |
| **Enter** | Execute selected command (auto-runs in your shell) |
| **Tab** | Insert command into prompt for editing |
| `Alt + s` | Toggle Time / Frequency mode |
| `Alt + n` | Enter number-input mode, digits + Enter to jump |
| `Alt + l` | Change display limit |
| `Alt + h` | Show help |
| `←` `→` `Home` `End` | Move cursor in search bar |
| `Backspace` / `Delete` | Edit search text |
| Type any text | Fuzzy filter |
| `PgUp` / `PgDn` | Page scroll |
| `Esc` / `Ctrl-C` | Quit |
| `Alt + z` | Toggle expand mode: show full command below the list |

> Tip: Key bindings are configurable via `~/.migu/config.toml`.

### Direct invocation

```bash
migu              # open TUI with default 30 entries
migu -n 20        # show 20 entries
migu --no-dedup   # don't fold consecutive duplicates
```

## Commands

| Command | Description |
|---------|-------------|
| `migu` | Launch TUI browser |
| `migu add -- <command>` | Record a command (called by shell hooks) |
| `migu init <shell>` | Output shell configuration (bash / zsh / fish) |
| `migu import <shell>` | Import existing shell history into database |
| `migu -n <N>` | Set max results (default 30, max 100) |
| `migu --no-dedup` | Show all entries without dedup |

## Shell Integration

### Bash

Bash uses a two-keystroke macro (like mcfly) for auto-execute: `Ctrl-R` → `\C-x1\C-x2`. The widget binds `\C-x2` to `accept-line` for Enter, or nothing for Tab.

### Zsh

Uses `zle accept-line` for auto-execute.

### Fish

Uses `commandline -f execute` for auto-execute.

## Configuration

Optional config file at `~/.migu/config.toml`:

```toml
[keys]
# Global leader key, referenced as ${leader} in bindings below.
# Default: "Alt". Supports: Alt, Ctrl, Shift, Ctrl+Shift, or "" for none.
leader = "Alt"

# Each binding can use ${leader} or specify its own combination.
# Format: "Modifier + key" — last part is always the character key.
toggle_sort = "${leader} + s"       # default: Alt + s
toggle_numbers = "${leader} + n"    # default: Alt + n
toggle_help = "${leader} + h"       # default: Alt + h
set_limit = "${leader} + l"         # inherit leader
# set_limit = "Alt + l"              # or override per binding

[database]
# Custom database path (default: ~/.migu/history.db)
path = "/mnt/data/migu/history.db"
# Max entries to keep; older ones are purged probabilistically (default: no limit).
# Env var MIGU_MAX_ENTRIES takes precedence over this setting.
max_entries = 500000
```

## Database

Location: `~/.migu/history.db` (SQLite, WAL mode). Directory permissions: `700`, file permissions: `600`.

## Build

```bash
cargo build --release
```

Requirements: Rust 1.70+

Dependencies: `rusqlite`, `clap`, `ratatui`, `crossterm`, `chrono`, `dirs`, `libc`, `rand`, `shell-words`, `whoami`, `toml`, `serde`
