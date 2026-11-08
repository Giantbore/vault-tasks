mod fonts;
mod menu;
mod tasks;

use chrono::{Datelike, Local, NaiveDate};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, Sense,
    Stroke, vec2,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use fonts::BodyFont;
use menu::Cmd;
use tasks::{Priority, Task};

const RESCAN_EVERY: Duration = Duration::from_secs(15);

// Palette (light, Things-like)
const SIDEBAR_BG: Color32 = Color32::from_rgb(244, 244, 246);
const MAIN_BG: Color32 = Color32::WHITE;
const TEXT: Color32 = Color32::from_rgb(30, 30, 32);
const SUBTLE: Color32 = Color32::from_rgb(150, 150, 156);
const BLUE: Color32 = Color32::from_rgb(28, 110, 220);
const RED: Color32 = Color32::from_rgb(232, 64, 82);
const STAR: Color32 = Color32::from_rgb(250, 190, 20);
const SELECTED: Color32 = Color32::from_rgb(222, 222, 227);
const HOVER: Color32 = Color32::from_rgb(233, 233, 237);
const GREEN: Color32 = Color32::from_rgb(52, 168, 90);
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

/// Upcoming Friday (today if it is Friday; next week's on Sat/Sun).
fn end_of_week(today: NaiveDate) -> NaiveDate {
    let wd = today.weekday().num_days_from_monday() as u64;
    let days = if wd <= 4 { 4 - wd } else { 11 - wd };
    today + chrono::Days::new(days)
}

/// Next Monday (a week from today if today is Monday).
fn next_monday(today: NaiveDate) -> NaiveDate {
    let wd = today.weekday().num_days_from_monday() as u64;
    today + chrono::Days::new(7 - wd)
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
        if let Ok(s) = std::fs::read_to_string(Self::path()) {
            if let Ok(c) = serde_json::from_str(&s) {
                return c;
            }
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
}

/// The last task checked off, for the Undo bar.
struct LastCompleted {
    file: PathBuf,
    line: usize,
    description: String,
    at: Instant,
}

struct NewTask {
    text: String,
    due: Option<NaiveDate>,
    vault: usize,
}

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
        let mut app = App {
            view: if cfg.vaults.is_empty() { View::Settings } else { View::All },
            cfg,
            tasks: Vec::new(),
            last_scan: Instant::now(),
            scan_ms: 0,
            search: String::new(),
            error: None,
            new_exclude: String::new(),
            draft: NewTask { text: String::new(), due: None, vault: 0 },
            focus_new: false,
            menu: menu::MenuBar::install(&cc.egui_ctx),
            range: Range::Week,
            last_completed: None,
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
                Cmd::Refresh => self.rescan(),
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
        self.last_scan = Instant::now();
        self.scan_ms = start.elapsed().as_millis();
    }

    fn base_filter(&self, t: &Task, today: NaiveDate) -> bool {
        if self.cfg.hide_future_start && t.start.is_some_and(|s| s > today) {
            return false;
        }
        let q = self.search.trim().to_lowercase();
        q.is_empty()
            || t.description.to_lowercase().contains(&q)
            || t.rel_path().to_lowercase().contains(&q)
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
        let found = self
            .tasks
            .iter()
            .find(|t| t.completed && t.file == last.file && t.line == last.line && t.description == last.description)
            .or_else(|| self.tasks.iter().find(|t| t.completed && t.file == last.file && t.description == last.description))
            .cloned();
        match found {
            Some(t) => self.run(Action::Reopen(t)),
            None => self.error = Some("Couldn't find that task to undo. Reopen it from Completed.".into()),
        }
    }

    fn run(&mut self, action: Action) {
        let today = Local::now().date_naive();
        let result = match &action {
            Action::Complete(t) => tasks::complete_task(t).inspect(|_| {
                if let Some(s) = &self.cfg.chime {
                    play_sound(s);
                }
                self.last_completed = Some(LastCompleted {
                    file: t.file.clone(),
                    line: t.line,
                    description: t.description.clone(),
                    at: Instant::now(),
                });
            }),
            Action::Reopen(t) => tasks::uncomplete_task(t),
            Action::Star(t, on) => tasks::set_scheduled(t, on.then_some(today)),
            Action::SetDue(t, d) => tasks::set_due(t, *d),
            Action::Open(t) => {
                open_in_obsidian(t);
                Ok(())
            }
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

    fn add_draft(&mut self) {
        let Some(root) = self.cfg.vaults.get(self.draft.vault).cloned() else {
            self.error = Some("Add a vault folder in Settings first.".into());
            return;
        };
        let today = Local::now().date_naive();
        let sched = (self.view == View::Today).then_some(today);
        match tasks::add_task(&root, &self.draft.text, self.draft.due, sched) {
            Ok(()) => {
                self.draft.text.clear();
                self.draft.due = self.cfg.default_due_date(today);
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

        if side_item(ui, "📥", BLUE, "All", all_count, overdue, self.view == View::All).clicked() {
            self.view = View::All;
        }
        if side_item(ui, "★", STAR, "Today", today_count, today_overdue, self.view == View::Today).clicked() {
            self.view = View::Today;
        }
        ui.add_space(10.0);
        let done_today = self.completed_in(Range::Today, today).len();
        if side_item(ui, "✔", GREEN, "Completed", done_today, 0, self.view == View::Completed)
            .on_hover_text("Completed today")
            .clicked()
        {
            self.view = View::Completed;
        }

        if self.cfg.vaults.len() > 1 {
            ui.add_space(18.0);
            ui.label(RichText::new("VAULTS").size(11.0).color(SUBTLE).family(fonts::bold()));
            ui.add_space(4.0);
            for v in self.cfg.vaults.clone() {
                let name = vault_name(&v);
                let count = self.tasks_for(&View::Vault(name.clone()), today).len();
                let sel = self.view == View::Vault(name.clone());
                if side_item(ui, "◈", SUBTLE, &name, count, 0, sel).clicked() {
                    self.view = View::Vault(name);
                }
            }
        }

        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add(egui::Button::new(RichText::new("⚙  Settings").color(TEXT)).frame(false))
                    .clicked()
                {
                    self.view = View::Settings;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new(RichText::new("⟳").size(16.0).color(SUBTLE)).frame(false))
                        .on_hover_text("Refresh")
                        .clicked()
                    {
                        self.rescan();
                    }
                });
            });
            ui.separator();
        });
    }

    // ---------- Main list ----------

    fn header(&mut self, ui: &mut egui::Ui, today: NaiveDate) {
        let (icon, color, title, sub) = match &self.view {
            View::All => ("📥", BLUE, "All Tasks".to_string(), None),
            View::Today => ("★", STAR, "Today".to_string(), Some(today.format("%A, %B %-d").to_string())),
            View::Completed => ("✔", GREEN, "Completed".to_string(), None),
            View::Vault(v) => ("◈", SUBTLE, v.clone(), None),
            View::Settings => ("⚙", SUBTLE, "Settings".to_string(), None),
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(icon).size(26.0).color(color));
            ui.label(RichText::new(title).size(28.0).family(fonts::heading()).color(TEXT));
        });
        if let Some(s) = sub {
            ui.label(RichText::new(s).size(13.0).color(SUBTLE));
        }
        ui.add_space(14.0);
    }

    fn new_task_row(&mut self, ui: &mut egui::Ui, today: NaiveDate) {
        if let View::Vault(name) = &self.view {
            if let Some(i) = self.cfg.vaults.iter().position(|v| &vault_name(v) == name) {
                self.draft.vault = i;
            }
        }
        if self.draft.vault >= self.cfg.vaults.len() {
            self.draft.vault = 0;
        }
        Frame::new()
            .fill(Color32::from_rgb(248, 248, 250))
            .stroke(Stroke::new(1.0, Color32::from_rgb(230, 230, 234)))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    const ROW_H: f32 = 26.0;
                    ui.set_min_height(ROW_H);
                    // Painted plus sign so it centers exactly on the text line.
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, ROW_H), Sense::hover());
                    let c = r.center();
                    let stroke = Stroke::new(1.6, BLUE);
                    ui.painter().line_segment([c - vec2(5.5, 0.0), c + vec2(5.5, 0.0)], stroke);
                    ui.painter().line_segment([c - vec2(0.0, 5.5), c + vec2(0.0, 5.5)], stroke);
                    let hint = if self.view == View::Today { "New task for today" } else { "New task" };
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.draft.text)
                            .hint_text(hint)
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
                        match &mut self.draft.due {
                            Some(d) => {
                                if ui.add(egui::Button::new("✖").frame(false)).on_hover_text("Clear due date").clicked() {
                                    self.draft.due = None;
                                } else {
                                    let mut jd = jiff::civil::date(d.year() as i16, d.month() as i8, d.day() as i8);
                                    ui.add(egui_extras::DatePickerButton::new(&mut jd).id_salt("due").calendar_week(false));
                                    if let Some(nd) = NaiveDate::from_ymd_opt(jd.year() as i32, jd.month() as u32, jd.day() as u32) {
                                        *d = nd;
                                    }
                                }
                            }
                            None => {
                                if ui
                                    .add(egui::Button::new(RichText::new("📅 Due").color(SUBTLE)).frame(false))
                                    .clicked()
                                {
                                    self.draft.due = Some(today);
                                }
                            }
                        }
                    });
                });
            });
        ui.add_space(10.0);
    }

    fn task_list(&mut self, ui: &mut egui::Ui, today: NaiveDate) -> Option<Action> {
        let view = self.view.clone();
        let visible = self.tasks_for(&view, today);
        let mut action = None;

        if visible.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                let msg = if view == View::Today {
                    "Nothing starred for today. Click ☆ on any task to add it here."
                } else {
                    "No open tasks 🎉"
                };
                ui.label(RichText::new(msg).size(14.0).color(SUBTLE));
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
                    _ => groups.push((key, BLUE, vec![t])),
                }
            }
        } else if view == View::Today {
            groups.push((String::new(), BLUE, visible.clone()));
        } else {
            for name in ["Overdue", "Today", "Next 7 Days", "Later", "No Due Date"] {
                let items: Vec<&Task> = visible.iter().copied().filter(|t| date_bucket(t.due, today) == name).collect();
                if !items.is_empty() {
                    let c = if name == "Overdue" { RED } else { BLUE };
                    groups.push((name.to_string(), c, items));
                }
            }
        }

        let show_file = !self.cfg.group_by_file;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for (name, color, items) in &groups {
                if !name.is_empty() {
                    ui.add_space(10.0);
                    ui.label(RichText::new(name).size(15.0).family(fonts::bold()).color(*color));
                    let r = ui.available_rect_before_wrap();
                    ui.painter().hline(r.x_range(), r.top() + 2.0, Stroke::new(1.0, Color32::from_rgb(228, 228, 232)));
                    ui.add_space(8.0);
                }
                for t in items {
                    if let Some(a) = task_row(ui, t, today, show_file) {
                        action = Some(a);
                    }
                }
            }
            ui.add_space(20.0);
        });
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
                ui.label(RichText::new(msg).size(14.0).color(SUBTLE));
            });
            return None;
        }
        let mut action = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
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
                    ui.label(RichText::new(name).size(15.0).family(fonts::bold()).color(GREEN));
                    let r = ui.available_rect_before_wrap();
                    ui.painter().hline(r.x_range(), r.top() + 2.0, Stroke::new(1.0, Color32::from_rgb(228, 228, 232)));
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
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
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
            if ui.button("+ Add vault folder…").clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    if !self.cfg.vaults.contains(&p) {
                        self.cfg.vaults.push(p);
                        changed = true;
                    }
                }
            }

            ui.add_space(20.0);
            section(ui, "Excluded folders", "Any file whose path contains one of these is skipped (e.g. Recipes or 05 Journal/Recipes).");
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
                ui.label(RichText::new(format!("New tasks will be due {}.", d.format("%A, %b %-d"))).size(12.0).color(SUBTLE));
            }
            if before != (self.cfg.default_due, self.cfg.default_due_days) {
                if self.draft.text.trim().is_empty() {
                    self.draft.due = self.cfg.default_due_date(today);
                }
                changed = true;
            }

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
                            ui.label(RichText::new("Your sounds").size(11.0).color(SUBTLE));
                            for s in mine {
                                ui.selectable_value(&mut self.cfg.chime, Some(s.clone()), s);
                            }
                            ui.separator();
                            ui.label(RichText::new("macOS").size(11.0).color(SUBTLE));
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
                if let Some(s) = &self.cfg.chime {
                    if ui.button("▶ Play").clicked() {
                        play_sound(s);
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Add your own: drop .aiff, .wav, .mp3, .m4a or .caf files into ~/Library/Sounds.")
                        .size(12.0)
                        .color(SUBTLE),
                );
                if ui.small_button("Open folder").clicked() {
                    if let Some(dir) = user_sounds_dir() {
                        let _ = std::fs::create_dir_all(&dir);
                        let _ = std::process::Command::new("open").arg(dir).spawn();
                    }
                }
            });

            ui.add_space(20.0);
            section(ui, "Display", "");
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
                ui.label(RichText::new(fonts::SIZES[size].0).color(SUBTLE));
                if size != self.cfg.text_size {
                    let ctx = ui.ctx().clone();
                    self.set_text_size(&ctx, size);
                }
            });
            ui.label(RichText::new("Shortcuts: ⌘= bigger, ⌘- smaller, ⌘0 normal.").size(12.0).color(SUBTLE));
            ui.add_space(6.0);
            changed |= ui.checkbox(&mut self.cfg.group_by_file, "Group tasks by file instead of due date").changed();
            changed |= ui.checkbox(&mut self.cfg.hide_future_start, "Hide tasks whose start date (🛫) is in the future").changed();

            ui.add_space(20.0);
            section(ui, "Jump to line", "Clicking a task opens its note at the task's line when the free Advanced URI plugin is installed and enabled in that vault (Obsidian → Settings → Community plugins → Browse → \"Advanced URI\"). Otherwise the note opens at the top.");
            for v in &self.cfg.vaults {
                let ok = has_advanced_uri(v);
                ui.label(RichText::new(format!(
                    "{}  {}",
                    if ok { "✔" } else { "✖" },
                    format!("{}: {}", vault_name(v), if ok { "Advanced URI installed" } else { "Advanced URI not installed" })
                ))
                .color(if ok { GREEN } else { SUBTLE }));
            }

            ui.add_space(20.0);
            section(ui, "About", "");
            ui.label(RichText::new(format!(
                "New tasks are added to \"{}\" at the root of the chosen vault.\nStarring a task sets its ⏳ scheduled date to today (Tasks plugin format).\n{} open tasks, last scan took {} ms.",
                tasks::INBOX_FILE,
                self.tasks.iter().filter(|t| !t.completed).count(),
                self.scan_ms
            )).color(SUBTLE));
        });
        if changed {
            self.cfg.save();
            if let View::Vault(v) = &self.view {
                if !self.cfg.vaults.iter().any(|p| &vault_name(p) == v) {
                    self.view = View::All;
                }
            }
            self.rescan();
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str, sub: &str) {
    ui.label(RichText::new(title).size(15.0).family(fonts::bold()).color(BLUE));
    if !sub.is_empty() {
        ui.label(RichText::new(sub).color(SUBTLE));
    }
    ui.add_space(6.0);
}

fn side_item(
    ui: &mut egui::Ui,
    icon: &str,
    icon_color: Color32,
    label: &str,
    count: usize,
    badge: usize,
    selected: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, CornerRadius::same(6), SELECTED);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), HOVER);
    }
    let y = rect.center().y;
    p.text(egui::pos2(rect.left() + 10.0, y), Align2::LEFT_CENTER, icon, FontId::proportional(16.0), icon_color);
    p.text(egui::pos2(rect.left() + 36.0, y), Align2::LEFT_CENTER, label, FontId::proportional(14.5), TEXT);
    let mut right = rect.right() - 10.0;
    if count > 0 {
        let r = p.text(egui::pos2(right, y), Align2::RIGHT_CENTER, count.to_string(), FontId::proportional(13.0), SUBTLE);
        right = r.left() - 8.0;
    }
    if badge > 0 {
        let text = badge.to_string();
        let w = (10.0 + 7.0 * text.len() as f32).max(20.0);
        let b = egui::Rect::from_center_size(egui::pos2(right - w / 2.0, y), vec2(w, 18.0));
        p.rect_filled(b, CornerRadius::same(9), RED);
        p.text(b.center(), Align2::CENTER_CENTER, text, FontId::proportional(11.5), Color32::WHITE);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn task_row(ui: &mut egui::Ui, t: &Task, today: NaiveDate, show_file: bool) -> Option<Action> {
    let mut action = None;
    let starred = t.starred(today);
    ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        let mut checked = false;
        let cb = ui.checkbox(&mut checked, "");
        if t.recurrence.is_some() {
            if cb.on_hover_text("Recurring task: opens in Obsidian so the Tasks plugin can create the next one").clicked() {
                action = Some(Action::Open(t.clone()));
            }
        } else if checked {
            action = Some(Action::Complete(t.clone()));
        }

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.horizontal(|ui| {
                if t.priority != Priority::Normal {
                    ui.label(RichText::new(t.priority.icon()).size(12.0));
                }
                let mut title = RichText::new(&t.description).size(14.5).color(TEXT);
                if t.in_progress {
                    title = title.italics();
                }
                let resp = ui
                    .add(egui::Label::new(title).sense(Sense::click()).truncate())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(format!("Open in Obsidian\n{} / {} (line {})", t.vault, t.rel_path(), t.line + 1));
                if resp.clicked() {
                    action = Some(Action::Open(t.clone()));
                }
            });
            if show_file {
                let file = t.file.file_stem().unwrap_or_default().to_string_lossy();
                let mut sub = format!("{} · {}", t.vault, file);
                if let Some(r) = &t.recurrence {
                    sub.push_str(&format!("  ·  🔁 {r}"));
                }
                ui.label(RichText::new(sub).size(11.5).color(SUBTLE));
            }
        });

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (icon, color, tip) = if starred {
                ("★", STAR, "Remove from Today")
            } else {
                ("☆", Color32::from_rgb(195, 195, 200), "Add to Today")
            };
            if ui
                .add(egui::Button::new(RichText::new(icon).size(18.0).color(color)).frame(false))
                .on_hover_text(tip)
                .clicked()
            {
                action = Some(Action::Star(t.clone(), !starred));
            }
            let resp = match t.due {
                Some(d) => date_chip(ui, d, today),
                None => ui
                    .add(egui::Button::new(RichText::new("📅").size(13.0).color(Color32::from_rgb(195, 195, 200))).frame(false))
                    .on_hover_text("Add due date"),
            };
            if let Some(a) = due_menu(&resp, t, today) {
                action = Some(a);
            }
        });
    });
    action
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
            let title = RichText::new(&t.description).size(14.5).color(SUBTLE).strikethrough();
            let resp = ui
                .add(egui::Label::new(title).sense(Sense::click()).truncate())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("Open in Obsidian\n{} / {} (line {})", t.vault, t.rel_path(), t.line + 1));
            if resp.clicked() {
                action = Some(Action::Open(t.clone()));
            }
            let file = t.file.file_stem().unwrap_or_default().to_string_lossy();
            ui.label(RichText::new(format!("{} · {}", t.vault, file)).size(11.5).color(SUBTLE));
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
        (RED, Color32::from_rgb(253, 232, 235))
    } else if days == 0 {
        (Color32::from_rgb(200, 120, 0), Color32::from_rgb(255, 242, 220))
    } else {
        (Color32::from_rgb(110, 110, 116), Color32::from_rgb(238, 238, 241))
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

/// Due date menu opened from a task's date chip (or calendar button).
fn due_menu(resp: &egui::Response, t: &Task, today: NaiveDate) -> Option<Action> {
    let mut out = None;
    egui::Popup::menu(resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(230.0);
            ui.set_max_width(230.0);
            ui.label(RichText::new("Due date").size(12.0).family(fonts::bold()).color(SUBTLE));
            let quick = [
                ("Today", today),
                ("Tomorrow", today + chrono::Days::new(1)),
                ("End of week", end_of_week(today)),
                ("Next Monday", next_monday(today)),
                ("In 1 week", today + chrono::Days::new(7)),
            ];
            for (label, d) in quick {
                let b = egui::Button::new(RichText::new(label).color(TEXT))
                    .right_text(RichText::new(d.format("%a %b %-d").to_string()).color(SUBTLE))
                    .frame(false);
                if ui.add(b).clicked() {
                    out = Some(Action::SetDue(t.clone(), Some(d)));
                    ui.close();
                }
            }
            ui.separator();
            if let Some(d) = calendar(ui, resp.id.with("cal"), t.due, today) {
                out = Some(Action::SetDue(t.clone(), Some(d)));
                ui.close();
            }
            if t.due.is_some() {
                ui.separator();
                if ui.add(egui::Button::new(RichText::new("Remove due date").color(RED)).frame(false)).clicked() {
                    out = Some(Action::SetDue(t.clone(), None));
                    ui.close();
                }
            }
        });
    out
}

/// Month calendar. Returns the clicked day.
fn calendar(ui: &mut egui::Ui, id: egui::Id, selected: Option<NaiveDate>, today: NaiveDate) -> Option<NaiveDate> {
    let start = selected.unwrap_or(today);
    let mut month: NaiveDate = ui.data_mut(|d| {
        *d.get_temp_mut_or_insert_with(id, || start.with_day(1).unwrap())
    });
    let mut picked = None;
    ui.horizontal(|ui| {
        if ui.add(egui::Button::new("‹").frame(false)).clicked() {
            month = (month - chrono::Days::new(1)).with_day(1).unwrap();
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add(egui::Button::new("›").frame(false)).clicked() {
                month = month + chrono::Months::new(1);
            }
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(month.format("%B %Y").to_string()).family(fonts::bold()).color(TEXT));
            });
        });
    });
    egui::Grid::new(id.with("grid")).spacing([2.0, 2.0]).show(ui, |ui| {
        for w in ["M", "T", "W", "T", "F", "S", "S"] {
            ui.centered_and_justified(|ui| ui.label(RichText::new(w).size(11.0).color(SUBTLE)));
        }
        ui.end_row();
        let offset = month.weekday().num_days_from_monday();
        for _ in 0..offset {
            ui.label("");
        }
        let mut col = offset;
        let mut day = month;
        while day.month() == month.month() {
            let is_sel = Some(day) == selected;
            let color = if is_sel {
                Color32::WHITE
            } else if day == today {
                BLUE
            } else {
                TEXT
            };
            let mut b = egui::Button::new(RichText::new(day.day().to_string()).size(12.5).color(color))
                .min_size(vec2(28.0, 24.0))
                .corner_radius(CornerRadius::same(5));
            b = if is_sel { b.fill(BLUE) } else { b.frame(false) };
            if ui.add(b).clicked() {
                picked = Some(day);
            }
            col += 1;
            if col % 7 == 0 {
                ui.end_row();
            }
            day = day + chrono::Days::new(1);
        }
    });
    ui.data_mut(|d| d.insert_temp(id, month));
    picked
}

/// True if the vault has the Advanced URI community plugin installed and enabled,
/// which lets us open a note at a specific line.
fn has_advanced_uri(vault_root: &std::path::Path) -> bool {
    let obs = vault_root.join(".obsidian");
    obs.join("plugins/obsidian-advanced-uri").is_dir()
        && std::fs::read_to_string(obs.join("community-plugins.json"))
            .is_ok_and(|s| s.contains("\"obsidian-advanced-uri\""))
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
    ctx.set_visuals(egui::Visuals::light());
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(8.0, 4.0);
        s.spacing.interact_size.y = 22.0;
        s.visuals.panel_fill = MAIN_BG;
        s.visuals.selection.bg_fill = Color32::from_rgb(200, 222, 255);
        s.visuals.widgets.inactive.corner_radius = CornerRadius::same(5);
        s.visuals.widgets.hovered.corner_radius = CornerRadius::same(5);
        s.visuals.widgets.active.corner_radius = CornerRadius::same(5);
    });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.last_scan.elapsed() >= RESCAN_EVERY {
            self.rescan();
        }
        ui.ctx().request_repaint_after(RESCAN_EVERY);
        let ctx = ui.ctx().clone();
        self.handle_menu(&ctx);

        egui::Panel::left("sidebar")
            .exact_size(230.0)
            .resizable(false)
            .frame(Frame::new().fill(SIDEBAR_BG).inner_margin(Margin::symmetric(12, 10)))
            .show(ui, |ui| self.sidebar(ui));

        let today = Local::now().date_naive();
        let mut action = None;
        egui::CentralPanel::default()
            .frame(Frame::new().fill(MAIN_BG).inner_margin(Margin { left: 40, right: 40, top: 28, bottom: 0 }))
            .show(ui, |ui| {
                self.header(ui, today);
                if let Some(e) = &self.error {
                    ui.label(RichText::new(format!("⚠ {e}")).color(RED));
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
                    self.new_task_row(ui, today);
                    action = self.task_list(ui, today);
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
                            .fill(Color32::from_rgb(40, 40, 46))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(Margin::symmetric(16, 10))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let mut text = last.description.clone();
                                    if text.chars().count() > 48 {
                                        text = text.chars().take(47).collect::<String>() + "…";
                                    }
                                    ui.label(RichText::new("✔").color(GREEN));
                                    ui.label(RichText::new(format!("Completed \"{text}\"")).color(Color32::WHITE));
                                    ui.add_space(8.0);
                                    if ui
                                        .add(egui::Button::new(RichText::new("Undo").color(Color32::from_rgb(140, 200, 255))).frame(false))
                                        .clicked()
                                    {
                                        undo = true;
                                    }
                                });
                            });
                    });
            }
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
