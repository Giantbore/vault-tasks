//! Native macOS menu bar.

use eframe::egui;
use muda::{
    AboutMetadata, CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, channel};

#[derive(Clone, Copy, Debug)]
pub enum Cmd {
    Settings,
    NewTask,
    Refresh,
    ShowAll,
    ShowToday,
    ShowCompleted,
    GroupByFile,
    Bigger,
    Smaller,
    ActualSize,
}

pub struct MenuBar {
    _menu: Menu,
    rx: Receiver<MenuEvent>,
    ids: HashMap<MenuId, Cmd>,
    group_by_file: CheckMenuItem,
}

fn cmd_key(code: Code) -> Option<Accelerator> {
    Some(Accelerator::new(Modifiers::META, code))
}

impl MenuBar {
    pub fn install(ctx: &egui::Context) -> Self {
        let mut ids = HashMap::new();
        let mut item = |label: &str, accel: Option<Accelerator>, cmd: Cmd| {
            let it = MenuItem::new(label, true, accel);
            ids.insert(it.id().clone(), cmd);
            it
        };

        let settings = item("Settings…", cmd_key(Code::Comma), Cmd::Settings);
        let new_task = item("New Task", cmd_key(Code::KeyN), Cmd::NewTask);
        let refresh = item("Refresh", cmd_key(Code::KeyR), Cmd::Refresh);
        let all = item("All Tasks", cmd_key(Code::Digit1), Cmd::ShowAll);
        let today = item("Today", cmd_key(Code::Digit2), Cmd::ShowToday);
        let completed = item("Completed", cmd_key(Code::Digit3), Cmd::ShowCompleted);
        let bigger = item("Bigger Text", cmd_key(Code::Equal), Cmd::Bigger);
        let smaller = item("Smaller Text", cmd_key(Code::Minus), Cmd::Smaller);
        let actual = item("Normal Text Size", cmd_key(Code::Digit0), Cmd::ActualSize);
        let group_by_file = CheckMenuItem::new("Group by File", true, false, None);
        ids.insert(group_by_file.id().clone(), Cmd::GroupByFile);

        let sep = PredefinedMenuItem::separator;
        let app_menu = Submenu::with_items(
            "Vault Tasks",
            true,
            &[
                &PredefinedMenuItem::about(
                    None,
                    Some(AboutMetadata {
                        name: Some("Vault Tasks".into()),
                        version: Some(env!("CARGO_PKG_VERSION").into()),
                        comments: Some("Open tasks from your Obsidian vaults.".into()),
                        ..Default::default()
                    }),
                ),
                &sep(),
                &settings,
                &sep(),
                &PredefinedMenuItem::services(None),
                &sep(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &sep(),
                &PredefinedMenuItem::quit(None),
            ],
        )
        .expect("app menu");
        let file_menu = Submenu::with_items(
            "File",
            true,
            &[&new_task, &refresh, &sep(), &PredefinedMenuItem::close_window(None)],
        )
        .expect("file menu");
        let view_menu = Submenu::with_items(
            "View",
            true,
            &[&all, &today, &completed, &sep(), &group_by_file, &sep(), &bigger, &smaller, &actual],
        )
        .expect("view menu");
        let window_menu = Submenu::with_items(
            "Window",
            true,
            &[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::maximize(None)],
        )
        .expect("window menu");

        let menu = Menu::with_items(&[&app_menu, &file_menu, &view_menu, &window_menu]).expect("menu");
        #[cfg(target_os = "macos")]
        {
            menu.init_for_nsapp();
            window_menu.set_as_windows_menu_for_nsapp();
        }

        let (tx, rx) = channel();
        let repaint = ctx.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = tx.send(e);
            repaint.request_repaint();
        }));

        MenuBar { _menu: menu, rx, ids, group_by_file }
    }

    /// Commands clicked since the last frame.
    pub fn poll(&self) -> Vec<Cmd> {
        self.rx.try_iter().filter_map(|e| self.ids.get(e.id()).copied()).collect()
    }

    pub fn sync(&self, group_by_file: bool) {
        if self.group_by_file.is_checked() != group_by_file {
            self.group_by_file.set_checked(group_by_file);
        }
    }
}
