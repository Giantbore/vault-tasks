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
    // Open = space; in progress = '/'. Everything else (x, X, -, >, etc.) is treated as closed.
    if status != ' ' && status != '/' {
        return None;
    }
    if body.is_empty() {
        return None;
    }
    let mut due = None;
    let mut scheduled = None;
    let mut start = None;
    for c in DATE_RE.captures_iter(&body) {
        let d = parse_date(&c[2]);
        match &c[1] {
            "📅" => due = d,
            "⏳" => scheduled = d,
            "🛫" => start = d,
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

    // Clean description: strip metadata emoji + values.
    let mut desc = DATE_RE.replace_all(&body, "").to_string();
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
    lines[idx] = edit(&lines[idx])?;
    let mut out = lines.join(newline);
    if trailing {
        out.push_str(newline);
    }
    fs::write(&task.file, out).map_err(|e| e.to_string())
}

fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// Marks a task done using the Tasks plugin format (`[x]` + `✅ YYYY-MM-DD`).
pub fn complete_task(task: &Task) -> Result<(), String> {
    rewrite_line(task, |line| {
        let caps = TASK_RE.captures(line).ok_or("Line is no longer a task")?;
        Ok(format!(
            "{}x{}{} ✅ {}",
            &caps[1],
            &caps[3],
            caps[4].trim_end(),
            ymd(Local::now().date_naive())
        ))
    })
}

static SCHED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*⏳\uFE0F?\s*\d{4}-\d{2}-\d{2}").unwrap());

/// Sets or clears the Tasks plugin scheduled date (`⏳ YYYY-MM-DD`). Used for the Today list.
pub fn set_scheduled(task: &Task, date: Option<NaiveDate>) -> Result<(), String> {
    rewrite_line(task, |line| {
        let mut l = SCHED_RE.replace_all(line, "").trim_end().to_string();
        if let Some(d) = date {
            l.push_str(&format!(" ⏳ {}", ymd(d)));
        }
        Ok(l)
    })
}

pub const INBOX_FILE: &str = "Vault Tasks.md";

/// Appends a new task to the app-managed file at the root of the vault.
pub fn add_task(
    vault_root: &Path,
    text: &str,
    due: Option<NaiveDate>,
    scheduled: Option<NaiveDate>,
) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Task is empty".into());
    }
    let path = vault_root.join(INBOX_FILE);
    let mut content = fs::read_to_string(&path).unwrap_or_else(|_| {
        "# Vault Tasks\n\nTasks created in the Vault Tasks app.\n\n".to_string()
    });
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    let mut line = format!("- [ ] {} ➕ {}", text, ymd(Local::now().date_naive()));
    if let Some(d) = scheduled {
        line.push_str(&format!(" ⏳ {}", ymd(d)));
    }
    if let Some(d) = due {
        line.push_str(&format!(" 📅 {}", ymd(d)));
    }
    content.push_str(&line);
    content.push('\n');
    fs::write(&path, content).map_err(|e| e.to_string())
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
        assert!(build_task("V", Path::new("/v"), p, 0, "- [x] done").is_none());
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
    }

    #[test]
    fn star_and_add() {
        let dir = std::env::temp_dir().join("vt-test2");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let today = Local::now().date_naive();
        add_task(&dir, "New thing", parse_date("2026-12-01"), None).unwrap();
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
    fn recurrence() {
        let t = build_task("V", Path::new("/v"), Path::new("/v/a.md"), 0, "- [ ] Water the plants 🔁 every week").unwrap();
        assert_eq!(t.recurrence.as_deref(), Some("every week"));
        assert_eq!(t.description, "Water the plants");
    }
}
