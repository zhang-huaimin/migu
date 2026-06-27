mod cli;
mod config;
mod db;
mod shell;
mod tui;

use clap::Parser;
use crate::cli::{Cli, Commands};
use crate::db::query_collapsed;
use crate::tui::{Action, first_line, relative_time_compact};
use std::io::BufRead;
use std::process;

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Some(Commands::Add {
            command,
            cwd,
            exit_code,
            hostname,
            shell,
            session_id,
        }) => {
            run_add(command, cwd, *exit_code, hostname.as_deref(), shell.as_deref(), session_id.as_deref());
        }
        Some(Commands::Init { shell }) => {
            let script = shell::init_script(shell);
            println!("{}", script);
        }
        Some(Commands::Import { shell }) => {
            run_import(shell);
        }
        Some(Commands::List { frequency, expand, limit }) => {
            run_list(&cli, *frequency, *expand, limit.unwrap_or(cli.limit as usize));
        }
        None => {
            // Default: launch TUI
            run_tui(&cli);
        }
    }
}

/// Import existing history from a shell's native history file.
fn run_import(shell: &str) {
    let history_file = match shell {
        "bash" => dirs::home_dir().map(|h| h.join(".bash_history")),
        "zsh" => dirs::home_dir().map(|h| h.join(".zsh_history")),
        "fish" => dirs::data_local_dir().map(|d| d.join("fish").join("fish_history")),
        _ => {
            eprintln!("migu: unsupported shell for import: {}", shell);
            process::exit(1);
        }
    };

    let history_file = match history_file {
        Some(f) if f.exists() => f,
        _ => {
            eprintln!("migu: history file not found for {}", shell);
            process::exit(1);
        }
    };

    let cfg = config::load();
    let path = cfg
        .database
        .path
        .as_ref()
        .map(|p| p.into())
        .unwrap_or_else(db::db_path);
    let conn = match db::open(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("migu: failed to open database: {}", e);
            process::exit(1);
        }
    };

    // Skip if already imported
    if db::is_imported(&conn, shell) {
        return;
    }

    let host = whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string());
    let file = match std::fs::File::open(&history_file) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("migu: failed to open history file: {}", e);
            process::exit(1);
        }
    };

    let reader = std::io::BufReader::new(file);
    let mut count = 0u64;
    let mut pending_ts: Option<i64> = None;

    // Use a transaction for bulk import performance
    if let Err(e) = conn.execute("BEGIN", []) {
        eprintln!("migu: failed to begin transaction: {}", e);
        process::exit(1);
    }

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let result = match shell {
            "bash" => parse_bash_line(&line, &mut pending_ts),
            "zsh" => parse_zsh_line(&line),
            "fish" => parse_fish_line(&line),
            _ => None,
        };

        if let Some((cmd, ts)) = result {
            let created_at = ts.map(unix_to_iso8601);
            if let Err(e) = db::insert_imported_command(
                &conn, &cmd, &host, shell, created_at.as_deref(),
            ) {
                eprintln!("migu: failed to insert command: {}", e);
            } else {
                count += 1;
            }
        }
    }

    if let Err(e) = conn.execute("COMMIT", []) {
        eprintln!("migu: failed to commit: {}", e);
        process::exit(1);
    }

    if let Err(e) = db::mark_imported(&conn, shell) {
        eprintln!("migu: failed to mark import: {}", e);
    }

    eprintln!("migu: imported {} commands from {} history", count, shell);
}

/// Convert a Unix epoch timestamp to ISO 8601 string (UTC).
fn unix_to_iso8601(epoch: i64) -> String {
    chrono::DateTime::from_timestamp(epoch, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S").to_string())
        .unwrap_or_else(|| "1970-01-01T00:00:00".to_string())
}

/// Parse a line from bash history.
/// Lines starting with "#" followed by digits are HISTTIMEFORMAT timestamps
/// stored in pending_ts for the next command line.
/// Returns (command, optional_unix_timestamp).
fn parse_bash_line(line: &str, pending_ts: &mut Option<i64>) -> Option<(String, Option<i64>)> {
    if line.starts_with('#') && line[1..].chars().all(|c| c.is_ascii_digit()) {
        if let Ok(ts) = line[1..].parse::<i64>() {
            *pending_ts = Some(ts);
        }
        return None;
    }
    let ts = pending_ts.take();
    Some((line.to_string(), ts))
}

/// Parse a line from zsh history.
/// Format: ": 1234567890:0;command"
/// Returns (command, optional_unix_timestamp).
fn parse_zsh_line(line: &str) -> Option<(String, Option<i64>)> {
    if line.starts_with(':') {
        if let Some(rest) = line.strip_prefix(':') {
            let rest = rest.trim_start();
            if let Some(colon_pos) = rest.find(':') {
                let ts = rest[..colon_pos].parse::<i64>().ok();
                let after_colon = &rest[colon_pos + 1..];
                if let Some(semi_pos) = after_colon.find(';') {
                    return Some((after_colon[semi_pos + 1..].to_string(), ts));
                }
            }
        }
    }
    Some((line.to_string(), None))
}

/// Parse a line from fish history.
/// Fish uses YAML-like blocks: "- cmd: <command>" are the command lines.
/// Returns (command, optional_unix_timestamp).
fn parse_fish_line(line: &str) -> Option<(String, Option<i64>)> {
    if let Some(cmd) = line.strip_prefix("- cmd: ") {
        return Some((cmd.to_string(), None));
    }
    None
}

/// Detect the current shell from the SHELL environment variable.
fn detect_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .and_then(|s| {
            std::path::Path::new(&s)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// Handle the `migu list` subcommand: print history to stdout.
fn run_list(cli: &Cli, by_freq: bool, expand: bool, limit: usize) {
    let cfg = config::load();
    let path = cli
        .database
        .as_ref()
        .map(|p| p.into())
        .or_else(|| cfg.database.path.as_ref().map(|p| p.into()))
        .unwrap_or_else(db::db_path);
    let conn = match db::open(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("migu: failed to open database: {}", e);
            process::exit(1);
        }
    };

    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_default();

    let entries = match query_collapsed(&conn, "", &cwd, limit, by_freq) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("migu: query error: {}", e);
            process::exit(1);
        }
    };

    if entries.is_empty() {
        return;
    }

    // Build display columns and compute max widths
    struct Row {
        num: String,
        time: String,
        freq: String,
        cmd: String,
    }

    let mut rows: Vec<Row> = Vec::with_capacity(entries.len());

    let idx_w = if entries.len() >= 100 { 3 } else if entries.len() >= 10 { 2 } else { 1 };

    for (i, entry) in entries.iter().enumerate() {
        let num = format!("{:>width$}", i + 1, width = idx_w);
        let time = entry
            .created_at
            .as_deref()
            .map(|t| relative_time_compact(t))
            .unwrap_or_default();
        let freq = if entry.freq > 1 {
            format!("x{}", entry.freq)
        } else {
            String::new()
        };
        let cmd = if expand {
            crate::tui::strip_ansi(&entry.command)
        } else {
            first_line(&entry.command)
        };

        rows.push(Row { num, time, freq, cmd });
    }

    let num_w = rows.iter().map(|r| r.num.len()).max().unwrap_or(2).max(3);
    let time_w = rows.iter().map(|r| r.time.len()).max().unwrap_or(0).max(4);
    let freq_w = rows.iter().map(|r| r.freq.len()).max().unwrap_or(0).max(4);

    // Header
    println!(
        "{:<num_w$}  {:<time_w$}  {:<freq_w$}  COMMAND",
        "NO.",
        "TIME",
        "FREQ",
    );

    for row in &rows {
        println!(
            "{:<num_w$}  {:<time_w$}  {:<freq_w$}  {}",
            row.num,
            row.time,
            row.freq,
            row.cmd,
        );
    }
}

/// Handle the `re add` subcommand.
fn run_add(
    command: &[String],
    cwd: &str,
    exit_code: Option<i32>,
    hostname: Option<&str>,
    shell: Option<&str>,
    session_id: Option<&str>,
) {
    // Skip empty commands
    let cmd_str = command.join(" ").trim().to_string();
    if cmd_str.is_empty() {
        return;
    }

    let cfg = config::load();
    let path = cfg.database.path.as_deref().map(|p| p.into()).unwrap_or_else(db::db_path);
    let conn = match db::open(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("re: failed to open database: {}", e);
            process::exit(1);
        }
    };

    let mut fallback_host = String::new();
    let host = hostname.unwrap_or_else(|| {
        fallback_host = whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string());
        &fallback_host
    });
    let sh = shell.unwrap_or("bash");
    let cwd_opt = if cwd.is_empty() { None } else { Some(cwd) };

    if let Err(e) = db::insert_command(&conn, &cmd_str, host, sh, cwd_opt, exit_code, session_id)
    {
        eprintln!("re: failed to insert command: {}", e);
    }

    // Probabilistic purge: env var takes precedence over config; no limit by default
    let max_entries = std::env::var("MIGU_MAX_ENTRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .or(cfg.database.max_entries);
    if let Some(max) = max_entries {
        let _ = db::maybe_purge(&conn, max);
    }
}

/// Launch the interactive TUI.
fn run_tui(cli: &Cli) {
    let cfg = config::load();

    let path = cli
        .database
        .as_ref()
        .map(|p| p.into())
        .or_else(|| cfg.database.path.as_ref().map(|p| p.into()))
        .unwrap_or_else(db::db_path);
    let conn = match db::open(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("re: failed to open database: {}", e);
            process::exit(1);
        }
    };

    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_default();

    let ks = &cfg.keys;
    let leader = &ks.leader;
    let keys = config::ResolvedKeys {
        toggle_sort: config::resolve_binding(&ks.toggle_sort, leader),
        toggle_numbers: config::resolve_binding(&ks.toggle_numbers, leader),
        toggle_help: config::resolve_binding(&ks.toggle_help, leader),
        set_limit: config::resolve_binding(&ks.set_limit, leader),
    };

    match tui::run(&conn, &cwd, cli.limit as usize, &keys) {
        Ok(Action::Insert(cmd)) => {
            if std::env::var("MIGU_WIDGET").is_ok() {
                // Widget mode: write to temp file
                let _ = std::fs::write("/tmp/migu-cmd", &cmd);
            } else {
                // Direct mode: print to stdout
                println!("{}", cmd);
            }
        }
        Ok(Action::Execute(cmd)) => {
            // Widget mode: write both signal files
            if std::env::var("MIGU_WIDGET").is_ok() {
                let _ = std::fs::write("/tmp/migu-cmd", &cmd);
                let _ = std::fs::write("/tmp/migu-exec", "");
            } else {
                // Direct mode: print to stdout
                println!("{}", cmd);
            }

            // Record the command in the database
            if let Ok(conn) = db::open(&path) {
                let host = whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_string());
                let sh = detect_shell();
                let _ = db::insert_command(&conn, &cmd, &host, &sh, Some(&cwd), None, None);
            }
        }
        Ok(Action::Quit) => {
            // User quit without selecting
        }
        Err(e) => {
            eprintln!("re: TUI error: {}", e);
            process::exit(1);
        }
    }
}
