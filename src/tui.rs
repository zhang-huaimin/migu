use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    cursor::SetCursorStyle,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, enable_raw_mode, disable_raw_mode},
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;
use std::time::Duration;

use crate::config::ResolvedKeys;
use crate::db::{HistoryEntry, query_collapsed};

/// Action the user took in the TUI.
#[derive(Debug, Clone)]
pub enum Action {
    /// Tab: insert command into shell prompt for editing
    Insert(String),
    /// Enter: insert command and signal shell to auto-execute
    Execute(String),
    /// Esc / Ctrl-C: quit without action
    Quit,
}

/// TUI state and entry point.
pub struct App {
    pub frequent_mode: bool,
    pub keyword: String,
    /// Byte position of cursor within keyword
    pub cursor_pos: usize,
    pub number_buf: String,
    pub selected: usize,
    pub cwd: String,
    pub entries: Vec<HistoryEntry>,
    pub limit: usize,
    pub action: Option<Action>,
    pub quit: bool,
    pub num_mode: bool,
    /// Alt+L: input new limit
    pub limit_mode: bool,
    pub limit_input: String,
    pub notification: Option<String>,
    pub notification_timer: u8,
    pub show_help: bool,
    pub total_count: usize,
    /// First visible row index (for scroll)
    pub scroll_offset: usize,
    /// Key bindings from config
    pub mod_toggle_sort: KeyModifiers,
    pub key_toggle_sort: char,
    pub mod_toggle_numbers: KeyModifiers,
    pub key_toggle_numbers: char,
    pub mod_toggle_help: KeyModifiers,
    pub key_toggle_help: char,
    pub mod_set_limit: KeyModifiers,
    pub key_set_limit: char,
    /// Whether the selected entry is expanded to show full command
    pub expanded: bool,
}

impl App {
    pub fn new(cwd: String, limit: usize, keys: &ResolvedKeys) -> Self {
        Self {
            frequent_mode: false,
            keyword: String::new(),
            cursor_pos: 0,
            number_buf: String::new(),
            selected: 0,
            cwd,
            entries: Vec::new(),
            limit,
            action: None,
            quit: false,
            num_mode: false,
            limit_mode: false,
            limit_input: String::new(),
            notification: None,
            notification_timer: 0,
            show_help: false,
            total_count: 0,
            scroll_offset: 0,
            mod_toggle_sort: keys.toggle_sort.0,
            key_toggle_sort: keys.toggle_sort.1,
            mod_toggle_numbers: keys.toggle_numbers.0,
            key_toggle_numbers: keys.toggle_numbers.1,
            mod_toggle_help: keys.toggle_help.0,
            key_toggle_help: keys.toggle_help.1,
            mod_set_limit: keys.set_limit.0,
            key_set_limit: keys.set_limit.1,
            expanded: false,
        }
    }

    /// Load entries from the database based on current mode and keyword.
    pub fn load_entries(&mut self, conn: &rusqlite::Connection) {
        // Load total count
        if let Ok(count) = conn.query_row("SELECT COUNT(*) FROM commands", [], |r| r.get::<_, usize>(0)) {
            self.total_count = count;
        }

        let query_limit = if self.keyword.is_empty() { self.limit } else { self.limit * 5 };

        let result = query_collapsed(conn, &self.keyword, &self.cwd, query_limit, self.frequent_mode);

        match result {
            Ok(entries) => {
                self.entries = entries;
                // Clamp selected index
                if self.entries.is_empty() {
                    self.selected = 0;
                } else if self.selected >= self.entries.len() {
                    self.selected = self.entries.len() - 1;
                }
            }
            Err(_e) => {
                self.entries.clear();
                self.selected = 0;
            }
        }
    }

    pub fn move_up(&mut self) {
        if !self.entries.is_empty() {
            if self.selected == 0 {
                self.selected = self.entries.len() - 1;
            } else {
                self.selected -= 1;
            }
        }
    }

    pub fn move_down(&mut self) {
        if !self.entries.is_empty() {
            if self.selected >= self.entries.len() - 1 {
                self.selected = 0;
            } else {
                self.selected += 1;
            }
        }
    }

    pub fn toggle_mode(&mut self) {
        self.frequent_mode = !self.frequent_mode;
    }

    pub fn push_digit(&mut self, digit: char) {
        self.number_buf.push(digit);
    }

    pub fn jump_to_number(&mut self) -> bool {
        if self.number_buf.is_empty() {
            return false;
        }
        if let Ok(n) = self.number_buf.parse::<usize>() {
            if n > 0 && n <= self.entries.len() {
                self.selected = n - 1;
            }
        }
        self.number_buf.clear();
        true
    }
    /// Enter: select current entry and signal the shell to auto-execute.
    pub fn select_execute(&mut self) {
        if let Some(entry) = self.entries.get(self.selected) {
            self.action = Some(Action::Execute(entry.command.clone()));
            self.quit = true;
        }
    }

    /// Tab: select current entry for insertion into the shell prompt (no auto-execute).
    pub fn select_insert(&mut self) {
        if let Some(entry) = self.entries.get(self.selected) {
            self.action = Some(Action::Insert(entry.command.clone()));
            self.quit = true;
        }
    }
}

/// Run the interactive TUI and return the user's action.
pub fn run(
    conn: &rusqlite::Connection,
    cwd: &str,
    limit: usize,
    keys: &ResolvedKeys,
) -> io::Result<Action> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, SetCursorStyle::BlinkingBar)?;

    // Drain any buffered input (e.g. leftover from bash's readline)
    unsafe { libc::tcflush(0, libc::TCIFLUSH) };

    let backend = ratatui::backend::CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(cwd.to_string(), limit, keys);
    app.load_entries(conn);

    let result = run_loop(&mut terminal, &mut app, conn);

    // Restore terminal state
    execute!(io::stdout(), LeaveAlternateScreen)?;
    disable_raw_mode()?;
    terminal.show_cursor()?;

    result?;
    Ok(app.action.unwrap_or(Action::Quit))
}

fn is_mod_key(key: &crossterm::event::KeyEvent, modifier: KeyModifiers, ch: char) -> bool {
    key.modifiers == modifier && key.code == KeyCode::Char(ch)
}

fn run_loop(
    terminal: &mut Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
    app: &mut App,
    conn: &rusqlite::Connection,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;
        terminal.show_cursor()?;

        if app.notification_timer > 0 {
            app.notification_timer -= 1;
            if app.notification_timer == 0 {
                app.notification = None;
            }
        }

        if app.quit {
            return Ok(());
        }

        // Poll with 50ms timeout so notifications can expire
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Release {
                    continue;
                }

            // Ctrl+C quits
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                app.action = Some(Action::Quit);
                app.quit = true;
                continue;
            }

            match key.code {
                // Any key closes help
                _ if app.show_help => {
                    app.show_help = false;
                }
                KeyCode::Esc => {
                    if app.limit_mode {
                        app.limit_mode = false;
                        app.limit_input.clear();
                    } else if app.num_mode {
                        app.num_mode = false;
                        app.number_buf.clear();
                    } else {
                        app.action = Some(Action::Quit);
                        app.quit = true;
                    }
                }
                KeyCode::Char(_) if is_mod_key(&key, app.mod_toggle_sort, app.key_toggle_sort) => {
                    app.toggle_mode();
                    let mode_name = if app.frequent_mode { "频率顺序" } else { "时间顺序" };
                    app.notification = Some(mode_name.to_string());
                    app.notification_timer = 40;
                    app.load_entries(conn);
                }
                KeyCode::Char(_) if is_mod_key(&key, app.mod_toggle_numbers, app.key_toggle_numbers) => {
                    app.num_mode = !app.num_mode;
                }
                KeyCode::Char(_) if is_mod_key(&key, app.mod_toggle_help, app.key_toggle_help) => {
                    app.show_help = !app.show_help;
                }
                KeyCode::Char(_) if is_mod_key(&key, app.mod_set_limit, app.key_set_limit) => {
                    app.limit_mode = true;
                    app.limit_input.clear();
                }
                // Alt+Z: toggle expand mode
                KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::ALT) => {
                    app.expanded = !app.expanded;
                }
                KeyCode::Char(c) if app.limit_mode && c.is_ascii_digit() => {
                    app.limit_input.push(c);
                }
                KeyCode::Backspace if app.limit_mode => {
                    app.limit_input.pop();
                }
                // Ctrl+H sends Backspace on some terminals
                KeyCode::Char('h') if app.limit_mode && key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.limit_input.pop();
                }
                KeyCode::Enter if app.limit_mode => {
                    if let Ok(n) = app.limit_input.parse::<usize>() {
                        let n = n.clamp(1, 100);
                        app.limit = n;
                        app.notification = Some(format!("已设置为 {} 条", n));
                        app.notification_timer = 60;
                    }
                    app.limit_mode = false;
                    app.limit_input.clear();
                    app.load_entries(conn);
                }
                KeyCode::Char(c) if app.num_mode && c.is_ascii_digit() => {
                    app.push_digit(c);
                }
                KeyCode::Enter if app.num_mode => {
                    app.jump_to_number();
                    app.num_mode = false;
                }
                KeyCode::Up => {
                    app.move_up();
                }
                KeyCode::Down => {
                    app.move_down();
                }
                KeyCode::Left
                    if app.cursor_pos > 0 => {
                        let mut prev = app.cursor_pos - 1;
                        while prev > 0 && !app.keyword.is_char_boundary(prev) {
                            prev -= 1;
                        }
                        app.cursor_pos = prev;
                    }
                KeyCode::Right
                    if app.cursor_pos < app.keyword.len() => {
                        let mut next = app.cursor_pos + 1;
                        while next < app.keyword.len() && !app.keyword.is_char_boundary(next) {
                            next += 1;
                        }
                        app.cursor_pos = next;
                    }
                KeyCode::Home => {
                    app.cursor_pos = 0;
                }
                KeyCode::End => {
                    app.cursor_pos = app.keyword.len();
                }
                KeyCode::PageDown
                    if !app.entries.is_empty() => {
                        let page = 10usize;
                        app.selected = (app.selected + page).min(app.entries.len() - 1);
                        app.scroll_offset = (app.scroll_offset + page).min(app.entries.len().saturating_sub(20));
                    }
                KeyCode::PageUp => {
                    let page = 10usize;
                    app.selected = app.selected.saturating_sub(page);
                    app.scroll_offset = app.scroll_offset.saturating_sub(page);
                }
                KeyCode::Tab
                    if !app.jump_to_number() => {
                        app.select_insert();
                    }
                KeyCode::Enter
                    if !app.jump_to_number() => {
                        app.select_execute();
                    }
                KeyCode::Backspace => {
                    if !app.number_buf.is_empty() {
                        app.number_buf.clear();
                    } else if app.cursor_pos > 0 {
                        // Delete char before cursor
                        let mut prev = app.cursor_pos - 1;
                        while prev > 0 && !app.keyword.is_char_boundary(prev) {
                            prev -= 1;
                        }
                        app.keyword.remove(prev);
                        app.cursor_pos = prev;
                        app.load_entries(conn);
                    }
                }
                // Ctrl+H sends Backspace on some terminals
                KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if !app.number_buf.is_empty() {
                        app.number_buf.clear();
                    } else if app.cursor_pos > 0 {
                        let mut prev = app.cursor_pos - 1;
                        while prev > 0 && !app.keyword.is_char_boundary(prev) {
                            prev -= 1;
                        }
                        app.keyword.remove(prev);
                        app.cursor_pos = prev;
                        app.load_entries(conn);
                    }
                }
                KeyCode::Delete
                    if app.cursor_pos < app.keyword.len() => {
                        // Delete char at cursor
                        let mut next = app.cursor_pos + 1;
                        while next < app.keyword.len() && !app.keyword.is_char_boundary(next) {
                            next += 1;
                        }
                        app.keyword.drain(app.cursor_pos..next);
                        app.load_entries(conn);
                    }
                KeyCode::Char(c)
                    if (c.is_ascii_graphic() || c == ' ') => {
                        app.number_buf.clear();
                        app.keyword.insert(app.cursor_pos, c);
                        app.cursor_pos += c.len_utf8();
                        app.load_entries(conn);
                    }
                _ => {}
            }
        }
    }
    }
}

/// Render the full UI.
fn ui(f: &mut Frame, app: &mut App) {
    if app.show_help {
        render_help(f, f.area(), app);
        return;
    }

    let area = f.area();

    let expanded_height = if app.expanded && !app.entries.is_empty() {
        let entry = &app.entries[app.selected];
        let text = strip_ansi(&entry.command);
        let width = area.width.max(1) as usize;
        let lines: usize = text.lines()
            .map(|l| (l.chars().count().max(1) + width - 1) / width)
            .sum();
        (lines as u16).clamp(2, area.height / 2)
    } else {
        0
    };
    let list_height = if expanded_height > 0 {
        Constraint::Length(area.height.saturating_sub(3 + expanded_height + 1).max(1))
    } else {
        Constraint::Min(1)
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // header
            Constraint::Length(1),  // divider
            list_height,
            Constraint::Length(if expanded_height > 0 { 1 } else { 0 }), // separator
            Constraint::Length(expanded_height), // expanded detail
            Constraint::Length(1),  // footer
        ])
        .split(area);

    render_header(f, chunks[0], app);
    render_divider(f, chunks[1], app);
    render_list(f, chunks[2], app);
    if expanded_height > 0 {
        let sep_line = "─".repeat(area.width as usize);
        let sep = Paragraph::new(Line::from(Span::styled(sep_line, Style::default().fg(Color::Rgb(95, 95, 95)))));
        f.render_widget(sep, chunks[3]);
        render_expanded_detail(f, chunks[4], app);
    }
    render_footer(f, chunks[5], app);
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let label_style = Style::default().fg(Color::DarkGray);
    let input_style = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
    let jump_style = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);

    // Truncate keyword to fit within terminal
    let max_keyword = (area.width as usize).saturating_sub(12);
    let visible_keyword: String = app.keyword.chars().rev().take(max_keyword).collect::<Vec<_>>().into_iter().rev().collect();

    let mut spans = vec![
        Span::styled("> ", label_style),
        Span::styled(&visible_keyword, input_style),
    ];

    if app.num_mode {
        spans.push(Span::styled(format!(" │ 跳转：{}_", app.number_buf), jump_style));
    }
    if app.limit_mode {
        spans.push(Span::styled(format!(" │ 上限：{}_", app.limit_input), jump_style));
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line);
    f.render_widget(paragraph, area);

    let cursor_col = 2 + app.cursor_pos as u16;
    f.set_cursor_position((area.x + cursor_col, area.y));
}

fn render_divider(f: &mut Frame, area: Rect, app: &App) {
    let text = format!(" {}/{}", app.selected.saturating_add(1).min(app.entries.len()), app.entries.len());
    let line_char = "─";
    let total = area.width as usize;
    let right_len = total.saturating_sub(text.len());
    let line = format!("{} {}",
        text,
        line_char.repeat(right_len.saturating_sub(1)),
    );
    let span = Span::styled(line, Style::default().fg(Color::Rgb(95, 95, 95)));
    f.render_widget(Paragraph::new(Line::from(span)), area);
}

fn render_list(f: &mut Frame, area: Rect, app: &mut App) {
    let visible_rows = area.height as usize;
    let context_lines = 3usize;
    let mut offset = app.scroll_offset;
    let ctx = context_lines.min(visible_rows / 2);
    // Scroll up if selected is too close to top
    if app.selected < offset + ctx {
        offset = app.selected.saturating_sub(ctx);
    }
    // Scroll down if selected is too close to bottom
    if app.selected + ctx >= offset + visible_rows {
        offset = app.selected + ctx + 1 - visible_rows;
    }
    // Clamp offset
    let max_offset = app.entries.len().saturating_sub(visible_rows);
    offset = offset.min(max_offset);
    app.scroll_offset = offset;

    let visible_entries: Vec<(usize, &HistoryEntry)> = app
        .entries
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible_rows)
        .collect();

    let items: Vec<ListItem> = visible_entries
        .iter()
        .map(|&(i, entry)| {
            let is_selected = i == app.selected;
            let num = format!(" {:2}.", i + 1);
            let cmd_text = first_line(&entry.command);
            let cwd_display = entry.cwd.as_deref()
                .filter(|c| *c != app.cwd)
                .map(shorten_cwd)
                .unwrap_or_default();
            let time_display = entry.created_at.as_deref().map(relative_time).unwrap_or_default();

            let freq_badge = if entry.freq > 1 {
                format!(" (x{})", entry.freq)
            } else {
                String::new()
            };

            let line = Line::from(vec![
                Span::styled(num, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw(format!(" {} ", cmd_text)),
                Span::styled(cwd_display, Style::default().fg(Color::DarkGray)),
                Span::raw("  "),
                Span::styled(time_display, Style::default().fg(Color::Green)),
                Span::styled(freq_badge, Style::default().fg(Color::Magenta)),
            ]);

            let style = if is_selected {
                Style::default().bg(Color::Rgb(5, 122, 212)).fg(Color::White)
            } else {
                Style::default()
            };

            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::NONE));
    f.render_widget(list, area);
}

fn render_expanded_detail(f: &mut Frame, area: Rect, app: &App) {
    let entry = match app.entries.get(app.selected) {
        Some(e) => e,
        None => return,
    };
    let text = strip_ansi(&entry.command);
    let para = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::Rgb(200, 200, 200)));
    f.render_widget(para, area);
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    // In expanded mode, footer is hidden
    if app.expanded && !app.entries.is_empty() {
        return;
    }
    let selected_entry = app.entries.get(app.selected);

    if let Some(entry) = selected_entry {
        if entry.command.contains('\n') {
            let full_cmd = strip_ansi(&entry.command).replace('\n', " \\n ");
            let para = Paragraph::new(full_cmd);
            f.render_widget(para, area);
            return;
        }
    }

    // Split footer: notification (left) | key hints (right)
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Left: notification
    if let Some(ref msg) = app.notification {
        let para = Paragraph::new(Line::from(Span::styled(msg, Style::default().fg(Color::Gray))));
        f.render_widget(para, chunks[0]);
    }

    // Right: always empty (help moved to dedicated page via Alt+h)
    let _ = chunks[1];
}

fn render_help(f: &mut Frame, area: Rect, app: &App) {
    let key_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(Color::White);

    let mk = |m: KeyModifiers, ch: char| {
        let mod_name = modifier_name(m);
        if mod_name.is_empty() {
            ch.to_string()
        } else {
            format!("{} + {}", mod_name, ch)
        }
    };

    let pairs: Vec<(String, &str)> = vec![
        ("按键".to_string(), "功能"),
        (String::new(), ""),
        ("> keyword".to_string(), "输入关键字过滤历史"),
        ("Enter".to_string(), "执行高亮命令"),
        ("Tab".to_string(), "选中命令到命令行"),
        (mk(app.mod_toggle_sort, app.key_toggle_sort), "切换时间 / 频率排序"),
        (mk(app.mod_toggle_numbers, app.key_toggle_numbers), "进入数字跳转模式"),
        (mk(app.mod_toggle_help, app.key_toggle_help), "显示 / 隐藏此帮助"),
        (mk(app.mod_set_limit, app.key_set_limit), "设置显示条数"),
        ("Esc".to_string(), "退出（跳转模式中取消）"),
        ("Alt + z".to_string(), "展开 / 折叠选中命令"),
        ("Ctrl + C".to_string(), "强制退出"),
        ("↑ ↓ PgUp PgDn".to_string(), "上下导航 / 翻页"),
        ("← → Home End".to_string(), "搜索框内移动光标"),
        ("Backspace Del".to_string(), "搜索框内删除字符"),
    ];

    let inner = Block::default().borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(5, 122, 212)))
        .title(" 帮助 ");
    let inner_area = inner.inner(area);
    f.render_widget(inner, area);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(18), Constraint::Min(0)])
        .split(inner_area);

    let keys: Vec<Line> = pairs.iter().map(|(k, _)| {
        Line::from(Span::styled(k.as_str(), key_style))
    }).collect();
    let descs: Vec<Line> = pairs.iter().map(|(_, d)| {
        Line::from(Span::styled(*d, desc_style))
    }).collect();

    f.render_widget(Paragraph::new(keys), chunks[0]);
    f.render_widget(Paragraph::new(descs), chunks[1]);
}

/// Convert KeyModifiers to a human-readable string.
fn modifier_name(m: KeyModifiers) -> String {
    if m.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    if m.contains(KeyModifiers::CONTROL) {
        parts.push("Ctrl");
    }
    if m.contains(KeyModifiers::ALT) {
        parts.push("Alt");
    }
    if m.contains(KeyModifiers::SHIFT) {
        parts.push("Shift");
    }
    parts.join(" + ")
}

/// Strip ANSI escape codes from a string to prevent terminal corruption.
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            // Skip until we find a letter (end of CSI sequence)
            chars.next(); // skip '['
            while let Some(&nc) = chars.peek() {
                if nc.is_ascii_alphabetic() || nc == '~' {
                    chars.next();
                    break;
                }
                chars.next();
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn first_line(cmd: &str) -> String {
    let cmd = strip_ansi(cmd);
    let line = cmd.lines().next().unwrap_or("");
    let max_len = 80;
    if line.chars().count() > max_len {
        format!("{}...", line.chars().take(max_len).collect::<String>())
    } else {
        let mut s = line.to_string();
        if cmd.contains('\n') {
            s.push_str(" ...");
        }
        s
    }
}

fn shorten_cwd(cwd: &str) -> String {
    let home = dirs::home_dir()
        .and_then(|h| h.to_str().map(|s| s.to_string()))
        .unwrap_or_default();

    let shortened = if !home.is_empty() && cwd.starts_with(&home) {
        format!("~{}", &cwd[home.len()..])
    } else {
        cwd.to_string()
    };

    if shortened.len() > 30 {
        format!("...{}", &shortened[shortened.len().saturating_sub(27)..])
    } else {
        shortened
    }
}

fn relative_time(iso: &str) -> String {
    let parsed = chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S");
    let dt = match parsed {
        Ok(dt) => dt.and_utc(),
        Err(_) => return String::new(),
    };

    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(dt);

    if duration.num_seconds() < 0 {
        return "刚刚".to_string();
    }

    let secs = duration.num_seconds() as u64;
    if secs < 60 {
        format!("{}秒前", secs)
    } else if secs < 3600 {
        format!("{}分钟前", secs / 60)
    } else if secs < 86400 {
        format!("{}小时前", secs / 3600)
    } else if secs < 604800 {
        format!("{}天前", secs / 86400)
    } else {
        format!("{}周前", secs / 604800)
    }
}
