mod capture;
mod fonts;
mod menu;
mod meta;
mod native;
mod natural_date;
mod tasks;
mod theme;

use chrono::{Datelike, Local, NaiveDate};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, Sense,
    Stroke, vec2,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use fonts::BodyFont;
use menu::Cmd;
use meta::{MetaStore, TaskMeta};
use natural_date::{end_of_week, next_monday};
use tasks::{Priority, Task};
use theme::*;

const RESCAN_EVERY: Duration = Duration::from_secs(15);

const UNDO_FOR: Duration = Duration::from_secs(8);

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default)]
enum DefaultDue {
    #[default]
    None,
    Today,
    DaysOut,
    EndOfWeek,
}

impl DefaultDue {
    const ALL: [DefaultDue; 4] = [DefaultDue::None, DefaultDue::Today, DefaultDue::DaysOut, DefaultDue::EndOfWeek];

    fn label(self) -> &'static str {
        match self {
            DefaultDue::None => "No due date",
            DefaultDue::Today => "Today",
            DefaultDue::DaysOut => "A number of days out",
            DefaultDue::EndOfWeek => "End of week (Friday)",
        }
    }
}

const SOUND_EXTS: [&str; 6] = ["aiff", "aif", "wav", "mp3", "m4a", "caf"];
const SYSTEM_SOUNDS: &str = "/System/Library/Sounds";

/// Your own sounds: ~/Library/Sounds (macOS also offers these as alert sounds).
fn user_sounds_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join("Library/Sounds"))
}

fn sounds_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| SOUND_EXTS.contains(&e.to_lowercase().as_str()))
        })
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

/// (your sounds, system sounds)
fn list_sounds() -> (Vec<String>, Vec<String>) {
    let mine = user_sounds_dir().map(|d| sounds_in(&d)).unwrap_or_default();
    (mine, sounds_in(std::path::Path::new(SYSTEM_SOUNDS)))
}

/// Finds a sound by name, preferring your own folder over the system one.
fn find_sound(name: &str) -> Option<PathBuf> {
    let dirs = user_sounds_dir().into_iter().chain([PathBuf::from(SYSTEM_SOUNDS)]);
    for dir in dirs {
        for ext in SOUND_EXTS {
            let p = dir.join(format!("{name}.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn play_sound(name: &str) {
    let Some(path) = find_sound(name) else { return };
    // Wait on a background thread so the process is reaped without blocking the UI.
    std::thread::spawn(move || {
        let _ = std::process::Command::new("afplay").arg(path).status();
    });
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Config {
    vaults: Vec<PathBuf>,
    #[serde(default)]
    group_by_file: bool,
    #[serde(default)]
    hide_future_start: bool,
    /// Paths (or path fragments) inside vaults to skip, e.g. "05 Journal/Recipes".
    #[serde(default)]
    excluded: Vec<String>,
    font: BodyFont,
    /// Index into fonts::SIZES
    text_size: usize,
    default_due: DefaultDue,
    default_due_days: u32,
    /// Sound played when a task is completed; None = off.
    chime: Option<String>,
    appearance: Appearance,
    /// Custom lists shown in the sidebar, in order. Membership is stored in each vault's .vault-tasks.json.
    lists: Vec<String>,
    /// Quick-capture shortcut, e.g. "ctrl+option+space".
    capture_shortcut: String,
    /// Vault quick-capture tasks go to (None: the first vault).
    capture_vault: Option<PathBuf>,
    /// Note in that vault they're added to (default: the same note as tasks added in the app).
    /// Created if it doesn't exist.
    capture_note: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            vaults: Vec::new(),
            group_by_file: false,
            hide_future_start: false,
            excluded: Vec::new(),
            font: BodyFont::default(),
            text_size: fonts::DEFAULT_SIZE,
            default_due: DefaultDue::None,
            default_due_days: 3,
            chime: Some("Glass".into()),
            appearance: Appearance::System,
            lists: Vec::new(),
            capture_shortcut: capture::DEFAULT_SHORTCUT.into(),
            capture_vault: None,
            capture_note: tasks::INBOX_FILE.into(),
        }
    }
}

impl Config {
    fn default_due_date(&self, today: NaiveDate) -> Option<NaiveDate> {
        match self.default_due {
            DefaultDue::None => None,
            DefaultDue::Today => Some(today),
            DefaultDue::DaysOut => Some(today + chrono::Days::new(self.default_due_days as u64)),
            DefaultDue::EndOfWeek => Some(end_of_week(today)),
        }
    }
}

impl Config {
    fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("vault-tasks")
            .join("config.json")
    }

    fn load() -> Self {
        if let Ok(s) = std::fs::read_to_string(Self::path())
            && let Ok(c) = serde_json::from_str(&s) {
                return c;
            }
        // First run: look for Obsidian vaults in iCloud.
        let mut c = Config::default();
        if let Some(home) = dirs::home_dir() {
            let icloud = home.join("Library/Mobile Documents/iCloud~md~obsidian/Documents");
            if let Ok(rd) = std::fs::read_dir(icloud) {
                let mut found: Vec<PathBuf> = rd
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.join(".obsidian").is_dir())
                    .collect();
                found.sort();
                c.vaults = found;
            }
        }
        c
    }

    fn save(&self) {
        let p = Self::path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, s);
        }
    }
}

#[derive(Clone, PartialEq)]
enum View {
    All,
    Today,
    Completed,
    Vault(String),
    List(String),
    Settings,
}

#[derive(Clone, Copy, PartialEq)]
enum Range {
    Today,
    Week,
    Month,
}

impl Range {
    const ALL: [Range; 3] = [Range::Today, Range::Week, Range::Month];

    fn label(self) -> &'static str {
        match self {
            Range::Today => "Today",
            Range::Week => "Past 7 days",
            Range::Month => "Past 30 days",
        }
    }

    /// Earliest done date included.
    fn since(self, today: NaiveDate) -> NaiveDate {
        let days = match self {
            Range::Today => 0,
            Range::Week => 6,
            Range::Month => 29,
        };
        today - chrono::Days::new(days)
    }
}

enum Action {
    Complete(Task),
    Reopen(Task),
    Star(Task, bool),
    SetDue(Task, Option<NaiveDate>),
    Open(Task),
    Rename(Task, String),
    SaveNotes(Task, String),
    ToggleList(Task, String, bool),
    /// The same change applied to several tasks (or one, from the keyboard).
    Bulk(BulkOp, Vec<Task>),
    /// Ask before deleting these tasks (shows the confirmation dialog).
    AskDelete(Vec<Task>),
}

/// A change that can be applied to many tasks at once.
#[derive(Clone)]
enum BulkOp {
    Due(Option<NaiveDate>),
    Complete,
    /// Add to list `to`; when moving out of a list view, also leave list `from`.
    MoveToList { to: String, from: Option<String> },
    /// Remove the task's line from its note.
    Delete,
}

/// Drag-and-drop payload: the tasks being dragged onto a list in the sidebar.
struct DraggedTasks(Vec<TaskKey>);

/// A click on a task row that changes the selection.
struct RowClick {
    key: TaskKey,
    /// Cmd held: add or remove this one task.
    toggle: bool,
    /// Shift held: select everything between the last clicked task and this one.
    range: bool,
}

/// Identifies a task row across frames: (file, line).
type TaskKey = (PathBuf, usize);

fn task_key(t: &Task) -> TaskKey {
    (t.file.clone(), t.line)
}

/// A task title being renamed in place.
struct TitleEdit {
    key: TaskKey,
    buf: String,
    focus: bool,
}

/// Notes being edited for the expanded task.
struct NotesEdit {
    key: TaskKey,
    buf: String,
    focus: bool,
}

/// A list name being typed in the sidebar: a new list (`from` = None) or a rename.
struct ListEdit {
    from: Option<String>,
    buf: String,
    focus: bool,
}

/// The last task checked off, for the Undo bar.
struct LastCompleted {
    /// (file, line, description) of each task checked off together.
    items: Vec<(PathBuf, usize, String)>,
    at: Instant,
}

struct NewTask {
    text: String,
    due: Option<NaiveDate>,
    vault: usize,
    /// The user picked or cleared the date by hand, so typed date phrases are ignored.
    due_manual: bool,
    /// Date found in the text last frame (to notice when it changes).
    parsed: Option<NaiveDate>,
    /// When the date chip was last updated from the text (drives a short highlight).
    flash_at: Option<Instant>,
}

impl NewTask {
    /// Clears the draft after adding, back to the default due date.
    fn reset(&mut self, default_due: Option<NaiveDate>) {
        self.text.clear();
        self.due = default_due;
        self.due_manual = false;
        self.parsed = None;
        self.flash_at = None;
    }
}

/// How long the date chip glows after a typed date phrase sets it.
const DATE_FLASH: f32 = 0.9;

struct App {
    cfg: Config,
    tasks: Vec<Task>,
    last_scan: Instant,
    scan_ms: u128,
    search: String,
    view: View,
    error: Option<String>,
    new_exclude: String,
    draft: NewTask,
    focus_new: bool,
    menu: menu::MenuBar,
    range: Range,
    last_completed: Option<LastCompleted>,
    _logos: [egui::TextureHandle; 2],
    /// Theme currently applied (None until the first frame).
    applied_dark: Option<bool>,
    /// When the user last refreshed by hand (drives the refresh button animation).
    refreshed_at: Option<Instant>,
    meta: MetaStore,
    title_edit: Option<TitleEdit>,
    notes_edit: Option<NotesEdit>,
    /// The task whose notes and lists panel is open.
    expanded: Option<TaskKey>,
    list_edit: Option<ListEdit>,
    /// The view shown last frame, to notice when the user switches views.
    shown_view: View,
    /// Selected tasks (Cmd-click, Shift-click, Cmd-A).
    selection: HashSet<TaskKey>,
    /// Where a Shift-click range starts: the last task clicked.
    anchor: Option<TaskKey>,
    /// The highlighted task that arrow keys move and T/M/W/Space act on when nothing is selected.
    cursor: Option<TaskKey>,
    /// Visible tasks in the order shown, from the last frame (for ranges and arrow keys).
    order: Vec<TaskKey>,
    /// Where each task row was drawn last frame, so a click on the row's empty space can be caught.
    row_rects: HashMap<TaskKey, egui::Rect>,
    /// Scroll the cursor row into view on the next frame (after an arrow key).
    scroll_to_cursor: bool,
    hotkey: capture::Hotkey,
    /// The quick-capture window, while it's open.
    capture: Option<CaptureWindow>,
    /// Shortcut being typed in Settings.
    shortcut_buf: String,
    /// Tasks waiting for the user to confirm deleting them.
    confirm_delete: Option<Vec<Task>>,
}

/// State of the floating quick-capture window.
struct CaptureWindow {
    text: String,
    /// Focus the window and its text field on the first frame.
    first_frame: bool,
    error: Option<String>,
}

/// What the user did in the quick-capture window this frame.
enum CaptureOutcome {
    Typing,
    Cancel,
    Save,
}

fn vault_name(p: &std::path::Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().to_string()
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        setup_style(&cc.egui_ctx);
        let mut cfg = Config::load();
        cfg.text_size = cfg.text_size.min(fonts::SIZES.len() - 1);
        fonts::apply(&cc.egui_ctx, cfg.font);
        cc.egui_ctx.set_zoom_factor(fonts::SIZES[cfg.text_size].1);
        let cfg_shortcut = cfg.capture_shortcut.clone();
        let mut app = App {
            view: if cfg.vaults.is_empty() { View::Settings } else { View::All },
            cfg,
            tasks: Vec::new(),
            last_scan: Instant::now(),
            scan_ms: 0,
            search: String::new(),
            error: None,
            new_exclude: String::new(),
            draft: NewTask { text: String::new(), due: None, vault: 0, due_manual: false, parsed: None, flash_at: None },
            focus_new: false,
            menu: menu::MenuBar::install(&cc.egui_ctx),
            range: Range::Week,
            last_completed: None,
            _logos: load_logos(&cc.egui_ctx),
            applied_dark: None,
            refreshed_at: None,
            meta: MetaStore::default(),
            title_edit: None,
            notes_edit: None,
            expanded: None,
            list_edit: None,
            shown_view: View::All,
            selection: HashSet::new(),
            anchor: None,
            cursor: None,
            order: Vec::new(),
            row_rects: HashMap::new(),
            scroll_to_cursor: false,
            hotkey: capture::Hotkey::new(&cc.egui_ctx, &cfg_shortcut),
            capture: None,
            shortcut_buf: cfg_shortcut.clone(),
            confirm_delete: None,
        };
        app.draft.due = app.cfg.default_due_date(Local::now().date_naive());
        app.rescan();
        app
    }

    fn set_text_size(&mut self, ctx: &egui::Context, size: usize) {
        self.cfg.text_size = size.min(fonts::SIZES.len() - 1);
        ctx.set_zoom_factor(fonts::SIZES[self.cfg.text_size].1);
        self.cfg.save();
    }

    fn handle_menu(&mut self, ctx: &egui::Context) {
        for cmd in self.menu.poll() {
            // Draw the result right away (menu clicks can arrive while the window is idle).
            ctx.request_repaint();
            match cmd {
                Cmd::Settings => self.view = View::Settings,
                Cmd::NewTask => {
                    if self.view == View::Settings {
                        self.view = View::All;
                    }
                    self.focus_new = true;
                }
                Cmd::Refresh => self.manual_refresh(),
                Cmd::ShowAll => self.view = View::All,
                Cmd::ShowToday => self.view = View::Today,
                Cmd::ShowCompleted => self.view = View::Completed,
                Cmd::GroupByFile => {
                    self.cfg.group_by_file = !self.cfg.group_by_file;
                    self.cfg.save();
                }
                Cmd::Bigger => self.set_text_size(ctx, self.cfg.text_size + 1),
                Cmd::Smaller => self.set_text_size(ctx, self.cfg.text_size.saturating_sub(1)),
                Cmd::ActualSize => self.set_text_size(ctx, fonts::DEFAULT_SIZE),
            }
        }
        self.menu.sync(self.cfg.group_by_file);
    }

    fn rescan(&mut self) {
        let start = Instant::now();
        let mut all = Vec::new();
        for v in &self.cfg.vaults {
            all.extend(tasks::scan_vault(v, &self.cfg.excluded));
        }
        all.sort_by(|a, b| {
            (a.due.is_none(), a.due, a.priority, &a.file, a.line)
                .cmp(&(b.due.is_none(), b.due, b.priority, &b.file, b.line))
        });
        self.tasks = all;
        self.meta = MetaStore::load(&self.cfg.vaults);
        // Lists used on another Mac (synced through the vault) show up here too.
        let mut added = false;
        for name in self.meta.list_names() {
            if !self.cfg.lists.contains(&name) {
                self.cfg.lists.push(name);
                added = true;
            }
        }
        if added {
            self.cfg.save();
        }
        self.last_scan = Instant::now();
        self.scan_ms = start.elapsed().as_millis();
    }

    /// Rescan because the user asked to, and show feedback on the refresh button.
    fn manual_refresh(&mut self) {
        self.rescan();
        self.refreshed_at = Some(Instant::now());
    }

    fn base_filter(&self, t: &Task, today: NaiveDate) -> bool {
        if self.cfg.hide_future_start && t.start.is_some_and(|s| s > today) {
            return false;
        }
        let q = self.search.trim().to_lowercase();
        q.is_empty()
            || t.description.to_lowercase().contains(&q)
            || t.rel_path().to_lowercase().contains(&q)
            || self.meta.get(t).is_some_and(|m| m.notes.to_lowercase().contains(&q))
    }

    fn tasks_for(&self, view: &View, today: NaiveDate) -> Vec<&Task> {
        if *view == View::Completed {
            return self.completed_in(self.range, today);
        }
        self.tasks
            .iter()
            .filter(|t| !t.completed && self.base_filter(t, today))
            .filter(|t| match view {
                View::All | View::Settings | View::Completed => true,
                View::Today => t.starred(today),
                View::Vault(v) => &t.vault == v,
                View::List(l) => self.meta.in_list(t, l),
            })
            .collect()
    }

    /// Completed tasks with a done date in the range, newest first.
    fn completed_in(&self, range: Range, today: NaiveDate) -> Vec<&Task> {
        let since = range.since(today);
        let q = self.search.trim().to_lowercase();
        let mut v: Vec<&Task> = self
            .tasks
            .iter()
            .filter(|t| t.completed && t.done.is_some_and(|d| d >= since))
            .filter(|t| {
                q.is_empty()
                    || t.description.to_lowercase().contains(&q)
                    || t.rel_path().to_lowercase().contains(&q)
            })
            .collect();
        v.sort_by(|a, b| (b.done, &a.file, a.line).cmp(&(a.done, &b.file, b.line)));
        v
    }

    fn undo_last(&mut self) {
        let Some(last) = self.last_completed.take() else { return };
        let mut missing = 0;
        for (file, line, description) in &last.items {
            let found = self
                .tasks
                .iter()
                .find(|t| t.completed && &t.file == file && t.line == *line && &t.description == description)
                .or_else(|| self.tasks.iter().find(|t| t.completed && &t.file == file && &t.description == description))
                .cloned();
            match found.map(|t| tasks::uncomplete_task(&t)) {
                Some(Ok(())) => {}
                _ => missing += 1,
            }
        }
        self.error = (missing > 0).then(|| format!("Couldn't undo {missing} task(s). Reopen them from Completed."));
        self.rescan();
    }

    fn run(&mut self, action: Action) {
        let today = Local::now().date_naive();
        let result = match &action {
            Action::Complete(t) => {
                // One task is a bulk edit of one: same chime, undo and error handling.
                self.run_bulk(BulkOp::Complete, std::slice::from_ref(t));
                return;
            }
            Action::Bulk(op, items) => {
                self.run_bulk(op.clone(), items);
                return;
            }
            Action::AskDelete(items) => {
                self.confirm_delete = (!items.is_empty()).then(|| items.clone());
                return;
            }
            Action::Reopen(t) => tasks::uncomplete_task(t),
            Action::Star(t, on) => tasks::set_scheduled(t, on.then_some(today)),
            Action::SetDue(t, d) => tasks::set_due(t, *d),
            Action::Open(t) => {
                open_in_obsidian(t);
                Ok(())
            }
            Action::Rename(t, text) => tasks::rename_task(t, text),
            Action::SaveNotes(t, notes) => {
                if notes.trim().is_empty() && self.meta.get(t).is_none() {
                    Ok(())
                } else {
                    tasks::ensure_block_id(t).and_then(|id| {
                        self.meta.update(&t.vault_root, &id, |m| m.notes = notes.trim_end().to_string())
                    })
                }
            }
            Action::ToggleList(t, list, on) => self.edit_lists(t, |lists| {
                lists.retain(|l| l != list);
                if *on {
                    lists.push(list.clone());
                }
            }),
        };
        match result {
            Ok(()) => {
                self.error = None;
                if !matches!(action, Action::Open(_)) {
                    self.rescan();
                }
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Changes which lists a task is in. Gives the task a block ID first if it has none.
    /// `impl FnOnce(...)` means "any function or closure that takes the list and is called once".
    fn edit_lists(&mut self, t: &Task, f: impl FnOnce(&mut Vec<String>)) -> Result<(), String> {
        let id = tasks::ensure_block_id(t)?;
        self.meta.update(&t.vault_root, &id, |m| f(&mut m.lists))
    }

    /// Applies one change to each task. A task that fails doesn't stop the others;
    /// the ones that failed are listed in the error message afterwards.
    fn run_bulk(&mut self, op: BulkOp, items: &[Task]) {
        let mut failed: Vec<String> = Vec::new();
        let mut completed = Vec::new();
        let mut items: Vec<&Task> = items.iter().collect();
        if matches!(op, BulkOp::Delete) {
            // Delete from the bottom of each file up, so removing one line doesn't
            // shift the line numbers of the others still to go.
            items.sort_by(|a, b| (&a.file, std::cmp::Reverse(a.line)).cmp(&(&b.file, std::cmp::Reverse(b.line))));
        }
        for t in items.iter().copied() {
            let result = match &op {
                BulkOp::Due(d) => tasks::set_due(t, *d),
                // Same as checking one off: recurring tasks open in Obsidian so the
                // Tasks plugin can create the next occurrence.
                BulkOp::Complete if t.recurrence.is_some() => {
                    open_in_obsidian(t);
                    Ok(())
                }
                BulkOp::Complete => {
                    tasks::complete_task(t).map(|()| completed.push((t.file.clone(), t.line, t.description.clone())))
                }
                BulkOp::Delete => tasks::delete_task(t).and_then(|()| match &t.block_id {
                    // Also drop the task's notes and lists, which nothing points to any more.
                    Some(id) => self.meta.update(&t.vault_root, id, |m| *m = TaskMeta::default()),
                    None => Ok(()),
                }),
                BulkOp::MoveToList { to, from } => self.edit_lists(t, |lists| {
                    if let Some(from) = from {
                        lists.retain(|l| l != from);
                    }
                    if !lists.contains(to) {
                        lists.push(to.clone());
                    }
                }),
            };
            if let Err(e) = result {
                failed.push(format!("\"{}\" ({e})", t.description));
            }
        }
        if !completed.is_empty() {
            if let Some(s) = &self.cfg.chime {
                play_sound(s);
            }
            self.last_completed = Some(LastCompleted { items: completed, at: Instant::now() });
        }
        self.error = match failed.len() {
            0 => None,
            1 if items.len() == 1 => Some(failed.remove(0)),
            n => Some(format!("Couldn't update {n} of {} tasks: {}", items.len(), failed.join(", "))),
        };
        self.rescan();
    }

    /// The tasks keyboard shortcuts and the action bar act on: the selection, or else the
    /// highlighted task, in the order they're shown.
    fn targets(&self) -> Vec<Task> {
        let keys: Vec<&TaskKey> = if self.selection.is_empty() {
            self.cursor.iter().collect()
        } else {
            self.order.iter().filter(|k| self.selection.contains(*k)).collect()
        };
        keys.into_iter()
            .filter_map(|k| self.tasks.iter().find(|t| !t.completed && &task_key(t) == k).cloned())
            .collect()
    }

    /// Updates the selection after a click on a task row.
    fn apply_click(&mut self, c: RowClick) {
        if c.range {
            let start = self.anchor.clone().or(self.cursor.clone()).unwrap_or(c.key.clone());
            let a = self.order.iter().position(|k| *k == start);
            let b = self.order.iter().position(|k| *k == c.key);
            if !c.toggle {
                self.selection.clear();
            }
            if let (Some(a), Some(b)) = (a, b) {
                // `a.min(b)..=a.max(b)` is an inclusive range, whichever direction the user went.
                self.selection.extend(self.order[a.min(b)..=a.max(b)].iter().cloned());
            }
        } else if c.toggle {
            // `insert` returns false if the key was already there: then remove it instead.
            if !self.selection.insert(c.key.clone()) {
                self.selection.remove(&c.key);
            }
            self.anchor = Some(c.key.clone());
        } else {
            self.selection.clear();
            self.anchor = Some(c.key.clone());
        }
        self.cursor = Some(c.key);
    }

    /// Keyboard shortcuts for the task list. Ignored while typing in a text field.
    fn list_keys(&mut self, ctx: &egui::Context, today: NaiveDate) -> Option<Action> {
        if ctx.egui_wants_keyboard_input()
            || self.confirm_delete.is_some()
            || matches!(self.view, View::Settings | View::Completed)
        {
            return None;
        }
        use egui::{Key, Modifiers};
        // `consume_key` returns true and removes the key press, so nothing else reacts to it.
        let pressed = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        if pressed(Modifiers::NONE, Key::Escape) {
            self.selection.clear();
            self.cursor = None;
            return None;
        }
        if pressed(Modifiers::COMMAND, Key::A) {
            self.selection = self.order.iter().cloned().collect();
            return None;
        }
        let step: isize = if pressed(Modifiers::NONE, Key::ArrowDown) {
            1
        } else if pressed(Modifiers::NONE, Key::ArrowUp) {
            -1
        } else {
            0
        };
        if step != 0 {
            if self.order.is_empty() {
                return None;
            }
            let i = self.cursor.as_ref().and_then(|c| self.order.iter().position(|k| k == c));
            let last = self.order.len() as isize - 1;
            let next = match i {
                Some(i) => (i as isize + step).clamp(0, last) as usize,
                None if step > 0 => 0,
                None => last as usize,
            };
            self.cursor = Some(self.order[next].clone());
            self.scroll_to_cursor = true;
            return None;
        }
        if pressed(Modifiers::NONE, Key::Backspace) || pressed(Modifiers::NONE, Key::Delete) {
            let items = self.targets();
            return (!items.is_empty()).then_some(Action::AskDelete(items));
        }
        let op = if pressed(Modifiers::NONE, Key::Space) {
            BulkOp::Complete
        } else if pressed(Modifiers::NONE, Key::T) {
            BulkOp::Due(Some(today))
        } else if pressed(Modifiers::NONE, Key::M) {
            BulkOp::Due(Some(today + chrono::Days::new(1)))
        } else if pressed(Modifiers::NONE, Key::W) {
            BulkOp::Due(Some(next_monday(today)))
        } else {
            return None;
        };
        let items = self.targets();
        // `then_some` turns a bool into an Option: Some(value) if true, None if false.
        (!items.is_empty()).then_some(Action::Bulk(op, items))
    }

    /// Asks before deleting tasks, and says exactly what will happen to the Markdown files.
    fn delete_dialog(&mut self, ctx: &egui::Context) -> Option<Action> {
        let items = self.confirm_delete.as_ref()?;
        let mut confirmed = false;
        let mut cancel = false;
        let n = items.len();
        let frame = Frame::popup(&ctx.global_style()).inner_margin(Margin::same(20)).corner_radius(CornerRadius::same(10));
        let modal = egui::Modal::new(egui::Id::unique("confirm_delete")).frame(frame).show(ctx, |ui| {
            ui.set_width(420.0);
            let title = if n == 1 { "Delete this task?".to_string() } else { format!("Delete {n} tasks?") };
            ui.label(RichText::new(title).size(17.0).family(fonts::bold()).color(text()));
            ui.add_space(6.0);
            let what = if n == 1 { "the task's line" } else { "each task's line" };
            ui.label(
                RichText::new(format!(
                    "This deletes {what} from its Markdown file in your vault. Nothing else in the file changes. \
                     Notes and lists for it in Vault Tasks are removed too. This can't be undone in Vault Tasks."
                ))
                .color(text()),
            );
            ui.add_space(8.0);
            for t in items.iter().take(6) {
                ui.label(RichText::new(format!("• {}", t.description)).color(text()));
                ui.label(
                    RichText::new(format!("    {} / {} (line {})", t.vault, t.rel_path(), t.line + 1))
                        .size(11.5)
                        .color(subtle()),
                );
            }
            if n > 6 {
                ui.label(RichText::new(format!("…and {} more", n - 6)).color(subtle()));
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let delete = egui::Button::new(RichText::new(if n == 1 { "Delete" } else { "Delete all" }).color(Color32::WHITE))
                        .fill(red());
                    if ui.add(delete).clicked() {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        });
        // Esc or a click outside the dialog cancels.
        if cancel || modal.should_close() {
            self.confirm_delete = None;
        }
        if confirmed {
            let items = self.confirm_delete.take()?;
            return Some(Action::Bulk(BulkOp::Delete, items));
        }
        None
    }

    /// Shown in place of the new task box while tasks are selected.
    fn action_bar(&mut self, ui: &mut egui::Ui, today: NaiveDate) -> Option<Action> {
        let mut op = None;
        Frame::new()
            .fill(blue().gamma_multiply(0.10))
            .stroke(Stroke::new(1.0, blue().gamma_multiply(0.4)))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(26.0);
                    let n = self.selection.len();
                    ui.label(RichText::new(format!("{n} selected")).family(fonts::bold()).color(text()));
                    ui.add_space(8.0);
                    for (label, d) in [
                        ("Today", today),
                        ("Tomorrow", today + chrono::Days::new(1)),
                        ("Next week", next_monday(today)),
                    ] {
                        if ui.button(label).on_hover_text(format!("Due {}", d.format("%a %b %-d"))).clicked() {
                            op = Some(BulkOp::Due(Some(d)));
                        }
                    }
                    let pick = ui.button("📅 Pick date…");
                    if let Some(picked) = date_menu(&pick, None, true, today) {
                        op = Some(BulkOp::Due(picked));
                    }
                    ui.menu_button("Move to list", |ui| {
                        if self.cfg.lists.is_empty() {
                            ui.label(RichText::new("Create a list with + next to LISTS first.").color(subtle()));
                        }
                        let from = match &self.view {
                            View::List(l) => Some(l.clone()),
                            _ => None,
                        };
                        for name in &self.cfg.lists {
                            if from.as_ref() != Some(name) && ui.button(name).clicked() {
                                op = Some(BulkOp::MoveToList { to: name.clone(), from: from.clone() });
                            }
                        }
                    });
                    if ui.button(RichText::new("✔ Complete").color(green())).clicked() {
                        op = Some(BulkOp::Complete);
                    }
                    if ui.button(RichText::new("Delete…").color(red())).clicked() {
                        op = Some(BulkOp::Delete);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new(RichText::new("✖ Clear").color(subtle())).frame(false))
                            .on_hover_text("Clear selection (Esc)")
                            .clicked()
                        {
                            self.selection.clear();
                        }
                    });
                });
            });
        ui.add_space(10.0);
        // `?` on an Option returns None from this function when nothing was clicked.
        let op = op?;
        Some(match op {
            // Deleting always asks first.
            BulkOp::Delete => Action::AskDelete(self.targets()),
            op => Action::Bulk(op, self.targets()),
        })
    }

    /// The vault quick-capture adds to: the one chosen in Settings, else the first vault.
    fn capture_root(&self) -> Option<PathBuf> {
        self.cfg
            .capture_vault
            .clone()
            .filter(|v| self.cfg.vaults.contains(v))
            .or_else(|| self.cfg.vaults.first().cloned())
    }

    /// Draws the floating quick-capture window (a second native window) while it's open.
    fn capture_window(&mut self, ctx: &egui::Context) {
        if self.hotkey.pressed() {
            self.capture = Some(CaptureWindow { text: String::new(), first_frame: true, error: None });
        }
        if self.capture.is_none() {
            return;
        }
        let today = Local::now().date_naive();
        // Read what's needed from `self` first: once `cap` borrows `self.capture` mutably
        // below, the rest of `self` can't be borrowed until the closure is done with it.
        let default_due = self.cfg.default_due_date(today);
        let target = self.capture_root().map(|v| format!("{} / {}", vault_name(&v), self.cfg.capture_note.trim()));
        let Some(cap) = &mut self.capture else { return };
        let id = egui::ViewportId::from_hash_of("quick_capture");
        let builder = egui::ViewportBuilder::default()
            .with_title("Quick Capture")
            .with_inner_size([560.0, 96.0])
            .with_resizable(false)
            .with_always_on_top()
            .with_active(true);
        let first = std::mem::take(&mut cap.first_frame);
        if first {
            // Bring the app and this window to the front so typing goes straight in.
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Focus);
        }
        // An "immediate" viewport is drawn right now, inside this frame, by the closure below.
        let outcome = ctx.show_viewport_immediate(id, builder, |ui, _class| {
            let mut outcome = CaptureOutcome::Typing;
            egui::CentralPanel::default()
                .frame(Frame::new().fill(main_bg()).inner_margin(Margin::symmetric(16, 12)))
                .show(ui, |ui| {
                    let parsed = natural_date::parse(&cap.text, today);
                    ui.horizontal(|ui| {
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut cap.text)
                                .hint_text("New task, e.g. \"Call the dentist tomorrow\"")
                                .font(FontId::proportional(16.0))
                                .desired_width(ui.available_width() - 110.0),
                        );
                        if first || !resp.has_focus() && cap.error.is_none() {
                            resp.request_focus();
                        }
                        let due = parsed.due.or(default_due);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| match due {
                            Some(d) => {
                                date_chip(ui, d, today);
                            }
                            None => {
                                ui.label(RichText::new("No date").size(12.0).color(subtle()));
                            }
                        });
                    });
                    ui.add_space(6.0);
                    let hint = match (&cap.error, &target) {
                        (Some(e), _) => RichText::new(format!("⚠ {e}")).color(red()),
                        (None, Some(t)) => RichText::new(format!("Enter adds to {t}  ·  Esc cancels")).color(subtle()),
                        (None, None) => RichText::new("Add a vault in Settings first.").color(red()),
                    };
                    ui.label(hint.size(12.0));
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        outcome = CaptureOutcome::Cancel;
                    } else if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !cap.text.trim().is_empty() {
                        outcome = CaptureOutcome::Save;
                    }
                });
            if ui.input(|i| i.viewport().close_requested()) {
                outcome = CaptureOutcome::Cancel;
            }
            outcome
        });
        match outcome {
            CaptureOutcome::Typing => {}
            CaptureOutcome::Cancel => {
                self.capture = None;
                native::restore_after_capture();
            }
            CaptureOutcome::Save => {
                let text = self.capture.as_ref().map(|c| c.text.clone()).unwrap_or_default();
                let parsed = natural_date::parse(&text, today);
                let result = match self.capture_root() {
                    Some(root) => tasks::add_task_in(
                        &root,
                        &self.cfg.capture_note,
                        &parsed.title,
                        parsed.due.or(default_due),
                        None,
                        None,
                    ),
                    None => Err("Add a vault folder in Settings first.".into()),
                };
                match result {
                    Ok(()) => {
                        self.capture = None;
                        native::restore_after_capture();
                        self.rescan();
                    }
                    Err(e) => {
                        if let Some(c) = &mut self.capture {
                            c.error = Some(e);
                        }
                    }
                }
            }
        }
    }

    fn add_draft(&mut self) {
        let Some(root) = self.cfg.vaults.get(self.draft.vault).cloned() else {
            self.error = Some("Add a vault folder in Settings first.".into());
            return;
        };
        let today = Local::now().date_naive();
        let sched = (self.view == View::Today).then_some(today);
        let list = match &self.view {
            View::List(l) => Some(l.clone()),
            _ => None,
        };
        let id = list.as_ref().map(|_| tasks::new_block_id());
        // A typed date phrase ("tomorrow", "nov 15") set the date, so leave it out of the
        // task's name. If the date was picked by hand, the text is kept exactly as typed.
        let text = if self.draft.due_manual {
            self.draft.text.clone()
        } else {
            natural_date::parse(&self.draft.text, today).title
        };
        let result = tasks::add_task(&root, &text, self.draft.due, sched, id.as_deref()).and_then(|()| {
            match (&id, &list) {
                (Some(id), Some(l)) => self.meta.update(&root, id, |m| m.lists.push(l.clone())),
                _ => Ok(()),
            }
        });
        match result {
            Ok(()) => {
                self.draft.reset(self.cfg.default_due_date(today));
                self.error = None;
                self.focus_new = true;
                self.rescan();
            }
            Err(e) => self.error = Some(e),
        }
    }

    // ---------- Sidebar ----------

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let today = Local::now().date_naive();
        ui.add_space(8.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("🔍  Quick Find")
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(8, 6)),
        );
        ui.add_space(14.0);

        let all = self.tasks_for(&View::All, today);
        let all_count = all.len();
        let overdue = all.iter().filter(|t| t.due.is_some_and(|d| d < today)).count();
        let today_list = self.tasks_for(&View::Today, today);
        let today_count = today_list.len();
        let today_overdue = today_list.iter().filter(|t| t.due.is_some_and(|d| d <= today)).count();

        if side_item(ui, Glyph::Logo, "All", all_count, overdue, self.view == View::All).clicked() {
            self.view = View::All;
        }
        if side_item(ui, Glyph::Text("★", star()), "Starred", today_count, today_overdue, self.view == View::Today).clicked() {
            self.view = View::Today;
        }
        ui.add_space(10.0);
        let done_today = self.completed_in(Range::Today, today).len();
        if side_item(ui, Glyph::Done, "Completed", done_today, 0, self.view == View::Completed)
            .on_hover_text("Completed today")
            .clicked()
        {
            self.view = View::Completed;
        }

        if self.cfg.vaults.len() > 1 {
            ui.add_space(18.0);
            ui.label(RichText::new("VAULTS").size(11.0).color(subtle()).family(fonts::bold()));
            ui.add_space(4.0);
            for v in self.cfg.vaults.clone() {
                let name = vault_name(&v);
                let count = self.tasks_for(&View::Vault(name.clone()), today).len();
                let sel = self.view == View::Vault(name.clone());
                if side_item(ui, Glyph::Text("◈", subtle()), &name, count, 0, sel).clicked() {
                    self.view = View::Vault(name);
                }
            }
        }

        self.sidebar_lists(ui, today);

        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(8.0);
            let row = vec2(ui.available_width(), 28.0);
            ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
                if ui
                    .add(egui::Button::new(RichText::new("⚙  Settings").color(text())).frame(false))
                    .clicked()
                {
                    self.view = View::Settings;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if refresh_button(ui, self.refreshed_at, self.tasks.len()).clicked() {
                        self.manual_refresh();
                    }
                });
            });
            ui.separator();
        });
    }

    fn sidebar_lists(&mut self, ui: &mut egui::Ui, today: NaiveDate) {
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("LISTS").size(11.0).color(subtle()).family(fonts::bold()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(egui::Button::new(RichText::new("+").size(15.0).color(subtle())).frame(false))
                    .on_hover_text("New list")
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    self.list_edit = Some(ListEdit { from: None, buf: String::new(), focus: true });
                }
            });
        });
        ui.add_space(4.0);
        let mut rename: Option<(String, Option<String>)> = None;
        let mut dropped: Option<(String, Vec<TaskKey>)> = None;
        for name in self.cfg.lists.clone() {
            if self.list_edit.as_ref().is_some_and(|e| e.from.as_deref() == Some(name.as_str())) {
                if let Some(r) = self.list_name_field(ui) {
                    rename = Some(r);
                }
                continue;
            }
            let count = self.tasks_for(&View::List(name.clone()), today).len();
            let sel = self.view == View::List(name.clone());
            let resp = side_item(ui, Glyph::List, &name, count, 0, sel);
            if resp.clicked() {
                self.view = View::List(name.clone());
            }
            // Drop target: outline the list while tasks are dragged over it.
            if resp.dnd_hover_payload::<DraggedTasks>().is_some() {
                ui.painter().rect_stroke(resp.rect, CornerRadius::same(6), Stroke::new(2.0, blue()), egui::StrokeKind::Inside);
            }
            if let Some(dragged) = resp.dnd_release_payload::<DraggedTasks>() {
                dropped = Some((name.clone(), dragged.0.clone()));
            }
            resp.context_menu(|ui| {
                if ui.button("Rename list…").clicked() {
                    self.list_edit = Some(ListEdit { from: Some(name.clone()), buf: name.clone(), focus: true });
                }
                if ui.button(RichText::new("Delete list").color(red())).on_hover_text("Tasks stay in your notes").clicked() {
                    rename = Some((name.clone(), None));
                }
            });
        }
        if self.list_edit.as_ref().is_some_and(|e| e.from.is_none())
            && let Some(r) = self.list_name_field(ui) {
                rename = Some(r);
            }
        if self.cfg.lists.is_empty() && self.list_edit.is_none() {
            let r = ui.add(
                egui::Label::new(RichText::new("+ New list").size(13.0).color(faint())).sense(Sense::click()),
            );
            if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.list_edit = Some(ListEdit { from: None, buf: String::new(), focus: true });
            }
        }
        if let Some((from, to)) = rename {
            self.apply_list_rename(&from, to);
        }
        if let Some((list, keys)) = dropped {
            let items: Vec<Task> = self.tasks.iter().filter(|t| keys.contains(&task_key(t))).cloned().collect();
            self.run(Action::Bulk(BulkOp::MoveToList { to: list, from: None }, items));
        }
    }

    /// Text field for a new or renamed list. Returns (old name, new name) when a rename
    /// is confirmed; creates new lists itself.
    fn list_name_field(&mut self, ui: &mut egui::Ui) -> Option<(String, Option<String>)> {
        let e = self.list_edit.as_mut()?;
        let resp = ui.add(
            egui::TextEdit::singleline(&mut e.buf)
                .hint_text("List name")
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(8, 5)),
        );
        if e.focus {
            resp.request_focus();
            e.focus = false;
        }
        if !resp.lost_focus() {
            return None;
        }
        let e = self.list_edit.take()?;
        let name = e.buf.trim().to_string();
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) || name.is_empty() {
            return None;
        }
        match e.from {
            None => {
                if !self.cfg.lists.contains(&name) {
                    self.cfg.lists.push(name.clone());
                    self.cfg.save();
                }
                self.view = View::List(name);
                None
            }
            Some(from) if from != name => Some((from, Some(name))),
            Some(_) => None,
        }
    }

    /// Renames a list, or deletes it when `to` is None. Tasks themselves are not touched.
    fn apply_list_rename(&mut self, from: &str, to: Option<String>) {
        if let Err(e) = self.meta.rename_list(from, to.as_deref()) {
            self.error = Some(e);
            return;
        }
        let pos = self.cfg.lists.iter().position(|l| l == from);
        match (&to, pos) {
            (Some(n), Some(i)) if !self.cfg.lists.contains(n) => self.cfg.lists[i] = n.clone(),
            (_, Some(i)) => {
                self.cfg.lists.remove(i);
            }
            _ => {}
        }
        self.cfg.save();
        if self.view == View::List(from.to_string()) {
            self.view = match to {
                Some(n) => View::List(n),
                None => View::All,
            };
        }
    }

    // ---------- Main list ----------

    fn header(&mut self, ui: &mut egui::Ui, today: NaiveDate) {
        let (icon, title, sub) = match &self.view {
            View::All => (Glyph::Logo, "All Tasks".to_string(), None),
            View::Today => (Glyph::Text("★", star()), "Starred".to_string(), Some(today.format("%A, %B %-d").to_string())),
            View::Completed => (Glyph::Done, "Completed".to_string(), None),
            View::Vault(v) => (Glyph::Text("◈", subtle()), v.clone(), None),
            View::List(l) => (Glyph::List, l.clone(), None),
            View::Settings => (Glyph::Text("⚙", subtle()), "Settings".to_string(), None),
        };
        ui.horizontal(|ui| {
            // Nudge the icon down so it centers on the serif title's letters.
            let (rect, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
            paint_glyph(ui.painter(), rect.center() + vec2(0.0, 3.0), 24.0, icon);
            ui.label(RichText::new(title).size(28.0).family(fonts::heading()).color(text()));
        });
        if let Some(s) = sub {
            ui.label(RichText::new(s).size(13.0).color(subtle()));
        }
        ui.add_space(14.0);
    }

    fn new_task_row(&mut self, ui: &mut egui::Ui, today: NaiveDate) {
        if let View::Vault(name) = &self.view
            && let Some(i) = self.cfg.vaults.iter().position(|v| &vault_name(v) == name) {
                self.draft.vault = i;
            }
        if self.draft.vault >= self.cfg.vaults.len() {
            self.draft.vault = 0;
        }
        // Natural language date: look for a date phrase in the text every frame, and
        // update the date chip when it changes (unless the date was picked by hand).
        let parsed = natural_date::parse(&self.draft.text, today);
        if !self.draft.due_manual && parsed.due != self.draft.parsed {
            self.draft.due = parsed.due.or(self.cfg.default_due_date(today));
            self.draft.flash_at = parsed.due.map(|_| Instant::now());
            self.draft.parsed = parsed.due;
        }
        let highlight_phrase = !self.draft.due_manual;
        Frame::new()
            .fill(field_bg())
            .stroke(Stroke::new(1.0, field_stroke()))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    const ROW_H: f32 = 26.0;
                    ui.set_min_height(ROW_H);
                    // Painted plus sign so it centers exactly on the text line.
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, ROW_H), Sense::hover());
                    let c = r.center();
                    let stroke = Stroke::new(1.6, blue());
                    ui.painter().line_segment([c - vec2(5.5, 0.0), c + vec2(5.5, 0.0)], stroke);
                    ui.painter().line_segment([c - vec2(0.0, 5.5), c + vec2(0.0, 5.5)], stroke);
                    let hint = match &self.view {
                        View::Today => "New starred task".to_string(),
                        View::List(l) => format!("New task in {l}"),
                        _ => "New task".to_string(),
                    };
                    // Draws the text with the date phrase in blue, so it's clear which words
                    // become the due date (and will be left out of the task's name).
                    // `move` copies `today` and `highlight_phrase` into the closure.
                    let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                        let text = buf.as_str();
                        let font = egui::TextStyle::Body.resolve(ui.style());
                        let plain = egui::text::TextFormat::simple(font.clone(), theme::text());
                        let mut job = egui::text::LayoutJob::default();
                        let phrase = natural_date::parse(text, today).phrase.filter(|_| highlight_phrase);
                        match phrase {
                            Some(r) => {
                                let mut date = egui::text::TextFormat::simple(font, blue());
                                date.background = blue().gamma_multiply(0.12);
                                job.append(&text[..r.start], 0.0, plain.clone());
                                job.append(&text[r.clone()], 0.0, date);
                                job.append(&text[r.end..], 0.0, plain);
                            }
                            None => job.append(text, 0.0, plain),
                        }
                        job.wrap.max_width = wrap_width;
                        ui.fonts_mut(|f| f.layout_job(job))
                    };
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.draft.text)
                            .layouter(&mut layouter)
                            .hint_text(hint.as_str())
                            .frame(Frame::NONE)
                            .font(egui::TextStyle::Body)
                            .align(Align2::LEFT_CENTER)
                            .min_size(vec2(0.0, ROW_H))
                            .margin(Margin::symmetric(4, 0))
                            .desired_width(ui.available_width() - 230.0),
                    );
                    if self.focus_new {
                        resp.request_focus();
                        self.focus_new = false;
                    }
                    let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add_enabled(!self.draft.text.trim().is_empty(), egui::Button::new("Add")).clicked()
                            || (submit && !self.draft.text.trim().is_empty())
                        {
                            self.add_draft();
                        }
                        if self.cfg.vaults.len() > 1 && !matches!(self.view, View::Vault(_)) {
                            egui::ComboBox::from_id_salt("new_vault")
                                .width(80.0)
                                .selected_text(vault_name(&self.cfg.vaults[self.draft.vault]))
                                .show_ui(ui, |ui| {
                                    for (i, v) in self.cfg.vaults.iter().enumerate() {
                                        ui.selectable_value(&mut self.draft.vault, i, vault_name(v));
                                    }
                                });
                        }
                        // The due date chip opens the same date menu as a task's chip:
                        // quick picks, a calendar and "Remove due date".
                        let chip = match self.draft.due {
                            Some(d) => date_chip(ui, d, today),
                            None => ui
                                .add(egui::Button::new(RichText::new("📅 Due").color(subtle())).frame(false))
                                .on_hover_text("Set a due date"),
                        };
                        if let Some(picked) = date_menu(&chip, self.draft.due, self.draft.due.is_some(), today) {
                            // Picked by hand: this wins over date phrases typed in the text.
                            self.draft.due = picked;
                            self.draft.due_manual = true;
                            self.draft.flash_at = None;
                        }
                        // Short blue glow around the chip after a typed phrase changed it.
                        if let Some(t) = self.draft.flash_at {
                            let age = t.elapsed().as_secs_f32();
                            if age < DATE_FLASH {
                                let alpha = 1.0 - age / DATE_FLASH;
                                ui.painter().rect(
                                    chip.rect.expand(2.0),
                                    CornerRadius::same(6),
                                    blue().gamma_multiply(0.18 * alpha),
                                    Stroke::new(1.5, blue().gamma_multiply(alpha)),
                                    egui::StrokeKind::Outside,
                                );
                                ui.ctx().request_repaint();
                            }
                        }
                    });
                });
            });
        ui.add_space(10.0);
    }

    fn task_list(&mut self, ui: &mut egui::Ui, today: NaiveDate) -> Option<Action> {
        let view = self.view.clone();
        // Cloned so rows can borrow the edit state mutably.
        let visible: Vec<Task> = self.tasks_for(&view, today).into_iter().cloned().collect();
        let visible: Vec<&Task> = visible.iter().collect();
        let mut action = None;

        if visible.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                let msg = match view {
                    View::Today => "Nothing starred. Click ☆ on any task to add it here.",
                    View::List(_) => "No tasks in this list yet. Add one above, or right-click any task to add it here.",
                    _ => "No open tasks 🎉",
                };
                ui.label(RichText::new(msg).size(14.0).color(subtle()));
            });
            return None;
        }

        let mut groups: Vec<(String, Color32, Vec<&Task>)> = Vec::new();
        if self.cfg.group_by_file {
            let mut sorted = visible.clone();
            sorted.sort_by(|a, b| (&a.vault, a.rel_path(), a.line).cmp(&(&b.vault, b.rel_path(), b.line)));
            for t in sorted {
                let key = format!("{} / {}", t.vault, t.rel_path().trim_end_matches(".md"));
                match groups.last_mut() {
                    Some((k, _, v)) if *k == key => v.push(t),
                    _ => groups.push((key, blue(), vec![t])),
                }
            }
        } else if view == View::Today {
            groups.push((String::new(), blue(), visible.clone()));
        } else {
            for name in ["Overdue", "Today", "Next 7 Days", "Later", "No Due Date"] {
                let items: Vec<&Task> = visible.iter().copied().filter(|t| date_bucket(t.due, today) == name).collect();
                if !items.is_empty() {
                    let c = if name == "Overdue" { red() } else { blue() };
                    groups.push((name.to_string(), c, items));
                }
            }
        }

        // Remember the order rows are shown in, and drop selected tasks that are gone
        // (completed, filtered out…).
        self.order = groups.iter().flat_map(|(_, _, items)| items.iter().map(|t| task_key(t))).collect();
        let shown: HashSet<&TaskKey> = self.order.iter().collect();
        self.selection.retain(|k| shown.contains(k));
        if self.cursor.as_ref().is_some_and(|c| !shown.contains(c)) {
            self.cursor = None;
        }

        let show_file = !self.cfg.group_by_file;
        let group_by_file = self.cfg.group_by_file;
        let mut click = None;
        let mut row = RowUi {
            meta: &self.meta,
            lists: &self.cfg.lists,
            title_edit: &mut self.title_edit,
            notes_edit: &mut self.notes_edit,
            expanded: &mut self.expanded,
            selection: &self.selection,
            cursor: self.cursor.as_ref(),
            row_rects: &mut self.row_rects,
            // `take` reads the flag and resets it to false in one step.
            scroll_to_cursor: std::mem::take(&mut self.scroll_to_cursor),
            click: &mut click,
        };
        list_scroll(ui, |ui| {
            for (name, color, items) in &groups {
                if !name.is_empty() {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(name).size(15.0).family(fonts::bold()).color(*color));
                        if name == "Overdue" && !group_by_file {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add_space(8.0);
                                let link = ui
                                    .add(
                                        egui::Label::new(RichText::new("Reschedule all to today").size(12.5).color(blue()))
                                            .sense(Sense::click()),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                                if link.clicked() {
                                    let all: Vec<Task> = items.iter().map(|t| (*t).clone()).collect();
                                    action = Some(Action::Bulk(BulkOp::Due(Some(today)), all));
                                }
                            });
                        }
                    });
                    let r = ui.available_rect_before_wrap();
                    ui.painter().hline(r.x_range(), r.top() + 2.0, Stroke::new(1.0, rule()));
                    ui.add_space(8.0);
                }
                for t in items {
                    if let Some(a) = task_row(ui, t, today, show_file, &mut row) {
                        action = Some(a);
                    }
                }
            }
            ui.add_space(20.0);
        });
        if let Some(c) = click {
            self.apply_click(c);
        }
        action
    }

    fn completed_list(&mut self, ui: &mut egui::Ui, today: NaiveDate) -> Option<Action> {
        let visible = self.completed_in(self.range, today);
        if visible.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                let msg = match self.range {
                    Range::Today => "Nothing completed today yet.",
                    _ => "Nothing completed in this period.",
                };
                ui.label(RichText::new(msg).size(14.0).color(subtle()));
            });
            return None;
        }
        let mut action = None;
        list_scroll(ui, |ui| {
            let mut current: Option<NaiveDate> = None;
            for t in visible {
                if t.done != current {
                    current = t.done;
                    let d = t.done.unwrap_or(today);
                    let days = (today - d).num_days();
                    let name = match days {
                        0 => "Today".to_string(),
                        1 => "Yesterday".to_string(),
                        _ => d.format("%A, %B %-d").to_string(),
                    };
                    ui.add_space(10.0);
                    ui.label(RichText::new(name).size(15.0).family(fonts::bold()).color(green()));
                    let r = ui.available_rect_before_wrap();
                    ui.painter().hline(r.x_range(), r.top() + 2.0, Stroke::new(1.0, rule()));
                    ui.add_space(8.0);
                }
                if let Some(a) = completed_row(ui, t) {
                    action = Some(a);
                }
            }
            ui.add_space(20.0);
        });
        action
    }

    // ---------- Settings ----------

    fn settings(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        list_scroll(ui, |ui| {
            section(ui, "Vault folders", "Each folder is scanned for .md files with open tasks.");
            let mut remove = None;
            for (i, v) in self.cfg.vaults.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.small_button("✖").on_hover_text("Remove").clicked() {
                        remove = Some(i);
                    }
                    ui.label(vault_name(v)).on_hover_text(v.to_string_lossy());
                });
            }
            if let Some(i) = remove {
                self.cfg.vaults.remove(i);
                changed = true;
            }
            if ui.button("+ Add vault folder…").clicked()
                && let Some(p) = rfd::FileDialog::new().pick_folder()
                    && !self.cfg.vaults.contains(&p) {
                        self.cfg.vaults.push(p);
                        changed = true;
                    }

            ui.add_space(20.0);
            section(ui, "Excluded folders", "Any file whose path contains one of these is skipped (e.g. Recipes, Grocery lists, etc.).");
            let mut remove_ex = None;
            for (i, e) in self.cfg.excluded.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.small_button("✖").on_hover_text("Remove").clicked() {
                        remove_ex = Some(i);
                    }
                    ui.label(e);
                });
            }
            if let Some(i) = remove_ex {
                self.cfg.excluded.remove(i);
                changed = true;
            }
            ui.horizontal(|ui| {
                let resp = ui.add(egui::TextEdit::singleline(&mut self.new_exclude).hint_text("Folder or path").desired_width(220.0));
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button("Add").clicked() || enter) && !self.new_exclude.trim().is_empty() {
                    self.cfg.excluded.push(self.new_exclude.trim().trim_matches('/').to_string());
                    self.new_exclude.clear();
                    changed = true;
                }
            });

            ui.add_space(20.0);
            section(ui, "New tasks", "");
            let today = Local::now().date_naive();
            let before = (self.cfg.default_due, self.cfg.default_due_days);
            ui.horizontal(|ui| {
                ui.label("Default due date");
                egui::ComboBox::from_id_salt("default_due")
                    .width(200.0)
                    .selected_text(self.cfg.default_due.label())
                    .show_ui(ui, |ui| {
                        for d in DefaultDue::ALL {
                            ui.selectable_value(&mut self.cfg.default_due, d, d.label());
                        }
                    });
                if self.cfg.default_due == DefaultDue::DaysOut {
                    ui.add(egui::DragValue::new(&mut self.cfg.default_due_days).range(1..=365).speed(0.2));
                    ui.label(if self.cfg.default_due_days == 1 { "day" } else { "days" });
                }
            });
            if let Some(d) = self.cfg.default_due_date(today) {
                ui.label(RichText::new(format!("New tasks will be due {}.", d.format("%A, %b %-d"))).size(12.0).color(subtle()));
            }
            if before != (self.cfg.default_due, self.cfg.default_due_days) {
                if self.draft.text.trim().is_empty() {
                    self.draft.due = self.cfg.default_due_date(today);
                }
                changed = true;
            }

            ui.add_space(20.0);
            section(
                ui,
                "Quick capture",
                "Press the shortcut anywhere on your Mac to add a task without opening the app. Dates like \"tomorrow\" or \"fri\" work here too.",
            );
            ui.horizontal(|ui| {
                ui.label("Shortcut");
                let resp = ui.add(egui::TextEdit::singleline(&mut self.shortcut_buf).desired_width(180.0));
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let differs = self.shortcut_buf.trim() != self.cfg.capture_shortcut;
                if (ui.add_enabled(differs, egui::Button::new("Apply")).clicked() || (enter && differs))
                    && self.hotkey.set(self.shortcut_buf.trim()).is_ok()
                {
                    self.cfg.capture_shortcut = self.shortcut_buf.trim().to_string();
                    changed = true;
                }
                ui.label(RichText::new(capture::pretty(&self.cfg.capture_shortcut)).color(subtle()));
            });
            ui.label(
                RichText::new("Write it like ctrl+option+space or cmd+shift+k. macOS doesn't need any extra permission for this.")
                    .size(12.0)
                    .color(subtle()),
            );
            if let Some(e) = &self.hotkey.error {
                ui.label(RichText::new(format!("⚠ {e}")).color(red()));
            }
            ui.horizontal(|ui| {
                ui.label("Add tasks to");
                let current = self.capture_root();
                let label = current.as_deref().map(vault_name).unwrap_or_else(|| "No vault".into());
                egui::ComboBox::from_id_salt("capture_vault").width(140.0).selected_text(label).show_ui(ui, |ui| {
                    for v in &self.cfg.vaults {
                        if ui.selectable_label(current.as_ref() == Some(v), vault_name(v)).clicked() {
                            self.cfg.capture_vault = Some(v.clone());
                            changed = true;
                        }
                    }
                });
                ui.label("/");
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut self.cfg.capture_note).hint_text(tasks::INBOX_FILE).desired_width(180.0))
                    .lost_focus();
            });

            ui.add_space(20.0);
            section(ui, "Sound", "");
            ui.horizontal(|ui| {
                ui.label("Completion chime");
                let before = self.cfg.chime.clone();
                egui::ComboBox::from_id_salt("chime")
                    .width(140.0)
                    .selected_text(self.cfg.chime.as_deref().unwrap_or("Off"))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.cfg.chime, None, "Off");
                        let (mine, system) = list_sounds();
                        if !mine.is_empty() {
                            ui.separator();
                            ui.label(RichText::new("Your sounds").size(11.0).color(subtle()));
                            for s in mine {
                                ui.selectable_value(&mut self.cfg.chime, Some(s.clone()), s);
                            }
                            ui.separator();
                            ui.label(RichText::new("macOS").size(11.0).color(subtle()));
                        }
                        for s in system {
                            ui.selectable_value(&mut self.cfg.chime, Some(s.clone()), s);
                        }
                    });
                if self.cfg.chime != before {
                    if let Some(s) = &self.cfg.chime {
                        play_sound(s);
                    }
                    changed = true;
                }
                if let Some(s) = &self.cfg.chime
                    && ui.button("▶ Play").clicked() {
                        play_sound(s);
                    }
            });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Add your own: drop .aiff, .wav, .mp3, .m4a or .caf files into ~/Library/Sounds.")
                        .size(12.0)
                        .color(subtle()),
                );
                if ui.small_button("Open folder").clicked()
                    && let Some(dir) = user_sounds_dir() {
                        let _ = std::fs::create_dir_all(&dir);
                        let _ = std::process::Command::new("open").arg(dir).spawn();
                    }
            });

            ui.add_space(20.0);
            section(ui, "Display", "");
            ui.horizontal(|ui| {
                ui.label("Appearance");
                for a in Appearance::ALL {
                    if ui.selectable_label(self.cfg.appearance == a, a.label()).clicked() && self.cfg.appearance != a {
                        self.cfg.appearance = a;
                        self.cfg.save();
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Font");
                let before = self.cfg.font;
                egui::ComboBox::from_id_salt("font")
                    .width(200.0)
                    .selected_text(self.cfg.font.label())
                    .show_ui(ui, |ui| {
                        for f in BodyFont::ALL {
                            ui.selectable_value(&mut self.cfg.font, f, f.label());
                        }
                    });
                if self.cfg.font != before {
                    fonts::apply(ui.ctx(), self.cfg.font);
                    self.cfg.save();
                }
            });
            ui.horizontal(|ui| {
                ui.label("Text size");
                let mut size = self.cfg.text_size;
                ui.add(egui::Slider::new(&mut size, 0..=fonts::SIZES.len() - 1).show_value(false));
                ui.label(RichText::new(fonts::SIZES[size].0).color(subtle()));
                if size != self.cfg.text_size {
                    let ctx = ui.ctx().clone();
                    self.set_text_size(&ctx, size);
                }
            });
            ui.label(RichText::new("Shortcuts: ⌘= bigger, ⌘- smaller, ⌘0 normal.").size(12.0).color(subtle()));
            ui.add_space(6.0);
            changed |= ui.checkbox(&mut self.cfg.group_by_file, "Group tasks by file instead of due date").changed();
            changed |= ui.checkbox(&mut self.cfg.hide_future_start, "Hide tasks whose start date (🛫) is in the future").changed();

            ui.add_space(20.0);
            section(
                ui,
                "Obsidian plugins",
                "Install these from Obsidian → Settings → Community plugins → Browse, then enable them, in each vault.",
            );
            let plugins = [
                (TASKS_PLUGIN, "Tasks", "Recommended. Shows and queries these tasks in Obsidian and creates the next copy of 🔁 recurring tasks."),
                (ADV_URI_PLUGIN, "Advanced URI", "Optional. Lets a click open the note at the task's line."),
            ];
            for (id, name, why) in plugins {
                ui.add_space(4.0);
                ui.label(RichText::new(name).family(fonts::bold()).color(text()));
                ui.label(RichText::new(why).size(12.0).color(subtle()));
                for v in &self.cfg.vaults {
                    let (mark, state) = match plugin_state(v, id) {
                        PluginState::Enabled => (Glyph::Done, "installed and enabled"),
                        PluginState::InstalledOff => (Glyph::Partial, "installed but not enabled"),
                        PluginState::Missing => (Glyph::Missing, "not installed"),
                    };
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        glyph(ui, 13.0, mark);
                        ui.label(RichText::new(format!("{}: {state}", vault_name(v))).color(text()));
                    });
                }
            }

            ui.add_space(20.0);
            section(ui, "About", "");
            ui.label(RichText::new(format!(
                "New tasks are added to \"{}\" at the root of the chosen vault.\nStarring a task sets its ⏳ scheduled date to today (Tasks plugin format).\n{} open tasks, last scan took {} ms.",
                tasks::INBOX_FILE,
                self.tasks.iter().filter(|t| !t.completed).count(),
                self.scan_ms
            )).color(subtle()));
        });
        if changed {
            self.cfg.save();
            if let View::Vault(v) = &self.view
                && !self.cfg.vaults.iter().any(|p| &vault_name(p) == v) {
                    self.view = View::All;
                }
            self.rescan();
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str, sub: &str) {
    ui.label(RichText::new(title).size(15.0).family(fonts::bold()).color(blue()));
    if !sub.is_empty() {
        ui.label(RichText::new(sub).color(subtle()));
    }
    ui.add_space(6.0);
}

/// Icons used in the sidebar and page headers.
#[derive(Clone, Copy)]
enum Glyph<'a> {
    Text(&'a str, Color32),
    /// The app logo (assets/icon-small.png).
    Logo,
    /// Green circle with a white check.
    Done,
    /// Red circle with a white ✕.
    Missing,
    /// Amber circle with a white dot.
    Partial,
    /// Bulleted list icon (custom lists).
    List,
}

static LOGO_DARK: std::sync::OnceLock<egui::TextureId> = std::sync::OnceLock::new();
static LOGO_LIGHT: std::sync::OnceLock<egui::TextureId> = std::sync::OnceLock::new();

fn load_texture(ctx: &egui::Context, name: &str, png: &[u8]) -> egui::TextureHandle {
    let icon = eframe::icon_data::from_png_bytes(png).expect("logo png");
    let img = egui::ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
    let opts = egui::TextureOptions {
        mipmap_mode: Some(egui::TextureFilter::Linear),
        ..egui::TextureOptions::LINEAR
    };
    ctx.load_texture(name, img, opts)
}

/// Loads the in-app logos (dark and light versions). Keep the handles alive.
fn load_logos(ctx: &egui::Context) -> [egui::TextureHandle; 2] {
    let dark = load_texture(ctx, "logo-dark", include_bytes!("../assets/glyph.png"));
    let light = load_texture(ctx, "logo-light", include_bytes!("../assets/glyph-light.png"));
    let _ = LOGO_DARK.set(dark.id());
    let _ = LOGO_LIGHT.set(light.id());
    [dark, light]
}

/// Dock icon for the theme.
fn dock_icon(dark: bool) -> std::sync::Arc<egui::IconData> {
    let png: &[u8] = if dark {
        include_bytes!("../assets/icon.png")
    } else {
        include_bytes!("../assets/icon-light.png")
    };
    std::sync::Arc::new(eframe::icon_data::from_png_bytes(png).expect("icon png"))
}

/// Paints a glyph centered at `c`, `size` points tall.
fn paint_glyph(p: &egui::Painter, c: egui::Pos2, size: f32, g: Glyph) {
    match g {
        Glyph::Text(t, color) => {
            p.text(c, Align2::CENTER_CENTER, t, FontId::proportional(size), color);
        }
        Glyph::Logo => {
            let logo = if is_dark() { LOGO_DARK.get() } else { LOGO_LIGHT.get() };
            if let Some(id) = logo {
                let r = egui::Rect::from_center_size(c, vec2(size, size));
                p.image(*id, r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
        }
        Glyph::Done => {
            let r = size * 0.5;
            p.circle_filled(c, r, green());
            let pts = vec![c + vec2(-0.45 * r, 0.02 * r), c + vec2(-0.12 * r, 0.34 * r), c + vec2(0.46 * r, -0.30 * r)];
            p.add(egui::Shape::line(pts, Stroke::new((r * 0.24).max(1.5), Color32::WHITE)));
        }
        Glyph::Missing => {
            let r = size * 0.5;
            p.circle_filled(c, r, red());
            let k = r * 0.38;
            let st = Stroke::new((r * 0.22).max(1.5), Color32::WHITE);
            p.line_segment([c + vec2(-k, -k), c + vec2(k, k)], st);
            p.line_segment([c + vec2(-k, k), c + vec2(k, -k)], st);
        }
        Glyph::Partial => {
            let r = size * 0.5;
            p.circle_filled(c, r, amber());
            p.circle_filled(c, r * 0.3, Color32::WHITE);
        }
        Glyph::List => {
            let s = size / 16.0;
            let color = blue();
            for i in -1..=1 {
                let y = c.y + i as f32 * 4.5 * s;
                p.circle_filled(egui::pos2(c.x - 5.5 * s, y), 1.4 * s, color);
                p.line_segment([egui::pos2(c.x - 2.5 * s, y), egui::pos2(c.x + 6.5 * s, y)], Stroke::new(1.6 * s, color));
            }
        }
    }
}

/// Allocates space in a layout and paints a glyph there.
fn glyph(ui: &mut egui::Ui, size: f32, g: Glyph) {
    let (rect, _) = ui.allocate_exact_size(vec2(size * 1.25, size * 1.25), Sense::hover());
    paint_glyph(ui.painter(), rect.center(), size, g);
}

/// How long the refresh icon spins, then how long the "Updated" check stays.
const REFRESH_SPIN: f32 = 0.6;
const REFRESH_DONE: f32 = 2.0;

/// Painted refresh icon (a circular arrow), so it centers exactly in the footer row.
/// After a manual refresh it spins briefly, then shows a green check and "Updated".
fn refresh_button(ui: &mut egui::Ui, refreshed_at: Option<Instant>, task_count: usize) -> egui::Response {
    let since = refreshed_at.map(|t| t.elapsed().as_secs_f32());
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), hover());
    }
    let c = rect.center();
    match since {
        Some(s) if (REFRESH_SPIN..REFRESH_SPIN + REFRESH_DONE).contains(&s) => {
            paint_glyph(p, c, 15.0, Glyph::Done);
            ui.label(RichText::new("Updated").size(12.5).color(subtle()));
        }
        _ => {
            let spin = since.filter(|s| *s < REFRESH_SPIN).map(|s| {
                let t = s / REFRESH_SPIN;
                // Ease out so it slows into place.
                (1.0 - (1.0 - t).powi(3)) * std::f32::consts::TAU
            });
            let color = if resp.hovered() || spin.is_some() { text() } else { subtle() };
            paint_refresh_icon(p, c, 6.0, spin.unwrap_or(0.0), color);
        }
    }
    if since.is_some_and(|s| s < REFRESH_SPIN + REFRESH_DONE) {
        ui.ctx().request_repaint();
    }
    let tip = format!("Refresh ({task_count} open tasks)");
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn paint_refresh_icon(p: &egui::Painter, c: egui::Pos2, r: f32, rot: f32, color: Color32) {
    let stroke = Stroke::new(1.6, color);
    // Arc from 40° to 330°, leaving a gap at the top right for the arrowhead.
    let (a0, a1) = (40f32.to_radians(), 330f32.to_radians());
    let n = 24;
    let pts: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32 + rot;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(egui::Shape::line(pts, stroke));
    // Arrowhead at the end of the arc, pointing along the direction of travel.
    let a = a1 + rot;
    let tip = c + vec2(a.cos(), a.sin()) * r;
    let dir = vec2(-a.sin(), a.cos());
    let out = vec2(a.cos(), a.sin());
    let back = tip - dir * 3.6;
    p.add(egui::Shape::convex_polygon(
        vec![tip + dir * 1.2, back + out * 2.8, back - out * 2.8],
        color,
        Stroke::NONE,
    ));
}

fn side_item(
    ui: &mut egui::Ui,
    icon: Glyph,
    label: &str,
    count: usize,
    badge: usize,
    selected: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, CornerRadius::same(6), theme::selected());
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), hover());
    }
    let y = rect.center().y;
    paint_glyph(p, egui::pos2(rect.left() + 19.0, y), 16.0, icon);
    p.text(egui::pos2(rect.left() + 36.0, y), Align2::LEFT_CENTER, label, FontId::proportional(14.5), text());
    let mut right = rect.right() - 10.0;
    if count > 0 {
        let r = p.text(egui::pos2(right, y), Align2::RIGHT_CENTER, count.to_string(), FontId::proportional(13.0), subtle());
        right = r.left() - 8.0;
    }
    if badge > 0 {
        let text = badge.to_string();
        let w = (10.0 + 7.0 * text.len() as f32).max(20.0);
        let b = egui::Rect::from_center_size(egui::pos2(right - w / 2.0, y), vec2(w, 18.0));
        p.rect_filled(b, CornerRadius::same(9), red());
        p.text(b.center(), Align2::CENTER_CENTER, text, FontId::proportional(11.5), Color32::WHITE);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Width kept free on the right of a task row for the notes button, date chip and star.
const ROW_CONTROLS_W: f32 = 170.0;

/// Edit state and app data the task rows need.
struct RowUi<'a> {
    meta: &'a MetaStore,
    lists: &'a [String],
    title_edit: &'a mut Option<TitleEdit>,
    notes_edit: &'a mut Option<NotesEdit>,
    expanded: &'a mut Option<TaskKey>,
    selection: &'a HashSet<TaskKey>,
    cursor: Option<&'a TaskKey>,
    row_rects: &'a mut HashMap<TaskKey, egui::Rect>,
    scroll_to_cursor: bool,
    /// Set when a row is clicked in a way that changes the selection.
    click: &'a mut Option<RowClick>,
}

fn task_row(ui: &mut egui::Ui, t: &Task, today: NaiveDate, show_file: bool, r: &mut RowUi) -> Option<Action> {
    let mut action = None;
    let starred = t.starred(today);
    let key = task_key(t);
    let meta = r.meta.get(t);
    let has_notes = meta.is_some_and(|m| !m.notes.trim().is_empty());
    let task_lists: Vec<String> = meta.map(|m| m.lists.clone()).unwrap_or_default();
    let selected = r.selection.contains(&key);
    let is_cursor = r.cursor == Some(&key);
    // The row's size is only known after drawing it, so use where it was last frame.
    // Registering this click area before the row's own widgets lets the checkbox, title
    // and buttons keep their clicks; clicks on empty space land here.
    if let Some(rect) = r.row_rects.get(&key).copied() {
        let bg = ui.interact(rect, ui.scope_id().with(("row", &key)), Sense::click_and_drag());
        bg.dnd_set_drag_payload(drag_payload(&key, r.selection));
        let fill = if selected {
            Some(blue().gamma_multiply(0.16))
        } else if is_cursor {
            Some(hover())
        } else if bg.hovered() {
            Some(hover().gamma_multiply(0.5))
        } else {
            None
        };
        if let Some(fill) = fill {
            ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
        }
        if is_cursor && r.scroll_to_cursor {
            ui.scroll_to_rect(rect, None);
        }
        if bg.double_clicked() {
            // Double-clicking the row (anywhere but the name) opens or closes its notes and lists.
            toggle_expanded(r, &key);
        } else if bg.clicked() {
            let m = ui.input(|i| i.modifiers);
            *r.click = Some(RowClick { key: key.clone(), toggle: m.command, range: m.shift });
        }
    }
    ui.add_space(3.0);
    let row = ui.horizontal_top(|ui| {
        let mut checked = false;
        let cb = ui.checkbox(&mut checked, "");
        if t.recurrence.is_some() {
            if cb.on_hover_text("Recurring task: opens in Obsidian so the Tasks plugin can create the next one").clicked() {
                action = Some(Action::Open(t.clone()));
            }
        } else if checked {
            action = Some(Action::Complete(t.clone()));
        }

        let text_w = (ui.available_width() - ROW_CONTROLS_W).max(120.0);
        ui.allocate_ui_with_layout(vec2(text_w, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(text_w);
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add_space(1.0);
            let editing = r.title_edit.as_ref().is_some_and(|e| e.key == key);
            if editing {
                let e = r.title_edit.as_mut().unwrap();
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut e.buf)
                        .font(FontId::proportional(14.5))
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(4, 2)),
                );
                if e.focus {
                    resp.request_focus();
                    e.focus = false;
                }
                if resp.lost_focus() {
                    let text = e.buf.trim().to_string();
                    let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    if !cancelled && !text.is_empty() && text != t.description {
                        action = Some(Action::Rename(t.clone(), text));
                    }
                    *r.title_edit = None;
                }
            } else {
                let mut job = egui::text::LayoutJob::default();
                if t.priority != Priority::Normal {
                    job.append(&format!("{} ", t.priority.icon()), 0.0, text_format(12.0, text(), false));
                }
                let mut fmt = text_format(14.5, text(), false);
                fmt.italics = t.in_progress;
                job.append(&t.description, 0.0, fmt);
                let resp = ui
                    .add(egui::Label::new(job).sense(Sense::click_and_drag()).wrap())
                    .on_hover_cursor(egui::CursorIcon::Text)
                    .on_hover_text("Click to rename. Right-click for more.");
                // Dragging the title drags the task (or the whole selection) toward a list.
                resp.dnd_set_drag_payload(drag_payload(&key, r.selection));
                if resp.clicked() {
                    let m = ui.input(|i| i.modifiers);
                    if m.command || m.shift {
                        // Cmd/Shift-click selects instead of renaming.
                        *r.click = Some(RowClick { key: key.clone(), toggle: m.command, range: m.shift });
                    } else {
                        *r.title_edit = Some(TitleEdit { key: key.clone(), buf: t.description.clone(), focus: true });
                    }
                }
                resp.context_menu(|ui| {
                    if let Some(a) = task_menu(ui, t, &task_lists, r) {
                        action = Some(a);
                    }
                });
            }

            let mut parts: Vec<String> = Vec::new();
            if show_file {
                parts.push(format!("{} · {}", t.vault, t.file.file_stem().unwrap_or_default().to_string_lossy()));
            }
            if let Some(rec) = &t.recurrence {
                parts.push(format!("🔁 {rec}"));
            }
            if !parts.is_empty() || !task_lists.is_empty() {
                let resp = ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let mut out = None;
                    if !parts.is_empty() {
                        let l = ui
                            .add(egui::Label::new(RichText::new(parts.join("  ·  ")).size(11.5).color(subtle())).sense(Sense::click()))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(format!("Open in Obsidian\n{} / {} (line {})", t.vault, t.rel_path(), t.line + 1));
                        if l.hovered() {
                            let y = l.rect.bottom() - 1.0;
                            ui.painter().hline(l.rect.x_range(), y, Stroke::new(1.0, subtle()));
                        }
                        if l.clicked() {
                            out = Some(Action::Open(t.clone()));
                        }
                    }
                    for name in &task_lists {
                        if list_tag(ui, name) {
                            out = Some(Action::ToggleList(t.clone(), name.clone(), false));
                        }
                    }
                    out
                });
                if let Some(a) = resp.inner {
                    action = Some(a);
                }
            }
        });

        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            ui.add_space(8.0);
            let (icon, color, tip) = if starred {
                ("★", star(), "Remove from Starred")
            } else {
                ("☆", faint(), "Add to Starred")
            };
            if ui
                .add(egui::Button::new(RichText::new(icon).size(18.0).color(color)).frame(false))
                .on_hover_text(tip)
                .clicked()
            {
                action = Some(Action::Star(t.clone(), !starred));
            }
            // Laid out right to left: star, notes, then the date chip, whose width varies.
            // Keeping it leftmost stops the notes button from shifting between rows.
            let open = r.expanded.as_ref() == Some(&key);
            if notes_button(ui, has_notes, open).clicked() {
                toggle_expanded(r, &key);
            }
            let resp = match t.due {
                Some(d) => date_chip(ui, d, today),
                None => ui
                    .add(egui::Button::new(RichText::new("📅").size(13.0).color(faint())).frame(false))
                    .on_hover_text("Add due date"),
            };
            if let Some(a) = due_menu(&resp, t, today) {
                action = Some(a);
            }
        });
    });
    r.row_rects.insert(key.clone(), row.response.rect.expand2(vec2(6.0, 3.0)));
    ui.add_space(3.0);

    if r.expanded.as_ref() == Some(&key)
        && let Some(a) = details_panel(ui, t, meta, &task_lists, r) {
            action = Some(a);
        }
    action
}

/// Room taken from the page's right margin for the scroll bar, so it sits beside the
/// rows instead of on top of the stars.
const SCROLLBAR_GUTTER: f32 = 22.0;

/// A scrolling page area whose scroll bar is pushed out into the right margin.
fn list_scroll(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let mut rect = ui.available_rect_before_wrap();
    rect.max.x += SCROLLBAR_GUTTER;
    // `scope_builder` lays out a child area in a rect we choose: here, a bit wider than the page.
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            // The same width back on the inside, so rows end where they did before.
            let margin = Margin { right: SCROLLBAR_GUTTER as i8, ..Default::default() };
            Frame::new().inner_margin(margin).show(ui, add);
        });
    });
}

/// What dragging a row carries: the whole selection if the row is selected, else just that task.
fn drag_payload(key: &TaskKey, selection: &HashSet<TaskKey>) -> DraggedTasks {
    if selection.contains(key) {
        DraggedTasks(selection.iter().cloned().collect())
    } else {
        DraggedTasks(vec![key.clone()])
    }
}

/// While tasks are being dragged, a small label follows the pointer.
fn drag_label(ctx: &egui::Context) {
    let Some(dragged) = egui::DragAndDrop::payload::<DraggedTasks>(ctx) else { return };
    let Some(pos) = ctx.pointer_latest_pos() else { return };
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let n = dragged.0.len();
    let text = if n == 1 { "1 task: drop on a list".to_string() } else { format!("{n} tasks: drop on a list") };
    egui::Area::new(egui::Id::unique("drag_label"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos + vec2(14.0, 10.0))
        .interactable(false)
        .show(ctx, |ui| {
            Frame::new()
                .fill(toast_bg())
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(10, 5))
                .show(ui, |ui| ui.label(RichText::new(text).size(12.5).color(Color32::WHITE)));
        });
}

fn toggle_expanded(r: &mut RowUi, key: &TaskKey) {
    if r.expanded.as_ref() == Some(key) {
        *r.expanded = None;
    } else {
        *r.expanded = Some(key.clone());
    }
    *r.notes_edit = None;
}

fn text_format(size: f32, color: Color32, bold: bool) -> egui::text::TextFormat {
    let family = if bold { fonts::bold() } else { egui::FontFamily::Proportional };
    egui::text::TextFormat { font_id: FontId::new(size, family), color, ..Default::default() }
}

/// Small tag showing a list the task belongs to, with an × to remove it.
/// Returns true when the × is clicked.
fn list_tag(ui: &mut egui::Ui, name: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(name.to_string(), FontId::proportional(11.0), blue());
    // Room on the right for a small ×.
    let size = galley.size() + vec2(28.0, 2.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let x_area = egui::Rect::from_min_max(egui::pos2(rect.right() - 15.0, rect.top()), rect.max);
    let over_x = resp.hover_pos().is_some_and(|p| x_area.contains(p));
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(4), blue().gamma_multiply(if resp.hovered() { 0.2 } else { 0.12 }));
    p.circle_filled(egui::pos2(rect.left() + 6.0, rect.center().y), 1.8, blue());
    p.galley(egui::pos2(rect.left() + 11.0, rect.center().y - galley.size().y / 2.0), galley, blue());
    // The × is faint until the tag is hovered, and bright when the pointer is on it.
    let alpha = if over_x { 1.0 } else if resp.hovered() { 0.6 } else { 0.3 };
    let c = x_area.center();
    let stroke = Stroke::new(1.2, blue().gamma_multiply(alpha));
    p.line_segment([c + vec2(-2.5, -2.5), c + vec2(2.5, 2.5)], stroke);
    p.line_segment([c + vec2(-2.5, 2.5), c + vec2(2.5, -2.5)], stroke);
    if over_x {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let resp = resp.on_hover_text(if over_x { format!("Remove from {name}") } else { name.to_string() });
    // Only a click on the × removes the task from the list.
    resp.clicked() && resp.interact_pointer_pos().is_some_and(|p| x_area.contains(p))
}

/// Painted notes icon: lines of text, blue when the task has notes.
fn notes_button(ui: &mut egui::Ui, has_notes: bool, open: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() || open {
        p.rect_filled(rect, CornerRadius::same(5), hover());
    }
    let color = if has_notes { blue() } else if resp.hovered() || open { subtle() } else { faint() };
    let c = rect.center();
    for (i, w) in [9.0f32, 9.0, 5.5].iter().enumerate() {
        let y = c.y - 4.0 + i as f32 * 4.0;
        p.line_segment([egui::pos2(c.x - 4.5, y), egui::pos2(c.x - 4.5 + w, y)], Stroke::new(1.5, color));
    }
    let tip = if has_notes { "Show notes and lists" } else { "Add notes or lists" };
    resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Right-click menu on a task title.
fn task_menu(ui: &mut egui::Ui, t: &Task, task_lists: &[String], r: &mut RowUi) -> Option<Action> {
    let mut out = None;
    let key = task_key(t);
    if ui.button("Rename").clicked() {
        *r.title_edit = Some(TitleEdit { key: key.clone(), buf: t.description.clone(), focus: true });
    }
    if ui.button("Notes and lists").clicked() {
        *r.expanded = Some(key);
        *r.notes_edit = None;
    }
    if ui.button("Open in Obsidian").clicked() {
        out = Some(Action::Open(t.clone()));
    }
    ui.separator();
    if ui.button(RichText::new("Delete task…").color(red())).clicked() {
        out = Some(Action::AskDelete(vec![t.clone()]));
    }
    if !r.lists.is_empty() {
        ui.separator();
        ui.label(RichText::new("Lists").size(11.5).color(subtle()));
        if let Some(a) = list_toggles(ui, t, task_lists, r.lists) {
            out = Some(a);
        }
    }
    out
}

/// One checkbox per list. Returns the toggle the user clicked.
fn list_toggles(ui: &mut egui::Ui, t: &Task, task_lists: &[String], lists: &[String]) -> Option<Action> {
    let mut out = None;
    for name in lists {
        let mut on = task_lists.contains(name);
        if ui.checkbox(&mut on, name).changed() {
            out = Some(Action::ToggleList(t.clone(), name.clone(), on));
        }
    }
    out
}

/// Notes and lists for the expanded task, shown under its row.
fn details_panel(ui: &mut egui::Ui, t: &Task, meta: Option<&TaskMeta>, task_lists: &[String], r: &mut RowUi) -> Option<Action> {
    let mut out = None;
    let key = task_key(t);
    let notes = meta.map(|m| m.notes.as_str()).unwrap_or("");
    ui.horizontal(|ui| {
        ui.add_space(28.0);
        Frame::new()
            .fill(field_bg())
            .stroke(Stroke::new(1.0, field_stroke()))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| ui.vertical(|ui| {
                ui.set_width(ui.available_width() - 4.0);
                ui.label(RichText::new("Notes").size(11.5).family(fonts::bold()).color(subtle()));
                ui.add_space(2.0);
                let editing = r.notes_edit.as_ref().is_some_and(|e| e.key == key);
                if editing {
                    let e = r.notes_edit.as_mut().unwrap();
                    let resp = ui.add(
                        egui::TextEdit::multiline(&mut e.buf)
                            .hint_text("Markdown works: - bullets, **bold**, *italic*, `code`, # headings")
                            .desired_width(f32::INFINITY)
                            .desired_rows(4)
                            .font(FontId::proportional(13.5)),
                    );
                    if e.focus {
                        resp.request_focus();
                        e.focus = false;
                    }
                    ui.label(RichText::new("Saves when you click away. Esc cancels.").size(11.0).color(faint()));
                    if resp.lost_focus() {
                        let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        if !cancelled && e.buf.trim_end() != notes {
                            out = Some(Action::SaveNotes(t.clone(), e.buf.clone()));
                        }
                        *r.notes_edit = None;
                    }
                } else {
                    let start_edit = if notes.trim().is_empty() {
                        ui.add(egui::Label::new(RichText::new("Add notes…").size(13.5).color(faint())).sense(Sense::click()))
                            .on_hover_cursor(egui::CursorIcon::Text)
                            .clicked()
                    } else {
                        let inner = ui.scope(|ui| markdown(ui, notes));
                        ui.interact(inner.response.rect, ui.scope_id().with(("notes", &key)), Sense::click())
                            .on_hover_cursor(egui::CursorIcon::Text)
                            .on_hover_text("Click to edit")
                            .clicked()
                    };
                    if start_edit {
                        *r.notes_edit = Some(NotesEdit { key: key.clone(), buf: notes.to_string(), focus: true });
                    }
                }

                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Lists").size(11.5).family(fonts::bold()).color(subtle()));
                    for name in task_lists {
                        if list_tag(ui, name) {
                            out = Some(Action::ToggleList(t.clone(), name.clone(), false));
                        }
                    }
                    let add = ui
                        .add(egui::Button::new(RichText::new("+ Add to list").size(12.0).color(blue())).frame(false))
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    egui::Popup::menu(&add)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            if r.lists.is_empty() {
                                ui.label(RichText::new("Create a list with + next to LISTS in the sidebar.").color(subtle()));
                            } else if let Some(a) = list_toggles(ui, t, task_lists, r.lists) {
                                out = Some(a);
                            }
                        });
                });
            }));
    });
    ui.add_space(6.0);
    out
}

/// Renders a small subset of Markdown: headings, bullets, numbered and task lists,
/// quotes, **bold**, *italic*, `code` and [links](url).
fn markdown(ui: &mut egui::Ui, src: &str) {
    ui.spacing_mut().item_spacing.y = 3.0;
    let size = 13.5;
    for raw in src.lines() {
        let expanded = raw.replace('\t', "    ");
        let line = expanded.trim_start();
        let level = (expanded.len() - line.len()) / 2;
        if line.is_empty() {
            ui.add_space(4.0);
            continue;
        }
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
            let hsize = [17.0, 15.5, 14.5, 14.0, 13.5, 13.5][hashes - 1];
            ui.add(egui::Label::new(md_inline(line[hashes..].trim(), hsize, text(), true)).wrap());
            continue;
        }
        enum Marker {
            Bullet,
            Check(bool),
            Number(String),
            Quote,
        }
        let num_len = line.chars().take_while(|c| c.is_ascii_digit()).count();
        let (marker, rest) = if let Some(r) = ["- [ ] ", "* [ ] ", "+ [ ] "].iter().find_map(|p| line.strip_prefix(p)) {
            (Some(Marker::Check(false)), r)
        } else if let Some(r) = ["- [x] ", "* [x] ", "- [X] ", "* [X] "].iter().find_map(|p| line.strip_prefix(p)) {
            (Some(Marker::Check(true)), r)
        } else if let Some(r) = ["- ", "* ", "+ "].iter().find_map(|p| line.strip_prefix(p)) {
            (Some(Marker::Bullet), r)
        } else if num_len > 0 && (line[num_len..].starts_with(". ") || line[num_len..].starts_with(") ")) {
            (Some(Marker::Number(line[..num_len + 1].to_string())), &line[num_len + 2..])
        } else if let Some(r) = line.strip_prefix("> ") {
            (Some(Marker::Quote), r)
        } else {
            (None, line)
        };
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add_space(level as f32 * 16.0);
            let color = if matches!(marker, Some(Marker::Quote) | Some(Marker::Check(true))) { subtle() } else { text() };
            match &marker {
                None => {}
                Some(Marker::Bullet) => {
                    let (rect, _) = ui.allocate_exact_size(vec2(10.0, size * 1.3), Sense::hover());
                    let r = if level == 0 { 2.4 } else { 2.0 };
                    let c = rect.center();
                    if level % 2 == 0 {
                        ui.painter().circle_filled(c, r, subtle());
                    } else {
                        ui.painter().circle_stroke(c, r, Stroke::new(1.0, subtle()));
                    }
                }
                Some(Marker::Check(done)) => {
                    let (rect, _) = ui.allocate_exact_size(vec2(12.0, size * 1.3), Sense::hover());
                    let b = egui::Rect::from_center_size(rect.center(), vec2(11.0, 11.0));
                    if *done {
                        ui.painter().rect_filled(b, CornerRadius::same(3), subtle());
                        let c = b.center();
                        let pts = vec![c + vec2(-2.8, 0.2), c + vec2(-0.8, 2.3), c + vec2(3.0, -2.2)];
                        ui.painter().add(egui::Shape::line(pts, Stroke::new(1.5, Color32::WHITE)));
                    } else {
                        ui.painter().rect_stroke(b, CornerRadius::same(3), Stroke::new(1.2, subtle()), egui::StrokeKind::Inside);
                    }
                }
                Some(Marker::Number(n)) => {
                    ui.label(RichText::new(n).size(size).color(subtle()));
                }
                Some(Marker::Quote) => {
                    let (rect, _) = ui.allocate_exact_size(vec2(3.0, size * 1.3), Sense::hover());
                    ui.painter().rect_filled(rect, CornerRadius::same(1), divider());
                }
            }
            let mut job = md_inline(rest, size, color, false);
            if matches!(marker, Some(Marker::Quote)) {
                for s in &mut job.sections {
                    s.format.italics = true;
                }
            }
            if matches!(marker, Some(Marker::Check(true))) {
                for s in &mut job.sections {
                    s.format.strikethrough = Stroke::new(1.0, subtle());
                }
            }
            ui.add(egui::Label::new(job).wrap());
        });
    }
}

/// Inline Markdown: **bold**, *italic*, `code`, [text](url).
fn md_inline(s: &str, size: f32, color: Color32, bold_base: bool) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let (mut bold, mut italic, mut code) = (false, false, false);
    let mut buf = String::new();
    let push = |job: &mut egui::text::LayoutJob, buf: &mut String, bold: bool, italic: bool, code: bool, link: bool| {
        if buf.is_empty() {
            return;
        }
        let mut f = text_format(size, if link { blue() } else { color }, bold_base || bold);
        f.italics = italic;
        if code {
            f.font_id = FontId::monospace(size * 0.92);
            f.background = hover();
        }
        if link {
            f.underline = Stroke::new(1.0, blue().gamma_multiply(0.5));
        }
        job.append(buf, 0.0, f);
        buf.clear();
    };
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            push(&mut job, &mut buf, bold, italic, code, false);
            code = !code;
            i += 1;
            continue;
        }
        if !code {
            if (c == '*' || c == '_') && chars.get(i + 1) == Some(&c) {
                push(&mut job, &mut buf, bold, italic, code, false);
                bold = !bold;
                i += 2;
                continue;
            }
            if c == '*' {
                push(&mut job, &mut buf, bold, italic, code, false);
                italic = !italic;
                i += 1;
                continue;
            }
            if c == '[' {
                let rest: String = chars[i + 1..].iter().collect();
                if let Some(close) = rest.find("](")
                    && let Some(end) = rest[close + 2..].find(')') {
                        push(&mut job, &mut buf, bold, italic, code, false);
                        buf.push_str(&rest[..close]);
                        push(&mut job, &mut buf, bold, italic, code, true);
                        i += 1 + rest[..close + 2 + end + 1].chars().count();
                        continue;
                    }
            }
        }
        buf.push(c);
        i += 1;
    }
    push(&mut job, &mut buf, bold, italic, code, false);
    job
}

fn completed_row(ui: &mut egui::Ui, t: &Task) -> Option<Action> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        let mut checked = true;
        let cb = ui.checkbox(&mut checked, "");
        if t.recurrence.is_some() {
            if cb.on_hover_text("Recurring task: opens in Obsidian, since the Tasks plugin already created the next one").clicked() {
                action = Some(Action::Open(t.clone()));
            }
        } else if cb.on_hover_text("Uncheck to reopen").clicked() {
            action = Some(Action::Reopen(t.clone()));
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let title = RichText::new(&t.description).size(14.5).color(subtle()).strikethrough();
            let resp = ui
                .add(egui::Label::new(title).sense(Sense::click()).wrap())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("Open in Obsidian\n{} / {} (line {})", t.vault, t.rel_path(), t.line + 1));
            if resp.clicked() {
                action = Some(Action::Open(t.clone()));
            }
            let file = t.file.file_stem().unwrap_or_default().to_string_lossy();
            ui.label(RichText::new(format!("{} · {}", t.vault, file)).size(11.5).color(subtle()));
        });
    });
    action
}

fn date_chip(ui: &mut egui::Ui, d: NaiveDate, today: NaiveDate) -> egui::Response {
    let days = (d - today).num_days();
    let text = match days {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        -1 => "Yesterday".to_string(),
        2..=6 => d.format("%a").to_string(),
        _ if d.format("%Y").to_string() == today.format("%Y").to_string() => d.format("%b %-d").to_string(),
        _ => d.format("%b %-d, %Y").to_string(),
    };
    let (fg, bg) = if days < 0 {
        chip_overdue()
    } else if days == 0 {
        chip_today()
    } else {
        chip_later()
    };
    // Painted manually so the chip keeps its own height inside the row.
    let galley = ui.painter().layout_no_wrap(text, FontId::proportional(11.5), fg);
    let size = galley.size() + vec2(12.0, 4.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let bg = if resp.hovered() { bg.gamma_multiply(0.85) } else { bg };
    ui.painter().rect_filled(rect, CornerRadius::same(5), bg);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Change due date")
}

// Due date menu layout: the calendar is 7 cells wide and everything else matches it.
const CAL_CELL: f32 = 32.0;
const CAL_GAP: f32 = 2.0;
const MENU_W: f32 = CAL_CELL * 7.0 + CAL_GAP * 6.0;

/// Full-width menu row with a label on the left and an optional hint on the right.
fn menu_row(ui: &mut egui::Ui, label: &str, hint: &str, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(MENU_W, 28.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), hover());
    }
    let y = rect.center().y;
    p.text(egui::pos2(rect.left() + 8.0, y), Align2::LEFT_CENTER, label, FontId::proportional(14.0), color);
    if !hint.is_empty() {
        p.text(egui::pos2(rect.right() - 8.0, y), Align2::RIGHT_CENTER, hint, FontId::proportional(13.0), subtle());
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Divider spanning the full menu width.
fn menu_divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(MENU_W, 11.0), Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, Stroke::new(1.0, divider()));
}

/// Due date menu opened from a task's date chip (or calendar button).
fn due_menu(resp: &egui::Response, t: &Task, today: NaiveDate) -> Option<Action> {
    date_menu(resp, t.due, t.due.is_some(), today).map(|d| Action::SetDue(t.clone(), d))
}

/// Date menu shown under `resp` when it's clicked: quick picks, a calendar and,
/// with `allow_remove`, "Remove due date". Returns `Some(Some(date))` for a picked date,
/// `Some(None)` for remove, and `None` if nothing was chosen this frame.
fn date_menu(resp: &egui::Response, current: Option<NaiveDate>, allow_remove: bool, today: NaiveDate) -> Option<Option<NaiveDate>> {
    let mut out = None;
    egui::Popup::menu(resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(MENU_W);
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add_space(2.0);
            ui.label(RichText::new("  Due date").size(12.0).family(fonts::bold()).color(subtle()));
            ui.add_space(2.0);
            let quick = [
                ("Today", today),
                ("Tomorrow", today + chrono::Days::new(1)),
                ("End of week", end_of_week(today)),
                ("Next Monday", next_monday(today)),
                ("In 1 week", today + chrono::Days::new(7)),
            ];
            for (label, d) in quick {
                if menu_row(ui, label, &d.format("%a %b %-d").to_string(), text()).clicked() {
                    out = Some(Some(d));
                    ui.close();
                }
            }
            menu_divider(ui);
            if let Some(d) = calendar(ui, resp.id.with("cal"), current, today) {
                out = Some(Some(d));
                ui.close();
            }
            if allow_remove {
                menu_divider(ui);
                if menu_row(ui, "Remove due date", "", red()).clicked() {
                    out = Some(None);
                    ui.close();
                }
            }
        });
    out
}

/// Month calendar, painted on a fixed grid. Returns the clicked day.
fn calendar(ui: &mut egui::Ui, id: egui::Id, selected: Option<NaiveDate>, today: NaiveDate) -> Option<NaiveDate> {
    let start = selected.unwrap_or(today);
    let mut month: NaiveDate = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || start.with_day(1).unwrap()));
    let mut picked = None;

    // Header: ‹  Month Year  ›
    let (head, _) = ui.allocate_exact_size(vec2(MENU_W, 30.0), Sense::hover());
    ui.painter().text(
        head.center(),
        Align2::CENTER_CENTER,
        month.format("%B %Y").to_string(),
        FontId::new(14.0, fonts::bold()),
        text(),
    );
    for (dx, arrow, forward) in [(0.0, "‹", false), (MENU_W - CAL_CELL, "›", true)] {
        let r = egui::Rect::from_min_size(egui::pos2(head.left() + dx, head.top() + 1.0), vec2(CAL_CELL, 28.0));
        let resp = ui.interact(r, id.with(arrow), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(6), hover());
        }
        ui.painter().text(r.center(), Align2::CENTER_CENTER, arrow, FontId::proportional(18.0), text());
        if resp.clicked() {
            month = if forward {
                month + chrono::Months::new(1)
            } else {
                (month - chrono::Days::new(1)).with_day(1).unwrap()
            };
        }
    }

    // Weekday letters + day cells.
    // Weeks start on Sunday.
    let offset = month.weekday().num_days_from_sunday() as usize;
    let days_in_month = (month + chrono::Months::new(1) - month).num_days() as usize;
    let weeks = (offset + days_in_month).div_ceil(7);
    let row_h = 28.0;
    let (grid, _) = ui.allocate_exact_size(vec2(MENU_W, 22.0 + weeks as f32 * (row_h + CAL_GAP)), Sense::hover());
    let cell = |col: usize, row: f32, h: f32| {
        egui::Rect::from_min_size(
            egui::pos2(grid.left() + col as f32 * (CAL_CELL + CAL_GAP), grid.top() + row),
            vec2(CAL_CELL, h),
        )
    };
    for (i, w) in ["S", "M", "T", "W", "T", "F", "S"].iter().enumerate() {
        ui.painter().text(cell(i, 0.0, 20.0).center(), Align2::CENTER_CENTER, *w, FontId::proportional(11.5), subtle());
    }
    for n in 0..days_in_month {
        let day = month + chrono::Days::new(n as u64);
        let slot = offset + n;
        let r = cell(slot % 7, 22.0 + (slot / 7) as f32 * (row_h + CAL_GAP), row_h);
        let resp = ui.interact(r, id.with(n), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let is_sel = Some(day) == selected;
        let weekend = matches!(day.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun);
        let p = ui.painter();
        if is_sel {
            p.rect_filled(r, CornerRadius::same(6), blue());
        } else if resp.hovered() {
            p.rect_filled(r, CornerRadius::same(6), hover());
        } else if weekend {
            // Weekends get a soft grey cell instead of a strong color.
            p.rect_filled(r, CornerRadius::same(6), hover().gamma_multiply(0.6));
        }
        let color = if is_sel {
            Color32::WHITE
        } else if day == today {
            blue()
        } else if weekend {
            subtle()
        } else {
            text()
        };
        p.text(r.center(), Align2::CENTER_CENTER, day.day().to_string(), FontId::proportional(13.0), color);
        if resp.clicked() {
            picked = Some(day);
        }
    }
    ui.data_mut(|d| d.insert_temp(id, month));
    picked
}

#[derive(PartialEq)]
enum PluginState {
    Enabled,
    InstalledOff,
    Missing,
}

/// Checks a community plugin in a vault: installed (folder exists) and enabled
/// (listed in community-plugins.json).
fn plugin_state(vault_root: &std::path::Path, id: &str) -> PluginState {
    let obs = vault_root.join(".obsidian");
    if !obs.join("plugins").join(id).is_dir() {
        return PluginState::Missing;
    }
    let enabled = std::fs::read_to_string(obs.join("community-plugins.json"))
        .is_ok_and(|s| s.contains(&format!("\"{id}\"")));
    if enabled { PluginState::Enabled } else { PluginState::InstalledOff }
}

const TASKS_PLUGIN: &str = "obsidian-tasks-plugin";
const ADV_URI_PLUGIN: &str = "obsidian-advanced-uri";

/// Advanced URI lets us open a note at a specific line.
fn has_advanced_uri(vault_root: &std::path::Path) -> bool {
    plugin_state(vault_root, ADV_URI_PLUGIN) == PluginState::Enabled
}

fn open_in_obsidian(t: &Task) {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    let vault = utf8_percent_encode(&t.vault, NON_ALPHANUMERIC).to_string();
    let path = utf8_percent_encode(&t.rel_path(), NON_ALPHANUMERIC).to_string();
    let url = if has_advanced_uri(&t.vault_root) {
        format!("obsidian://adv-uri?vault={vault}&filepath={path}&line={}", t.line + 1)
    } else {
        format!("obsidian://open?vault={vault}&file={path}")
    };
    let _ = std::process::Command::new("open").arg(url).spawn();
}

fn date_bucket(d: Option<NaiveDate>, today: NaiveDate) -> &'static str {
    match d {
        None => "No Due Date",
        Some(d) if d < today => "Overdue",
        Some(d) if d == today => "Today",
        Some(d) if (d - today).num_days() <= 7 => "Next 7 Days",
        Some(_) => "Later",
    }
}

fn setup_style(ctx: &egui::Context) {
    // Both egui themes (light and dark) get the same spacing; see theme::apply.
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(8.0, 4.0);
        s.spacing.interact_size.y = 22.0;
    });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.last_scan.elapsed() >= RESCAN_EVERY {
            self.rescan();
        }
        ui.ctx().request_repaint_after(RESCAN_EVERY);
        let ctx = ui.ctx().clone();
        let dark = self.cfg.appearance.is_dark(&ctx);
        if self.applied_dark != Some(dark) {
            theme::apply(&ctx, dark);
            ctx.send_viewport_cmd(egui::ViewportCommand::Icon(Some(dock_icon(dark))));
            self.applied_dark = Some(dark);
        }
        self.handle_menu(&ctx);
        self.capture_window(&ctx);
        // Switching views (vault, list, Starred, Completed…) closes any open notes panel.
        if self.view != self.shown_view {
            self.expanded = None;
            self.notes_edit = None;
            self.title_edit = None;
            self.selection.clear();
            self.cursor = None;
            self.anchor = None;
            self.shown_view = self.view.clone();
        }

        egui::Panel::left("sidebar")
            .exact_size(230.0)
            .resizable(false)
            .frame(Frame::new().fill(sidebar_bg()).inner_margin(Margin::symmetric(12, 10)))
            .show(ui, |ui| self.sidebar(ui));

        let today = Local::now().date_naive();
        // Keys are read before drawing so a shortcut (e.g. Esc) can't also reach a widget.
        let mut action = self.list_keys(&ctx, today);
        egui::CentralPanel::default()
            .frame(Frame::new().fill(main_bg()).inner_margin(Margin { left: 40, right: 40, top: 28, bottom: 0 }))
            .show(ui, |ui| {
                self.header(ui, today);
                if let Some(e) = &self.error {
                    ui.label(RichText::new(format!("⚠ {e}")).color(red()));
                    ui.add_space(6.0);
                }
                if self.view == View::Settings {
                    self.settings(ui);
                } else if self.view == View::Completed {
                    ui.horizontal(|ui| {
                        for r in Range::ALL {
                            if ui.selectable_label(self.range == r, r.label()).clicked() {
                                self.range = r;
                            }
                        }
                    });
                    ui.add_space(6.0);
                    action = self.completed_list(ui, today);
                } else {
                    // While tasks are selected, the action bar takes the new task box's place.
                    let bar = if self.selection.is_empty() {
                        self.new_task_row(ui, today);
                        None
                    } else {
                        self.action_bar(ui, today)
                    };
                    action = self.task_list(ui, today).or(bar).or(action.take());
                }
            });

        // Undo bar for the last completed task.
        let mut undo = false;
        if let Some(last) = &self.last_completed {
            let left = UNDO_FOR.saturating_sub(last.at.elapsed());
            if left.is_zero() {
                self.last_completed = None;
            } else {
                ui.ctx().request_repaint_after(left);
                egui::Area::new(egui::Id::unique("undo_bar"))
                    .anchor(Align2::CENTER_BOTTOM, vec2(115.0, -20.0))
                    .show(ui.ctx(), |ui| {
                        Frame::new()
                            .fill(toast_bg())
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::symmetric(16, 10))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let msg = match last.items.as_slice() {
                                        [(_, _, one)] => {
                                            let mut text = one.clone();
                                            if text.chars().count() > 48 {
                                                text = text.chars().take(47).collect::<String>() + "…";
                                            }
                                            format!("Completed \"{text}\"")
                                        }
                                        many => format!("Completed {} tasks", many.len()),
                                    };
                                    glyph(ui, 14.0, Glyph::Done);
                                    ui.label(RichText::new(msg).color(Color32::WHITE));
                                    ui.add_space(8.0);
                                    if ui
                                        .add(egui::Button::new(RichText::new("Undo").color(link())).frame(false))
                                        .clicked()
                                    {
                                        undo = true;
                                    }
                                });
                            });
                    });
            }
        }
        drag_label(ui.ctx());
        if let Some(a) = self.delete_dialog(ui.ctx()) {
            action = Some(a);
        }
        if undo {
            self.undo_last();
        }
        if let Some(a) = action {
            self.run(a);
        }
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 680.0])
            .with_min_inner_size([640.0, 360.0])
            .with_title("Vault Tasks")
            // Without this eframe swaps in its default icon in the Dock.
            .with_icon(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")).expect("icon.png")),
        ..Default::default()
    };
    eframe::run_native("Vault Tasks", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn default_due_dates() {
        let mut c = Config::default();
        let sat = d("2026-11-07");
        assert_eq!(c.default_due_date(sat), None);
        c.default_due = DefaultDue::Today;
        assert_eq!(c.default_due_date(sat), Some(sat));
        c.default_due = DefaultDue::DaysOut;
        c.default_due_days = 3;
        assert_eq!(c.default_due_date(sat), Some(d("2026-11-10")));
        c.default_due = DefaultDue::EndOfWeek;
        assert_eq!(c.default_due_date(sat), Some(d("2026-11-13"))); // Sat -> next Fri
        assert_eq!(c.default_due_date(d("2026-11-08")), Some(d("2026-11-13"))); // Sun
        assert_eq!(c.default_due_date(d("2026-11-09")), Some(d("2026-11-13"))); // Mon
        assert_eq!(c.default_due_date(d("2026-11-13")), Some(d("2026-11-13"))); // Fri
    }
}
