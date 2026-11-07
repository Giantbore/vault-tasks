use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontTweak, epaint::text::VariationCoords};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// Bundled under the SIL Open Font License (see assets/fonts/README.md).
const MANROPE: &[u8] = include_bytes!("../assets/fonts/Manrope.ttf");
const SOURCE_SERIF: &[u8] = include_bytes!("../assets/fonts/SourceSerif4.ttf");
const SF_PATH: &str = "/System/Library/Fonts/SFNS.ttf";

/// Font used for headings (page titles).
pub fn heading() -> FontFamily {
    FontFamily::Name("heading".into())
}

/// Semibold version of the body font (group headers, sidebar labels).
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default, Debug)]
pub enum BodyFont {
    #[default]
    Manrope,
    SanFrancisco,
    SourceSerif,
}

impl BodyFont {
    pub const ALL: [BodyFont; 3] = [BodyFont::Manrope, BodyFont::SanFrancisco, BodyFont::SourceSerif];

    pub fn label(self) -> &'static str {
        match self {
            BodyFont::Manrope => "Manrope",
            BodyFont::SanFrancisco => "San Francisco (system)",
            BodyFont::SourceSerif => "Source Serif 4",
        }
    }
}

/// Text size steps shown in Settings: (label, zoom factor).
pub const SIZES: [(&str, f32); 4] = [("Small", 0.9), ("Normal", 1.0), ("Large", 1.15), ("Larger", 1.3)];
pub const DEFAULT_SIZE: usize = 1;

fn variable(data: FontData, axes: &[(&[u8; 4], f32)]) -> Arc<FontData> {
    Arc::new(data.tweak(FontTweak {
        coords: VariationCoords::new(axes.iter().map(|(t, v)| (*t, *v))),
        ..Default::default()
    }))
}

pub fn apply(ctx: &egui::Context, body: BodyFont) {
    let mut defs = FontDefinitions::default();
    // egui's built-in fonts stay as fallbacks for symbols and emoji (★ ⟳ 📥 …).
    let fallbacks = defs.families[&FontFamily::Proportional].clone();
    let fd = &mut defs.font_data;

    fd.insert("manrope".into(), variable(FontData::from_static(MANROPE), &[(b"wght", 500.0)]));
    fd.insert("manrope-bold".into(), variable(FontData::from_static(MANROPE), &[(b"wght", 700.0)]));
    fd.insert("serif".into(), variable(FontData::from_static(SOURCE_SERIF), &[(b"wght", 420.0), (b"opsz", 16.0)]));
    fd.insert("serif-bold".into(), variable(FontData::from_static(SOURCE_SERIF), &[(b"wght", 640.0), (b"opsz", 16.0)]));
    fd.insert("serif-heading".into(), variable(FontData::from_static(SOURCE_SERIF), &[(b"wght", 600.0), (b"opsz", 36.0)]));

    let mut body = body;
    if body == BodyFont::SanFrancisco {
        match std::fs::read(SF_PATH) {
            Ok(bytes) => {
                fd.insert("sf".into(), variable(FontData::from_owned(bytes.clone()), &[(b"wght", 400.0)]));
                fd.insert("sf-bold".into(), variable(FontData::from_owned(bytes), &[(b"wght", 600.0)]));
            }
            Err(_) => body = BodyFont::Manrope,
        }
    }
    let (regular, semibold) = match body {
        BodyFont::Manrope => ("manrope", "manrope-bold"),
        BodyFont::SanFrancisco => ("sf", "sf-bold"),
        BodyFont::SourceSerif => ("serif", "serif-bold"),
    };

    let with_fallbacks = |first: &str| {
        let mut v = vec![first.to_string()];
        v.extend(fallbacks.iter().cloned());
        v
    };
    defs.families.insert(FontFamily::Proportional, with_fallbacks(regular));
    defs.families.insert(bold(), with_fallbacks(semibold));
    defs.families.insert(heading(), with_fallbacks("serif-heading"));
    ctx.set_fonts(defs);
}
