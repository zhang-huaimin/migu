mod cli;
mod db;
mod shell;
mod tui;

use clap::Parser;
use crate::cli::{Cli, Commands};
use crate::tui::Action;
use std::io::BufRead;
use std::process::{self, Command};

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

    let path = db::db_path();
    let conn = match db::open(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("migu: failed to open database: {}", e);
            process::exit(1);
        }
    };

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

        let cmd = match shell {
            "bash" => parse_bash_line(&line),
            "zsh" => parse_zsh_line(&line),
            "fish" => parse_fish_line(&line),
            _ => None,
        };

        if let Some(cmd) = cmd {
            if let Err(e) = db::insert_command(&conn, &cmd, &host, shell, None, None, None) {
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

    eprintln!("migu: imported {} commands from {} history", count, shell);
}

/// Parse a line from bash history.
/// Lines starting with "#" followed by digits are HISTTIMEFORMAT timestamps.
fn parse_bash_line(line: &str) -> Option<String> {
    // Skip timestamp lines: "#1234567890"
    if line.starts_with('#') && line[1..].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(line.to_string())
}

/// Parse a line from zsh history.
/// Format: ": 1234567890:0;command"
fn parse_zsh_line(line: &str) -> Option<String> {
    // Zsh extended history format: ": <timestamp>:<duration>;<command>"
    if line.starts_with(':') {
        if let Some(pos) = line.rfind(';') {
            return Some(line[pos + 1..].to_string());
        }
    }
    Some(line.to_string())
}

/// Parse a line from fish history.
/// Fish uses YAML-like blocks: "- cmd: <command>" are the command lines.
/// Non-command lines ("  when: ...", etc.) are skipped.
fn parse_fish_line(line: &str) -> Option<String> {
    if let Some(cmd) = line.strip_prefix("- cmd: ") {
        return Some(cmd.to_string());
    }
    None
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

    let path = db::db_path();
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

    // Probabilistic purge
    let max_entries = std::env::var("MIGU_MAX_ENTRIES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100_000);
    let _ = db::maybe_purge(&conn, max_entries);
}

/// Launch the interactive TUI.
fn run_tui(cli: &Cli) {
    let path = cli.database.as_ref().map(|p| p.into()).unwrap_or_else(|| db::db_path());
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

    let dedup = !cli.no_dedup;

    match tui::run(&conn, &cwd, cli.limit as usize, dedup) {
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
            // Restore terminal already done in tui::run
            let status = Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .spawn()
                .and_then(|mut child| child.wait());
            match status {
                Ok(s) if !s.success() => {
                    std::process::exit(s.code().unwrap_or(1));
                }
                Err(e) => {
                    eprintln!("re: failed to execute command: {}", e);
                    process::exit(1);
                }
                _ => {}
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
