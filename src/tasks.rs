use chrono::{Local, NaiveDate};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

static TASK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*(?:[-*+]|\d+[.)]) \[)(.)(\] )(.*)$").unwrap());
static DATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(📅|⏳|🛫|➕|✅|❌)️?\s*(\d{4}-\d{2}-\d{2})").unwrap()
});
static BLOCK_ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s\^([A-Za-z0-9-]+)\s*$").unwrap());
static RECUR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"🔁️?\s*([^📅⏳🛫➕✅❌🔺⏫🔼🔽⏬]*)").unwrap());

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Priority {
    Highest = 0,
    High = 1,
    Medium = 2,
    Normal = 3,
    Low = 4,
    Lowest = 5,
}

impl Priority {
    pub fn icon(self) -> &'static str {
        match self {
            Priority::Highest => "🔺",
            Priority::High => "⏫",
            Priority::Medium => "🔼",
            Priority::Normal => "",
            Priority::Low => "🔽",
            Priority::Lowest => "⏬",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Task {
    pub vault: String,
    pub vault_root: PathBuf,
    pub file: PathBuf,
    /// 0-based line number
    pub line: usize,
    pub raw_line: String,
    pub description: String,
    pub due: Option<NaiveDate>,
    pub scheduled: Option<NaiveDate>,
    pub start: Option<NaiveDate>,
    pub priority: Priority,
    pub recurrence: Option<String>,
    pub in_progress: bool,
    /// Checked off (`[x]`).
    pub completed: bool,
    /// Done date (`✅ YYYY-MM-DD`).
    pub done: Option<NaiveDate>,
    /// Obsidian block ID (`^abc123` at the end of the line), used to attach app-only notes and lists.
    pub block_id: Option<String>,
}

impl Task {
    pub fn rel_path(&self) -> String {
        self.file
            .strip_prefix(&self.vault_root)
            .unwrap_or(&self.file)
            .to_string_lossy()
            .to_string()
    }

    /// On the Today list: scheduled for today or earlier.
    pub fn starred(&self, today: NaiveDate) -> bool {
        self.scheduled.is_some_and(|d| d <= today)
    }
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

pub fn parse_line(line: &str) -> Option<(char, String)> {
    let caps = TASK_RE.captures(line)?;
    let status = caps.get(2)?.as_str().chars().next()?;
    let body = caps.get(4)?.as_str().trim().to_string();
    Some((status, body))
}

fn build_task(vault: &str, root: &Path, file: &Path, line_no: usize, line: &str) -> Option<Task> {
    let (status, body) = parse_line(line)?;
    // Open = space, in progress = '/', completed = x/X. Others (cancelled '-', '>', etc.) are skipped.
    let completed = matches!(status, 'x' | 'X');
    if !completed && status != ' ' && status != '/' {
        return None;
    }
    if body.is_empty() {
        return None;
    }
    let mut due = None;
    let mut scheduled = None;
    let mut start = None;
    let mut done = None;
    for c in DATE_RE.captures_iter(&body) {
        let d = parse_date(&c[2]);
        match &c[1] {
            "📅" => due = d,
            "⏳" => scheduled = d,
            "🛫" => start = d,
            "✅" => done = d,
            _ => {}
        }
    }
    let priority = if body.contains('🔺') {
        Priority::Highest
    } else if body.contains('⏫') {
        Priority::High
    } else if body.contains('🔼') {
        Priority::Medium
    } else if body.contains('🔽') {
        Priority::Low
    } else if body.contains('⏬') {
        Priority::Lowest
    } else {
        Priority::Normal
    };
    let recurrence = RECUR_RE
        .captures(&body)
        .map(|c| c[1].trim().to_string())
        .filter(|s| !s.is_empty());

    let block_id = BLOCK_ID_RE.captures(&body).map(|c| c[1].to_string());

    // Clean description: strip metadata emoji + values.
    let mut desc = BLOCK_ID_RE.replace(&body, "").to_string();
    desc = DATE_RE.replace_all(&desc, "").to_string();
    desc = RECUR_RE.replace_all(&desc, "").to_string();
    for p in ["🔺", "⏫", "🔼", "🔽", "⏬"] {
        desc = desc.replace(p, "");
    }
    let description = desc.split_whitespace().collect::<Vec<_>>().join(" ");
    if description.is_empty() {
        return None;
    }

    Some(Task {
        vault: vault.to_string(),
        vault_root: root.to_path_buf(),
        file: file.to_path_buf(),
        line: line_no,
        raw_line: line.to_string(),
        description,
        due,
        scheduled,
        start,
        priority,
        recurrence,
        in_progress: status == '/',
        completed,
        done,
        block_id,
    })
}

pub fn scan_vault(root: &Path, excluded: &[String]) -> Vec<Task> {
    let excluded: Vec<String> = excluded.iter().map(|e| e.to_lowercase()).collect();
    let vault = root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut out = Vec::new();
    let walker = WalkDir::new(root).into_iter().filter_entry(|e| {
        // Skip hidden folders like .obsidian and .trash
        if e.depth() == 0 {
            return true;
        }
        if e.file_name().to_string_lossy().starts_with('.') {
            return false;
        }
        let rel = e.path().strip_prefix(root).unwrap_or(e.path()).to_string_lossy().to_lowercase();
        !excluded.iter().any(|x| rel.contains(x.as_str()))
    });
    for entry in walker.flatten() {
        let p = entry.path();
        if !entry.file_type().is_file() || p.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(text) = fs::read_to_string(p) else { continue };
        let mut in_code = false;
        for (i, line) in text.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("```") || t.starts_with("~~~") {
                in_code = !in_code;
                continue;
            }
            if in_code {
                continue;
            }
            if let Some(task) = build_task(&vault, root, p, i, line) {
                out.push(task);
            }
        }
    }
    out
}

/// Marks a task done in its source file, using the Tasks plugin format (`[x]` + `✅ YYYY-MM-DD`).
/// Re-reads the file and only writes if the line is unchanged since the last scan.
/// Re-reads the task's file, finds its line (verifying it is unchanged since the scan),
/// applies `edit` to it, and writes the file back.
fn rewrite_line(task: &Task, edit: impl FnOnce(&str) -> Result<String, String>) -> Result<(), String> {
    // `.map(Some)` wraps the edited line: Some(line) keeps a line, None (see delete_task) removes it.
    edit_line(task, |line| edit(line).map(Some))
}

/// Shared by rewrite_line and delete_task: re-reads the file, finds the task's line
/// (checking it's unchanged since the scan), then replaces it with what `edit`
/// returns, or removes it if `edit` returns None. Every other line is kept as is.
fn edit_line(task: &Task, edit: impl FnOnce(&str) -> Result<Option<String>, String>) -> Result<(), String> {
    let text = fs::read_to_string(&task.file).map_err(|e| e.to_string())?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let trailing = text.ends_with('\n');
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let idx = if lines.get(task.line).map(|l| l == &task.raw_line) == Some(true) {
        task.line
    } else {
        // File shifted; fall back to finding the exact line.
        lines
            .iter()
            .position(|l| l == &task.raw_line)
            .ok_or("Task changed since last scan. Refresh and try again.")?
    };
    match edit(&lines[idx])? {
        Some(new) => lines[idx] = new,
        None => {
            lines.remove(idx);
        }
    }
    let mut out = lines.join(newline);
    if trailing && !lines.is_empty() {
        out.push_str(newline);
    }
    fs::write(&task.file, out).map_err(|e| e.to_string())
}

/// Deletes the task's line from its note. Nothing else in the file changes.
pub fn delete_task(task: &Task) -> Result<(), String> {
    edit_line(task, |_| Ok(None))
}

fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// Marks a task done using the Tasks plugin format (`[x]` + `✅ YYYY-MM-DD`).
pub fn complete_task(task: &Task) -> Result<(), String> {
    rewrite_line(task, |line| {
        let caps = TASK_RE.captures(line).ok_or("Line is no longer a task")?;
        let line = format!("{}x{}{}", &caps[1], &caps[3], caps[4].trim_end());
        Ok(insert_field(line, &format!("✅ {}", ymd(Local::now().date_naive()))))
    })
}

static DONE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*✅\uFE0F?\s*\d{4}-\d{2}-\d{2}").unwrap());

/// Reopens a completed task: `[x]` back to `[ ]` and removes the `✅` date.
pub fn uncomplete_task(task: &Task) -> Result<(), String> {
    rewrite_line(task, |line| {
        let caps = TASK_RE.captures(line).ok_or("Line is no longer a task")?;
        let body = DONE_RE.replace_all(&caps[4], "").trim_end().to_string();
        Ok(format!("{} {}{}", &caps[1], &caps[3], body))
    })
}

static SCHED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*⏳\uFE0F?\s*\d{4}-\d{2}-\d{2}").unwrap());
static DUE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*📅\uFE0F?\s*\d{4}-\d{2}-\d{2}").unwrap());
/// Removes one date field (matched by `re`) and, if `date` is set, re-adds it as
/// `<emoji> YYYY-MM-DD` at the end of the line (before a trailing `^block-id`).
fn replace_date_field(line: &str, re: &Regex, emoji: &str, date: Option<NaiveDate>) -> String {
    let l = re.replace_all(line, "").trim_end().to_string();
    let Some(d) = date else { return l };
    insert_field(l, &format!("{emoji} {}", ymd(d)))
}

/// Appends a field at the end of the line, before a trailing `^block-id`.
fn insert_field(line: String, field: &str) -> String {
    let l = line.trim_end();
    match BLOCK_ID_RE.find(l) {
        Some(m) => format!("{} {}{}", &l[..m.start()], field, &l[m.start()..]),
        None => format!("{l} {field}"),
    }
}

/// Matches any Tasks plugin metadata (dates, recurrence, priority) or a trailing block ID.
static META_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:📅|⏳|🛫|➕|✅|❌)\uFE0F?\s*\d{4}-\d{2}-\d{2}|🔁\uFE0F?[^📅⏳🛫➕✅❌🔺⏫🔼🔽⏬^]*|[🔺⏫🔼🔽⏬]\uFE0F?|\s\^[A-Za-z0-9-]+\s*$").unwrap()
});

/// Builds a task body with new text, keeping all metadata. When the metadata all
/// follows the text (the usual Tasks plugin layout) it is kept exactly as written;
/// otherwise the metadata pieces are moved after the new text in their original order.
fn rename_body(body: &str, new_text: &str) -> String {
    let body = body.trim_end();
    let Some(first) = META_RE.find(body) else { return new_text.to_string() };
    let tail = &body[first.start()..];
    let text_after_meta = META_RE.replace_all(tail, "");
    if text_after_meta.trim().is_empty() {
        return format!("{new_text} {}", tail.trim_start());
    }
    let pieces: Vec<&str> = META_RE.find_iter(body).map(|m| m.as_str().trim()).collect();
    format!("{new_text} {}", pieces.join(" "))
}

/// Changes a task's text, keeping its checkbox, dates, priority, recurrence and block ID.
pub fn rename_task(task: &Task, new_text: &str) -> Result<(), String> {
    let new_text = new_text.split_whitespace().collect::<Vec<_>>().join(" ");
    if new_text.is_empty() {
        return Err("Task name can't be empty".into());
    }
    rewrite_line(task, |line| {
        let caps = TASK_RE.captures(line).ok_or("Line is no longer a task")?;
        Ok(format!("{}{}{}{}", &caps[1], &caps[2], &caps[3], rename_body(&caps[4], &new_text)))
    })
}

/// A short random block ID, e.g. `vt-k3x9q2`.
pub fn new_block_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut n = nanos ^ COUNTER.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut id = String::from("vt-");
    for _ in 0..6 {
        id.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % 36) as usize] as char);
        n /= 36;
    }
    id
}

/// Returns the task's block ID, adding one (`^vt-xxxxxx`) to its line if it has none.
pub fn ensure_block_id(task: &Task) -> Result<String, String> {
    if let Some(id) = &task.block_id {
        return Ok(id.clone());
    }
    let id = new_block_id();
    rewrite_line(task, |line| Ok(format!("{} ^{id}", line.trim_end())))?;
    Ok(id)
}

/// Sets or clears the Tasks plugin scheduled date (`⏳ YYYY-MM-DD`). Used for the Today list.
pub fn set_scheduled(task: &Task, date: Option<NaiveDate>) -> Result<(), String> {
    rewrite_line(task, |line| Ok(replace_date_field(line, &SCHED_RE, "⏳", date)))
}

/// Sets or clears the due date (`📅 YYYY-MM-DD`).
pub fn set_due(task: &Task, date: Option<NaiveDate>) -> Result<(), String> {
    rewrite_line(task, |line| Ok(replace_date_field(line, &DUE_RE, "📅", date)))
}

pub const INBOX_FILE: &str = "Vault Tasks.md";

/// Appends a new task to the app-managed file at the root of the vault.
pub fn add_task(
    vault_root: &Path,
    text: &str,
    due: Option<NaiveDate>,
    scheduled: Option<NaiveDate>,
    block_id: Option<&str>,
) -> Result<(), String> {
    add_task_in(vault_root, INBOX_FILE, text, due, scheduled, block_id)
}

/// Appends a new task to `note` (a path inside the vault, e.g. "Inbox.md" or
/// "Notes/Inbox"). Creates the note, and any folders, if it doesn't exist.
/// Re-reads the note right before writing, so edits made in Obsidian are kept.
pub fn add_task_in(
    vault_root: &Path,
    note: &str,
    text: &str,
    due: Option<NaiveDate>,
    scheduled: Option<NaiveDate>,
    block_id: Option<&str>,
) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Task is empty".into());
    }
    let mut line = format!("- [ ] {} ➕ {}", text, ymd(Local::now().date_naive()));
    if let Some(d) = scheduled {
        line.push_str(&format!(" ⏳ {}", ymd(d)));
    }
    if let Some(d) = due {
        line.push_str(&format!(" 📅 {}", ymd(d)));
    }
    if let Some(id) = block_id {
        line.push_str(&format!(" ^{id}"));
    }
    append_line(vault_root, note, &line)
}

/// The Markdown file for `note` inside the vault, refusing paths that leave it.
fn note_path(vault_root: &Path, note: &str) -> Result<PathBuf, String> {
    let note = note.trim().trim_start_matches('/');
    if note.is_empty() || note.split('/').any(|p| p == "..") {
        return Err("Pick a note name inside the vault".into());
    }
    let mut path = vault_root.join(note);
    if path.extension().and_then(|e| e.to_str()) != Some("md") {
        path.set_extension("md");
    }
    Ok(path)
}

/// Adds one line at the end of a note, creating the note (with a heading) if needed.
fn append_line(vault_root: &Path, note: &str, line: &str) -> Result<(), String> {
    let path = note_path(vault_root, note)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut content = fs::read_to_string(&path).unwrap_or_else(|_| {
        if path == vault_root.join(INBOX_FILE) {
            "# Vault Tasks\n\nTasks created in the Vault Tasks app.\n\n".to_string()
        } else {
            let title = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            format!("# {title}\n\n")
        }
    });
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(line);
    content.push('\n');
    fs::write(&path, content).map_err(|e| e.to_string())
}

/// Moves a task to another vault: adds the same line (checkbox, text, dates, priority,
/// recurrence and block ID) to `note` in `dest_root`, then deletes it from its old note.
/// If the delete fails, the new note is put back as it was, so the task never ends up
/// in both vaults.
pub fn move_task(task: &Task, dest_root: &Path, note: &str) -> Result<(), String> {
    let caps = TASK_RE.captures(&task.raw_line).ok_or("Line is no longer a task")?;
    // Written at the top level of the note, without the old indentation or list number.
    let line = format!("- [{}] {}", &caps[2], caps[4].trim());
    let path = note_path(dest_root, note)?;
    let before = fs::read(&path).ok();
    append_line(dest_root, note, &line)?;
    if let Err(e) = delete_task(task) {
        let _ = match before {
            Some(b) => fs::write(&path, b),
            None => fs::remove_file(&path),
        };
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_metadata() {
        let t = build_task(
            "V",
            Path::new("/v"),
            Path::new("/v/a.md"),
            0,
            "- [ ] Ship v2 release #work 📅 2026-07-15 ⏫ ➕ 2026-07-01",
        )
        .unwrap();
        assert_eq!(t.description, "Ship v2 release #work");
        assert_eq!(t.due, parse_date("2026-07-15"));
        assert_eq!(t.priority, Priority::High);
    }

    #[test]
    fn skips_done_and_empty() {
        let p = Path::new("/v/a.md");
        let done = build_task("V", Path::new("/v"), p, 0, "- [x] done ✅ 2026-11-01").unwrap();
        assert!(done.completed);
        assert_eq!(done.done, parse_date("2026-11-01"));
        assert!(build_task("V", Path::new("/v"), p, 0, "- [-] cancelled").is_none());
        assert!(build_task("V", Path::new("/v"), p, 0, "- [ ]").is_none());
        assert!(build_task("V", Path::new("/v"), p, 0, "- [ ] ").is_none());
        assert!(build_task("V", Path::new("/v"), p, 0, "\t- [ ] nested").is_some());
    }

    #[test]
    fn completes_in_place() {
        let dir = std::env::temp_dir().join("vt-test");
        let _ = fs::create_dir_all(&dir);
        let f = dir.join("n.md");
        fs::write(&f, "# H\n- [ ] keep\n\t- [ ] Do it 📅 2026-01-01\n").unwrap();
        let t = scan_vault(&dir, &[]).into_iter().find(|t| t.description == "Do it").unwrap();
        complete_task(&t).unwrap();
        let out = fs::read_to_string(&f).unwrap();
        let today = Local::now().date_naive().format("%Y-%m-%d").to_string();
        assert_eq!(out, format!("# H\n- [ ] keep\n\t- [x] Do it 📅 2026-01-01 ✅ {today}\n"));
        let t = scan_vault(&dir, &[]).into_iter().find(|t| t.description == "Do it").unwrap();
        assert!(t.completed);
        uncomplete_task(&t).unwrap();
        let out = fs::read_to_string(&f).unwrap();
        assert_eq!(out, "# H\n- [ ] keep\n\t- [ ] Do it 📅 2026-01-01\n");
    }

    #[test]
    fn star_and_add() {
        let dir = std::env::temp_dir().join("vt-test2");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let today = Local::now().date_naive();
        add_task(&dir, "New thing", parse_date("2026-12-01"), None, None).unwrap();
        let t = scan_vault(&dir, &[]).pop().unwrap();
        assert_eq!(t.description, "New thing");
        assert!(!t.starred(today));
        set_scheduled(&t, Some(today)).unwrap();
        let t = scan_vault(&dir, &[]).pop().unwrap();
        assert!(t.starred(today));
        set_scheduled(&t, None).unwrap();
        let t = scan_vault(&dir, &[]).pop().unwrap();
        assert!(!t.starred(today));
        assert_eq!(t.due, parse_date("2026-12-01"));
        assert!(fs::read_to_string(dir.join(INBOX_FILE)).unwrap().starts_with("# Vault Tasks"));
    }

    #[test]
    fn due_dates() {
        let d = parse_date("2026-11-13");
        assert_eq!(replace_date_field("- [ ] A", &DUE_RE, "📅", d), "- [ ] A 📅 2026-11-13");
        assert_eq!(replace_date_field("- [ ] A 📅 2026-01-01 #x", &DUE_RE, "📅", d), "- [ ] A #x 📅 2026-11-13");
        assert_eq!(replace_date_field("- [ ] A 📅 2026-01-01", &DUE_RE, "📅", None), "- [ ] A");
        assert_eq!(replace_date_field("- [ ] A ^abc-1", &DUE_RE, "📅", d), "- [ ] A 📅 2026-11-13 ^abc-1");
    }

    #[test]
    fn recurrence() {
        let t = build_task("V", Path::new("/v"), Path::new("/v/a.md"), 0, "- [ ] Water the plants 🔁 every week").unwrap();
        assert_eq!(t.recurrence.as_deref(), Some("every week"));
        assert_eq!(t.description, "Water the plants");
    }

    #[test]
    fn block_ids() {
        let t = build_task("V", Path::new("/v"), Path::new("/v/a.md"), 0, "- [ ] Call Bob 📅 2026-07-15 ^vt-abc123").unwrap();
        assert_eq!(t.description, "Call Bob");
        assert_eq!(t.block_id.as_deref(), Some("vt-abc123"));
        assert_eq!(insert_field("- [x] A ^id1".into(), "✅ 2026-01-02"), "- [x] A ✅ 2026-01-02 ^id1");
        assert_ne!(new_block_id(), new_block_id());
    }

    #[test]
    fn renames() {
        assert_eq!(rename_body("Old name 📅 2026-07-15 ⏫ ^id1", "New"), "New 📅 2026-07-15 ⏫ ^id1");
        assert_eq!(rename_body("Old name", "New"), "New");
        assert_eq!(rename_body("Old 🔁 every week 📅 2026-01-01", "New"), "New 🔁 every week 📅 2026-01-01");
        assert_eq!(rename_body("Old ⏫ more text 📅 2026-01-01", "New"), "New ⏫ 📅 2026-01-01");

        let dir = std::env::temp_dir().join("vt-test3");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("n.md"), "- [ ] First #tag 📅 2026-01-01\n").unwrap();
        let t = scan_vault(&dir, &[]).pop().unwrap();
        rename_task(&t, "Second  #tag").unwrap();
        let t = scan_vault(&dir, &[]).pop().unwrap();
        let id = ensure_block_id(&t).unwrap();
        let out = fs::read_to_string(dir.join("n.md")).unwrap();
        assert_eq!(out, format!("- [ ] Second #tag 📅 2026-01-01 ^{id}\n"));
        let t = scan_vault(&dir, &[]).pop().unwrap();
        assert_eq!(t.block_id.as_deref(), Some(id.as_str()));
        assert_eq!(ensure_block_id(&t).unwrap(), id);
        complete_task(&t).unwrap();
        let t = scan_vault(&dir, &[]).pop().unwrap();
        assert!(t.completed);
        assert_eq!(t.block_id.as_deref(), Some(id.as_str()));
    }

    #[test]
    fn adds_to_named_note() {
        let dir = std::env::temp_dir().join("vt-test4");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        add_task_in(&dir, "Inbox/Capture", "Buy milk", parse_date("2026-12-01"), None, None).unwrap();
        let out = fs::read_to_string(dir.join("Inbox/Capture.md")).unwrap();
        assert!(out.starts_with("# Capture\n\n- [ ] Buy milk ➕ "));
        assert!(out.ends_with(" 📅 2026-12-01\n"));
        assert!(add_task_in(&dir, "../outside", "x", None, None, None).is_err());
    }

    #[test]
    fn deletes_only_the_line() {
        let dir = std::env::temp_dir().join("vt-test5");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("n.md");
        fs::write(&f, "# H\n- [ ] A\n- [ ] B 📅 2026-01-01\n- [ ] C\n").unwrap();
        let mut all = scan_vault(&dir, &[]);
        // Delete from the bottom up, as the app does, so line numbers stay valid.
        all.sort_by_key(|t| std::cmp::Reverse(t.line));
        for t in all.iter().filter(|t| t.description != "B") {
            delete_task(t).unwrap();
        }
        assert_eq!(fs::read_to_string(&f).unwrap(), "# H\n- [ ] B 📅 2026-01-01\n");
        // A task edited in Obsidian since the scan isn't deleted.
        let t = scan_vault(&dir, &[]).pop().unwrap();
        fs::write(&f, "# H\n- [ ] B changed\n").unwrap();
        assert!(delete_task(&t).is_err());
        assert_eq!(fs::read_to_string(&f).unwrap(), "# H\n- [ ] B changed\n");
    }

    #[test]
    fn moves_between_vaults() {
        let base = std::env::temp_dir().join("vt-test6");
        let _ = fs::remove_dir_all(&base);
        let (a, b) = (base.join("A"), base.join("B"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("n.md"), "# H\n- [ ] Keep\n\t- [/] Move me ⏫ 📅 2026-01-01 ^vt-abc\n").unwrap();
        let t = scan_vault(&a, &[]).into_iter().find(|t| t.description == "Move me").unwrap();
        move_task(&t, &b, INBOX_FILE).unwrap();
        assert_eq!(fs::read_to_string(a.join("n.md")).unwrap(), "# H\n- [ ] Keep\n");
        let moved = scan_vault(&b, &[]).pop().unwrap();
        assert_eq!(moved.raw_line, "- [/] Move me ⏫ 📅 2026-01-01 ^vt-abc");
        assert!(moved.in_progress);
        // A task changed since the scan isn't moved, and the other vault is left as it was.
        let t = scan_vault(&a, &[]).pop().unwrap();
        fs::write(a.join("n.md"), "# H\n- [ ] Keep edited\n").unwrap();
        let before = fs::read_to_string(b.join(INBOX_FILE)).unwrap();
        assert!(move_task(&t, &b, INBOX_FILE).is_err());
        assert_eq!(fs::read_to_string(b.join(INBOX_FILE)).unwrap(), before);
    }
}
