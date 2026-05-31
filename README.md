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
| **Enter** | Execute selected command |
| **Tab** | Insert command into prompt (editable) |
| `Alt + s` | Toggle Time / Frequency mode |
| `Alt + n` | Enter number-input mode, digits + Enter to jump |
| `Alt + l` | Change display limit |
| `Alt + h` | Show help |
| `←` `→` `Home` `End` | Move cursor in search bar |
| `Backspace` / `Delete` | Edit search text |
| Type any text | Fuzzy filter |
| `PgUp` / `PgDn` | Page scroll |
| `Esc` / `Ctrl-C` | Quit |

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

```bash
# Auto-record
_migu_prompt_command() {
    local cmd
    cmd="$(history 1 | sed 's/^ *[0-9][0-9]* *//')"
    migu add -- "$cmd"
}
PROMPT_COMMAND=_migu_prompt_command

# Ctrl-R binding
_migu_widget() {
    MIGU_WIDGET=1 command migu
    local cmd
    cmd="$(cat /tmp/migu-cmd 2>/dev/null)"
    rm -f /tmp/migu-cmd
    READLINE_LINE="${cmd:-$READLINE_LINE}"
    READLINE_POINT=${#READLINE_LINE}
}
bind -x '"\C-r": _migu_widget' 2>/dev/null
```

### Zsh

```zsh
autoload -Uz add-zsh-hook
_migu_add_hook() {
    [[ -n "$_migu_skip" ]] && return
    _migu_skip=1
    migu add -- "$1"
    unset _migu_skip
}
add-zsh-hook preexec _migu_add_hook

_migu_widget() {
    MIGU_WIDGET=1 command migu
    local cmd="$(cat /tmp/migu-cmd 2>/dev/null)"
    rm -f /tmp/migu-cmd
    zle reset-prompt
    LBUFFER+="$cmd"
}
zle -N _migu_widget
bindkey '^R' _migu_widget
```

### Fish

```fish
function _migu_add --on-event fish_preexec
    if set -q _migu_skip
        return
    end
    set -g _migu_skip 1
    migu add -- "$argv"
    set -e _migu_skip
end

function _migu_widget
    MIGU_WIDGET=1 command migu
    set -l cmd (cat /tmp/migu-cmd 2>/dev/null)
    rm -f /tmp/migu-cmd
    commandline -r -- $cmd
end
bind \cr _migu_widget
```

## Configuration

Optional config file at `~/.migu/config.toml`:

```toml
[database]
# Custom database path (default: ~/.migu/history.db)
path = "/mnt/data/migu/history.db"

[keys]
# Modifier key: "Alt" (default), "Ctrl", "Ctrl+Shift", or "" for none
modifier = "Alt"

# Character keys for each action (must be a single character)
toggle_sort = "s"       # default: Alt+s
toggle_numbers = "n"    # default: Alt+n
toggle_help = "h"       # default: Alt+h
set_limit = "l"         # default: Alt+l
```

## Database

Location: `~/.migu/history.db` (SQLite, WAL mode). Directory permissions: `700`, file permissions: `600`.

## Build

```bash
cargo build --release
```

Requirements: Rust 1.70+

Dependencies: `rusqlite`, `clap`, `ratatui`, `crossterm`, `chrono`, `dirs`, `libc`, `rand`, `shell-words`, `whoami`, `toml`, `serde`
