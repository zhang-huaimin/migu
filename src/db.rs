use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::PathBuf;

/// Get the database path: ~/.migu/history.db
pub fn db_path() -> PathBuf {
    let base = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join(".migu");
    std::fs::create_dir_all(&dir).ok();
    // Set directory permissions to 700 (owner only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    dir.join("history.db")
}

/// Open (or create) the database with WAL mode enabled.
pub fn open(path: &PathBuf) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    // Set file permissions to 600 (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    init_schema(&conn)?;
    Ok(conn)
}

/// Create tables and indexes if they don't exist.
fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS commands (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            command    TEXT    NOT NULL,
            hostname   TEXT    NOT NULL,
            shell      TEXT    NOT NULL,
            cwd        TEXT,
            exit_code  INTEGER,
            created_at TEXT    DEFAULT (strftime('%Y-%m-%dT%H:%M:%S', 'now')),
            session_id TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_cmd_command    ON commands(command);
        CREATE INDEX IF NOT EXISTS idx_cmd_created_at ON commands(created_at);
        CREATE INDEX IF NOT EXISTS idx_cmd_cwd        ON commands(cwd);

        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );"
    )?;
    Ok(())
}

/// Insert a command into the database. Called by `re add`.
pub fn insert_command(
    conn: &Connection,
    command: &str,
    hostname: &str,
    shell: &str,
    cwd: Option<&str>,
    exit_code: Option<i32>,
    session_id: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO commands (command, hostname, shell, cwd, exit_code, session_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![command, hostname, shell, cwd, exit_code, session_id],
    )?;
    Ok(())
}

/// Insert a command imported from shell history, with optional timestamp.
pub fn insert_imported_command(
    conn: &Connection,
    command: &str,
    hostname: &str,
    shell: &str,
    created_at: Option<&str>,
) -> rusqlite::Result<()> {
    if let Some(ts) = created_at {
        conn.execute(
            "INSERT INTO commands (command, hostname, shell, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![command, hostname, shell, ts],
        )?;
    } else {
        conn.execute(
            "INSERT INTO commands (command, hostname, shell, created_at)
             VALUES (?1, ?2, ?3, NULL)",
            params![command, hostname, shell],
        )?;
    }
    Ok(())
}

/// Check whether a shell's history has already been imported.
pub fn is_imported(conn: &Connection, shell: &str) -> bool {
    conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        params![format!("imported_{}", shell)],
        |_| Ok(()),
    )
    .is_ok()
}

/// Mark a shell's history as imported.
pub fn mark_imported(conn: &Connection, shell: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, '1')",
        params![format!("imported_{}", shell)],
    )?;
    Ok(())
}

/// A single history entry returned from queries.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    #[allow(dead_code)]
    pub id: i64,
    pub command: String,
    pub cwd: Option<String>,
    pub created_at: Option<String>,
    pub freq: i64,
}

/// Query recent commands, with optional keyword filter and cwd prioritization.
#[allow(dead_code)]
pub fn query_recent(
    conn: &Connection,
    keyword: &str,
    current_cwd: &str,
    limit: usize,
) -> rusqlite::Result<Vec<HistoryEntry>> {
    // Load extra rows for fuzzy filtering
    let query_limit = if keyword.is_empty() { limit } else { limit * 3 };

    let sql = "SELECT id, command, cwd, created_at,
                COUNT(*) OVER (PARTITION BY command, cwd) AS freq,
                CASE WHEN cwd = ?1 THEN 0 ELSE 1 END AS priority
         FROM commands
         ORDER BY priority ASC, created_at IS NULL ASC, created_at DESC
         LIMIT ?2";

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![current_cwd, query_limit as i64], row_to_entry)?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row?);
    }

    // Apply fuzzy filter if keyword is non-empty
    if !keyword.is_empty() {
        entries.retain(|e| fuzzy_match(keyword, &e.command));
        entries.truncate(limit);
    }

    Ok(entries)
}

/// Query frequent commands (deduplicated by command + cwd), with optional keyword filter.
#[allow(dead_code)]
pub fn query_frequent(
    conn: &Connection,
    keyword: &str,
    current_cwd: &str,
    limit: usize,
) -> rusqlite::Result<Vec<HistoryEntry>> {
    let query_limit = if keyword.is_empty() { limit } else { limit * 5 };

    let sql = "SELECT MIN(id) AS id,
                command,
                cwd,
                MAX(created_at) AS created_at,
                COUNT(*) AS freq,
                CASE WHEN cwd = ?1 THEN 0 ELSE 1 END AS priority
         FROM commands
         GROUP BY command, cwd
         ORDER BY priority ASC, freq DESC, created_at IS NULL ASC, created_at DESC
         LIMIT ?2";

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![current_cwd, query_limit as i64], row_to_entry)?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row?);
    }

    if !keyword.is_empty() {
        entries.retain(|e| fuzzy_match(keyword, &e.command));
        entries.truncate(limit);
    }

    Ok(entries)
}

/// Query commands collapsed across all directories (same command = one entry).
/// If the command was run in current_cwd, use that cwd's time and frequency.
/// Otherwise use the first matched cwd's time, and total frequency across all dirs.
pub fn query_collapsed(
    conn: &Connection,
    keyword: &str,
    current_cwd: &str,
    limit: usize,
    by_frequency: bool,
) -> rusqlite::Result<Vec<HistoryEntry>> {
    // Fetch all rows grouped by (command, cwd) and aggregate in Rust
    let sql = "SELECT command, cwd, MAX(created_at) AS created_at, COUNT(*) AS freq
         FROM commands
         GROUP BY command, cwd
         ORDER BY command";

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;

    let mut map: HashMap<String, HistoryEntry> = HashMap::new();
    for row in rows {
        let (cmd, cwd, created_at, freq) = row?;
        let entry = map.entry(cmd.clone()).or_insert_with(|| HistoryEntry {
            id: 0,
            command: cmd.clone(),
            cwd: None,
            created_at: None,
            freq: 0,
        });
        entry.freq += freq;
        // Prefer current directory's cwd and time if available
        if cwd.as_deref() == Some(current_cwd) {
            entry.cwd = cwd;
            entry.created_at = created_at;
        } else if entry.cwd.is_none() {
            // First non-current match: use its cwd and time
            entry.cwd = cwd;
            entry.created_at = created_at;
        }
    }

    let mut entries: Vec<HistoryEntry> = map.into_values().collect();

    // Sort: current cwd first, then by freq or time depending on mode
    entries.sort_by(|a, b| {
        let a_prio = if a.cwd.as_deref() == Some(current_cwd) { 0 } else { 1 };
        let b_prio = if b.cwd.as_deref() == Some(current_cwd) { 0 } else { 1 };
        let ord = a_prio.cmp(&b_prio);
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
        if by_frequency {
            b.freq.cmp(&a.freq)
                .then(b.created_at.cmp(&a.created_at))
        } else {
            b.created_at.cmp(&a.created_at)
                .then(b.freq.cmp(&a.freq))
        }
    });

    entries.truncate(limit);

    if !keyword.is_empty() {
        entries.retain(|e| fuzzy_match(keyword, &e.command));
        entries.truncate(limit);
    }

    Ok(entries)
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<HistoryEntry> {
    Ok(HistoryEntry {
        id: row.get(0)?,
        command: row.get(1)?,
        cwd: row.get(2)?,
        created_at: row.get(3)?,
        freq: row.get(4)?,
    })
}

/// Fuzzy match: check if all chars in `pattern` appear in order in `text` (case-insensitive).
fn fuzzy_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let text = text.to_lowercase();
    let mut chars = pattern.chars().peekable();
    for c in text.chars() {
        if let Some(&pc) = chars.peek() {
            if c == pc {
                chars.next();
            }
        }
    }
    chars.peek().is_none()
}

/// Probabilistic purge: keep only the most recent max_entries commands.
pub fn maybe_purge(conn: &Connection, max_entries: i64) -> rusqlite::Result<()> {
    // Run purge roughly every 100 inserts (probabilistic)
    if rand::random::<u32>() % 100 != 0 {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM commands
         WHERE id NOT IN (
             SELECT id FROM commands
             ORDER BY created_at DESC
             LIMIT ?1
         )",
        params![max_entries],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS commands (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                command    TEXT    NOT NULL,
                hostname   TEXT    NOT NULL,
                shell      TEXT    NOT NULL,
                cwd        TEXT,
                exit_code  INTEGER,
                created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%S', 'now')),
                session_id TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_cmd_command    ON commands(command);
            CREATE INDEX IF NOT EXISTS idx_cmd_created_at ON commands(created_at);
            CREATE INDEX IF NOT EXISTS idx_cmd_cwd        ON commands(cwd);"
        ).unwrap();
        conn
    }

    #[test]
    fn test_query_frequent() {
        let conn = setup_db();
        // Insert commands
        insert_command(&conn, "ls", "test", "bash", Some("/home/a"), None, None).unwrap();
        insert_command(&conn, "ls", "test", "bash", Some("/home/a"), None, None).unwrap();
        insert_command(&conn, "ls", "test", "bash", Some("/home/a"), None, None).unwrap();
        insert_command(&conn, "git status", "test", "bash", Some("/home/a"), None, None).unwrap();
        insert_command(&conn, "git status", "test", "bash", Some("/home/a"), None, None).unwrap();
        insert_command(&conn, "ssh server", "test", "bash", Some("/tmp"), None, None).unwrap();

        let results = query_frequent(&conn, "", "/home/a", 10).unwrap();
        assert!(!results.is_empty(), "frequent query should return results");

        // ls should be first (freq=3), then git status (freq=2)
        assert_eq!(results[0].command, "ls");
        assert_eq!(results[0].freq, 3);
        assert_eq!(results[1].command, "git status");
        assert_eq!(results[1].freq, 2);

        // ssh server is not in /home/a, so it should have lower priority
        // and should still appear
        let ssh = results.iter().find(|e| e.command == "ssh server");
        assert!(ssh.is_some(), "ssh server should be in results");
        assert_eq!(ssh.unwrap().freq, 1);
    }

    #[test]
    fn test_query_recent() {
        let conn = setup_db();
        insert_command(&conn, "cmd1", "test", "bash", Some("/dir"), None, None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        insert_command(&conn, "cmd2", "test", "bash", Some("/dir"), None, None).unwrap();

        let results = query_recent(&conn, "", "/dir", 10).unwrap();
        assert_eq!(results.len(), 2);
        // Most recent first
        assert_eq!(results[0].command, "cmd2");
        assert_eq!(results[1].command, "cmd1");
    }

    #[test]
    fn test_fuzzy_match() {
        assert!(fuzzy_match("gst", "git status"));
        assert!(fuzzy_match("gb", "git branch"));
        assert!(fuzzy_match("cb", "cargo build"));
        assert!(fuzzy_match("cargo", "cargo build --release"));
        assert!(!fuzzy_match("xyz", "git status"));
        assert!(!fuzzy_match("gsx", "git status"));
        assert!(fuzzy_match("", "anything"));
        assert!(fuzzy_match("TEST", "cargo test")); // case-insensitive
    }
}
