# AGENTS.md — migu project memory

## Project Identity

**migu** (迷榖) is a cross-shell command history manager for bash/zsh/fish, written in Rust. Named after a mythical tree from *Classic of Mountains and Seas* said to prevent disorientation.

- **Binary**: `migu`
- **Repo**: `github.com/zhang-huaimin/migu`
- **Language**: Rust (MSRV 1.70)
- **Database**: SQLite (WAL mode), `~/.migu/history.db`

## Architecture

```
┌─────────────────────────────────────────────────┐
│ Shell (bash/zsh/fish)                           │
│  ┌──────────┐   Ctrl-R    ┌──────────────────┐  │
│  │ PROMPT_  │◄───────────│ Widget function   │  │
│  │ COMMAND  │             │ _migu_widget()    │  │
│  │ hook     │             └───────┬──────────┘  │
│  └────┬─────┘                     │ MIGU_WIDGET=1
│       │ migu add                  │ command migu
└───────┼───────────────────────────┼──────────────┘
        │                           │
        ▼                           ▼
   ┌──────────────────────────────────────┐
   │            migu binary               │
   │  src/main.rs  src/tui.rs  src/db.rs  │
   └──────────────────────────────────────┘
                      │
                      ▼
              ~/.migu/history.db  (SQLite WAL)
```

## Shell Integration — The Most Critical Part

migu **never spawns commands directly**. Instead, it communicates back to the shell via temp files (`/tmp/migu-cmd`, `/tmp/migu-exec`). The shell widget handles execution.

### Action flow

| User action | migu writes | Shell widget does |
|---|---|---|
| **Enter** | `/tmp/migu-cmd` + `/tmp/migu-exec` | Insert + auto-execute |
| **Tab** | `/tmp/migu-cmd` only | Insert only (editable) |
| **Esc/Ctrl-C** | nothing | nothing |

### Execution mechanisms per shell

| Shell | Mechanism |
|---|---|
| **bash** | Two-keystroke readline macro `\C-x1\C-x2`. `\C-x1` → widget, `\C-x2` dynamically bound to `accept-line` for Enter, `""` for Tab/cancel. `bind -m emacs` and `-m vi-insert` required. |
| **zsh** | `zle accept-line` called after `LBUFFER+="$cmd"` |
| **fish** | `commandline -f execute` called after `commandline -r -- $cmd` |

### Why we don't use `Command::spawn()`

Spawning from migu's process causes terminal state issues (raw mode, alternate screen) that break interactive commands like `ssh`. All approaches that failed:
- `Command::spawn()` — terminal raw mode inherited by child
- `eval "$cmd"` in bash widget — runs in readline's raw mode
- `TIOCSTI` ioctl — deprecated, visual glitches, keyboard echo issues

### Temp file protocol

- `MIGU_WIDGET=1` env var signals migu to write temp files instead of stdout
- `/tmp/migu-cmd`: command text
- `/tmp/migu-exec`: empty file, presence = execute mode
- Widget removes files after reading

## Source Map

| File | Lines | Purpose |
|---|---|---|
| `src/main.rs` | ~320 | Entry point, CLI dispatch, action handlers, import logic |
| `src/tui.rs` | ~770 | ratatui TUI: App struct, rendering, key dispatch, fuzzy search UI |
| `src/db.rs` | ~400 | SQLite: schema, insert/query, fuzzy match, probabilistic purge |
| `src/shell.rs` | ~120 | Shell init script strings (bash/zsh/fish) |
| `src/config.rs` | ~120 | TOML config loading, key binding resolution |
| `src/cli.rs` | ~70 | clap CLI argument definitions |

## Configuration

Config file: `~/.migu/config.toml`

Priority for all settings: **env var > config file > default**

### Key config fields

```toml
[keys]
leader = "Alt"
toggle_sort = "${leader} + s"
toggle_numbers = "${leader} + n"
toggle_help = "${leader} + h"
set_limit = "${leader} + l"

[database]
path = "/custom/path/history.db"    # default: ~/.migu/history.db
max_entries = 500000               # default: no limit; env: MIGU_MAX_ENTRIES
```

### When adding new config
1. Add field to struct in `src/config.rs`
2. Add `#[serde(default)]` for optional fields
3. Read in the appropriate function (config loaded via `config::load()`)
4. Check env var first, then config value
5. Update README.md configuration section

## Key Behavior Notes

### Enter vs Tab in TUI
- `Enter` → `select_execute()` → `Action::Execute` → writes both temp files → shell auto-executes
- `Tab` → `select_insert()` → `Action::Insert` → writes only `/tmp/migu-cmd` → shell inserts only
- Both are defined in `src/tui.rs` `App` impl

### History recording
- Shell `PROMPT_COMMAND`/`preexec` hooks call `migu add -- <cmd>`
- `Action::Execute` handler also records in DB optimistically
- Dedup handled by `query_collapsed()` in frequency mode

### Database purge
- Probabilistic: ~1% chance per insert (every ~100 commands)
- Keeps most recent `max_entries` rows
- Default: unlimited. Set via `MIGU_MAX_ENTRIES` or `database.max_entries`

## When Changing Shell Init Scripts

The init scripts in `src/shell.rs` are embedded as Rust string constants (`BASH_INIT`, `ZSH_INIT`, `FISH_INIT`). When modifying:

1. **Test with all three shells** — bash/zsh/fish each have different mechanisms
2. **Bash two-keystroke macro** is the most fragile — test Enter, Tab, and cancel (Esc) paths
3. **Both emacs and vi-insert** keymaps must be covered for bash
4. **After changes**, users must re-run `eval "$(migu init <shell>)"`
5. **Update README shell integration section** if the widget logic changed

## Development

```bash
cargo build                    # debug build
cargo build --release          # release build
cargo test                     # 3 tests (db fuzzy match, recent, frequent)
cargo clippy                   # lint
cargo run -- init bash         # print bash init script
cargo publish --dry-run --locked --registry crates-io  # check publish readiness
```

### Version bumps
- Update `Cargo.toml` version
- `cargo update` to refresh `Cargo.lock`
- `git tag vX.Y.Z` and push with `--tags`
- Commit message: `chore: bump version to X.Y.Z`
