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

/// Delete all rows matching (command, cwd). If cwd is None, matches rows with NULL cwd.
#[allow(dead_code)]
pub fn delete_command(conn: &Connection, command: &str, cwd: Option<&str>) -> rusqlite::Result<usize> {
    let count = match cwd {
        Some(c) => conn.execute(
            "DELETE FROM commands WHERE command = ?1 AND cwd = ?2",
            params![command, c],
        )?,
        None => conn.execute(
            "DELETE FROM commands WHERE command = ?1 AND cwd IS NULL",
            params![command],
        )?,
    };
    Ok(count)
}

// ── Varint helpers (Protocol Buffers 7-bit encoding) ──

fn varint_push(value: u64, buf: &mut Vec<u8>) {
    let mut v = value;
    while v >= 0x80 {
        buf.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    buf.push(v as u8);
}

fn varint_pop(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value: u64 = 0;
    let mut shift = 0;
    loop {
        if *pos >= data.len() {
            return None;
        }
        let byte = data[*pos];
        *pos += 1;
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

// ── base64url helpers (RFC 4648 §5, no padding) ──

const B64URL_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64url_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity((data.len() * 4 + 2) / 3);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(B64URL_ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        out.push(B64URL_ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() >= 2 {
            out.push(B64URL_ALPHABET[((triple >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() >= 3 {
            out.push(B64URL_ALPHABET[(triple & 0x3f) as usize] as char);
        }
    }
    out
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    if !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let bytes: Vec<u8> = s.bytes().filter_map(|b| {
        B64URL_ALPHABET.iter().position(|&x| x == b).map(|i| i as u8)
    }).collect();
    if bytes.len() != s.len() {
        return None;
    }

    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let a = chunk[0] as u32;
        let b = chunk.get(1).copied().unwrap_or(0) as u32;
        let c = chunk.get(2).copied().unwrap_or(0) as u32;
        let d = chunk.get(3).copied().unwrap_or(0) as u32;
        let triple = (a << 18) | (b << 12) | (c << 6) | d;

        out.push(((triple >> 16) & 0xff) as u8);
        if chunk.len() >= 3 {
            out.push(((triple >> 8) & 0xff) as u8);
        }
        if chunk.len() >= 4 {
            out.push((triple & 0xff) as u8);
        }
    }
    Some(out)
}

/// Encode a list of row IDs into a compact base64url string.
/// Algorithm: sort → dedup → delta → zigzag → varint → base64url.
/// Fully reversible; variable length.
pub fn encode_ids(ids: &[i64]) -> String {
    let mut sorted = ids.to_vec();
    sorted.sort();
    sorted.dedup();

    // Delta + zigzag + varint
    let mut buf = Vec::new();
    let mut prev: i64 = 0;
    for &id in &sorted {
        let delta = id - prev;
        prev = id;
        let zigzag = ((delta << 1) ^ (delta >> 63)) as u64;
        varint_push(zigzag, &mut buf);
    }

    b64url_encode(&buf)
}

/// Decode a base64url string back to a list of row IDs (reverse of encode_ids).
pub fn decode_ids(s: &str) -> Option<Vec<i64>> {
    let data = b64url_decode(s)?;
    let mut ids = Vec::new();
    let mut pos = 0;
    let mut prev: i64 = 0;
    while pos < data.len() {
        let zigzag = varint_pop(&data, &mut pos)? as i64;
        let delta = (zigzag >> 1) ^ (-(zigzag & 1));
        let id = prev + delta;
        prev = id;
        ids.push(id);
    }
    if ids.is_empty() {
        None
    } else {
        Some(ids)
    }
}

/// Delete rows by their primary key IDs.
pub fn delete_by_ids(conn: &Connection, ids: &[i64]) -> rusqlite::Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders: Vec<String> = ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect();
    let sql = format!("DELETE FROM commands WHERE id IN ({})", placeholders.join(","));
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::types::ToSql> = ids.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
    let count = stmt.execute(rusqlite::params_from_iter(params.iter()))?;
    Ok(count)
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
    /// Database row IDs backing this collapsed entry
    pub row_ids: Vec<i64>,
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
    let sql = "SELECT command, cwd, MAX(created_at) AS created_at, COUNT(*) AS freq, GROUP_CONCAT(id) AS ids
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
            row.get::<_, Option<String>>(4)?,
        ))
    })?;

    let mut map: HashMap<String, HistoryEntry> = HashMap::new();
    for row in rows {
        let (cmd, cwd, created_at, freq, ids_str) = row?;
        let ids: Vec<i64> = ids_str
            .as_deref()
            .map(|s| s.split(',').filter_map(|n| n.parse().ok()).collect())
            .unwrap_or_default();

        let entry = map.entry(cmd.clone()).or_insert_with(|| HistoryEntry {
            id: 0,
            command: cmd.clone(),
            cwd: None,
            created_at: None,
            freq: 0,
            row_ids: Vec::new(),
        });
        entry.freq += freq;
        entry.row_ids.extend(ids);
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
        if by_frequency {
            a_prio.cmp(&b_prio)
                .then(b.freq.cmp(&a.freq))
                .then(b.created_at.cmp(&a.created_at))
        } else {
            a_prio.cmp(&b_prio)
                .then(b.created_at.cmp(&a.created_at))
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
        row_ids: Vec::new(),
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

    #[test]
    fn test_encode_decode_single() {
        let ids = vec![42];
        let s = encode_ids(&ids);
        let decoded = decode_ids(&s).unwrap();
        assert_eq!(decoded, ids);
    }

    #[test]
    fn test_encode_decode_multiple() {
        let ids = vec![1042, 1043, 1045];
        let s = encode_ids(&ids);
        let decoded = decode_ids(&s).unwrap();
        assert_eq!(decoded, ids);
    }

    #[test]
    fn test_encode_decode_large_ids() {
        let ids = vec![1, 50, 1000, 50000, 999999];
        let s = encode_ids(&ids);
        let decoded = decode_ids(&s).unwrap();
        assert_eq!(decoded, ids);
    }

    #[test]
    fn test_encode_decode_unsorted_input() {
        let input = vec![1045, 1042, 1043]; // unsorted
        let s = encode_ids(&input);
        let decoded = decode_ids(&s).unwrap();
        // encode_ids sorts+ddups internally
        assert_eq!(decoded, vec![1042, 1043, 1045]);
    }

    #[test]
    fn test_decode_invalid() {
        assert!(decode_ids("!!!invalid!!!").is_none());
        assert!(decode_ids("").is_none());
    }
}
