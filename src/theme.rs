//! Light and dark color palettes. Colors are functions so they follow the current theme.
use eframe::egui::{self, Color32, CornerRadius};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

static DARK: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default, Debug)]
pub enum Appearance {
    /// Follow the macOS setting.
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Appearance::System => "Match system",
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }

    /// Whether this appearance resolves to dark right now.
    pub fn is_dark(self, ctx: &egui::Context) -> bool {
        match self {
            Appearance::System => ctx.system_theme() == Some(egui::Theme::Dark),
            Appearance::Light => false,
            Appearance::Dark => true,
        }
    }
}

pub fn is_dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

fn pick(light: Color32, dark: Color32) -> Color32 {
    if is_dark() { dark } else { light }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

// Surfaces
pub fn sidebar_bg() -> Color32 { pick(rgb(244, 244, 246), rgb(32, 32, 36)) }
pub fn main_bg() -> Color32 { pick(Color32::WHITE, rgb(24, 24, 27)) }
pub fn field_bg() -> Color32 { pick(rgb(248, 248, 250), rgb(34, 34, 38)) }
pub fn field_stroke() -> Color32 { pick(rgb(230, 230, 234), rgb(54, 54, 60)) }
pub fn selected() -> Color32 { pick(rgb(222, 222, 227), rgb(58, 58, 65)) }
pub fn hover() -> Color32 { pick(rgb(233, 233, 237), rgb(46, 46, 52)) }
pub fn divider() -> Color32 { pick(rgb(222, 222, 228), rgb(58, 58, 64)) }
/// Line under group headings.
pub fn rule() -> Color32 { pick(rgb(228, 228, 232), rgb(48, 48, 54)) }
pub fn toast_bg() -> Color32 { pick(rgb(40, 40, 46), rgb(58, 58, 66)) }

// Text
pub fn text() -> Color32 { pick(rgb(30, 30, 32), rgb(232, 232, 236)) }
pub fn subtle() -> Color32 { pick(rgb(150, 150, 156), rgb(140, 140, 148)) }
/// Very light marks (empty star, add-date button).
pub fn faint() -> Color32 { pick(rgb(195, 195, 200), rgb(92, 92, 100)) }

// Accents
pub fn blue() -> Color32 { pick(rgb(28, 110, 220), rgb(84, 158, 245)) }
pub fn red() -> Color32 { pick(rgb(232, 64, 82), rgb(245, 96, 110)) }
pub fn green() -> Color32 { pick(rgb(52, 168, 90), rgb(64, 186, 106)) }
pub fn star() -> Color32 { rgb(250, 190, 20) }
pub fn amber() -> Color32 { pick(rgb(222, 140, 20), rgb(236, 160, 50)) }
pub fn link() -> Color32 { rgb(140, 200, 255) }

/// Date chip colors: (text, background) for overdue, today and later.
pub fn chip_overdue() -> (Color32, Color32) { (red(), pick(rgb(253, 232, 235), rgb(72, 32, 38))) }
pub fn chip_today() -> (Color32, Color32) { (pick(rgb(200, 120, 0), rgb(240, 170, 60)), pick(rgb(255, 242, 220), rgb(70, 52, 24))) }
pub fn chip_later() -> (Color32, Color32) { (pick(rgb(110, 110, 116), rgb(170, 170, 178)), pick(rgb(238, 238, 241), rgb(48, 48, 54))) }

/// Switches the palette and egui's own widget colors.
pub fn apply(ctx: &egui::Context, dark: bool) {
    DARK.store(dark, Ordering::Relaxed);
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = main_bg();
    v.window_fill = pick(Color32::WHITE, rgb(36, 36, 41));
    v.extreme_bg_color = pick(Color32::WHITE, rgb(44, 44, 50)); // text fields
    v.selection.bg_fill = pick(rgb(200, 222, 255), rgb(40, 80, 140));
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
        w.corner_radius = CornerRadius::same(5);
    }
    // The text cursor (caret): our accent blue, 2pt wide, blinking.
    v.text_cursor = egui::style::TextCursorStyle {
        stroke: egui::Stroke::new(2.0, blue()),
        blink: true,
        ..Default::default()
    };
    // egui keeps a separate style for its light and dark themes and picks one from the
    // macOS setting, which can differ from the Appearance chosen in Settings.
    // `ctx.set_visuals` only updates the style egui thinks is active, so text fields
    // could end up drawn with egui's default cursor color (e.g. light blue on white).
    // Setting both makes every widget use our palette whichever one egui picks.
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        ctx.set_visuals_of(theme, v.clone());
    }
}
