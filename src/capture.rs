//! System-wide quick-capture shortcut (default Ctrl+Option+Space).
//!
//! Uses the `global-hotkey` crate, which registers the shortcut with macOS's standard
//! hotkey API. That API doesn't need Accessibility or Input Monitoring permission;
//! registration can still fail, usually because another app already uses the shortcut.
use eframe::egui;
use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::sync::mpsc::{Receiver, channel};

pub const DEFAULT_SHORTCUT: &str = "ctrl+option+space";

pub struct Hotkey {
    /// Must stay alive (and be created on the main thread) for the shortcut to keep working.
    manager: Option<GlobalHotKeyManager>,
    current: Option<HotKey>,
    /// IDs of shortcuts pressed since the last frame.
    rx: Receiver<u32>,
    /// Why the shortcut isn't working, shown in Settings.
    pub error: Option<String>,
}

impl Hotkey {
    pub fn new(ctx: &egui::Context, shortcut: &str) -> Self {
        let (tx, rx) = channel();
        let repaint = ctx.clone();
        // Called on another thread when any registered shortcut is pressed. It passes
        // the ID to the UI and wakes it, even if the window is in the background.
        GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
            if e.state() == HotKeyState::Pressed {
                let _ = tx.send(e.id());
                repaint.request_repaint();
            }
        }));
        let (manager, error) = match GlobalHotKeyManager::new() {
            Ok(m) => (Some(m), None),
            Err(e) => (None, Some(format!("Global shortcuts aren't available: {e}"))),
        };
        let mut h = Hotkey { manager, current: None, rx, error };
        let _ = h.set(shortcut);
        h
    }

    /// Replaces the shortcut. On failure the error is kept for Settings and returned.
    pub fn set(&mut self, shortcut: &str) -> Result<(), String> {
        let Some(manager) = &self.manager else {
            return Err(self.error.clone().unwrap_or_default());
        };
        if let Some(old) = self.current.take() {
            let _ = manager.unregister(old);
        }
        let result = shortcut
            .parse::<HotKey>()
            .map_err(|_| format!("\"{shortcut}\" isn't a shortcut. Try something like ctrl+option+space."))
            .and_then(|hk| {
                manager
                    .register(hk)
                    .map(|()| hk)
                    .map_err(|_| {
                        format!(
                            "Couldn't use {} as the capture shortcut. Another app probably uses it already; pick a different one.",
                            pretty(shortcut)
                        )
                    })
            });
        match result {
            Ok(hk) => {
                self.current = Some(hk);
                self.error = None;
                Ok(())
            }
            Err(e) => {
                self.error = Some(e.clone());
                Err(e)
            }
        }
    }

    /// True if the shortcut was pressed since the last call.
    pub fn pressed(&self) -> bool {
        let Some(hk) = self.current else { return false };
        // `try_iter` reads everything waiting without blocking; `count` drains it all.
        self.rx.try_iter().filter(|id| *id == hk.id()).count() > 0
    }
}

/// "ctrl+option+space" as macOS shows it: ⌃⌥Space.
pub fn pretty(shortcut: &str) -> String {
    let mut out = String::new();
    for part in shortcut.split('+').map(str::trim) {
        let s = match part.to_lowercase().as_str() {
            "ctrl" | "control" => "⌃".to_string(),
            "alt" | "option" => "⌥".to_string(),
            "shift" => "⇧".to_string(),
            "cmd" | "command" | "super" | "meta" => "⌘".to_string(),
            other => {
                // Capitalize the key name: "space" -> "Space", "k" -> "K".
                let mut c = other.chars();
                match c.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                    None => String::new(),
                }
            }
        };
        out.push_str(&s);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn pretty_shortcuts() {
        assert_eq!(super::pretty("ctrl+option+space"), "⌃⌥Space");
        assert_eq!(super::pretty("cmd+shift+k"), "⌘⇧K");
    }

    #[test]
    fn default_parses() {
        assert!(super::DEFAULT_SHORTCUT.parse::<global_hotkey::hotkey::HotKey>().is_ok());
    }
}
