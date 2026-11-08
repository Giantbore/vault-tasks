//! Renders the app icons. Run after editing an SVG: `cargo run --example make_icon`
//!
//! From assets/icon.svg (dark):
//! - assets/AppIcon.icns    app bundle icon (Finder, Spotlight)
//! - assets/icon.png        Dock icon in dark mode
//! - assets/icon-small.png  in-app logo in dark mode
//!
//! From assets/icon-light.svg (light):
//! - assets/icon-light.png        Dock icon in light mode
//! - assets/icon-small-light.png  in-app logo in light mode
use resvg::{tiny_skia, usvg};
use std::process::Command;

fn load(path: &str) -> usvg::Tree {
    let svg = std::fs::read(path).unwrap_or_else(|_| panic!("read {path}"));
    usvg::Tree::from_data(&svg, &usvg::Options::default()).unwrap_or_else(|_| panic!("parse {path}"))
}

fn render(tree: &usvg::Tree, px: u32) -> tiny_skia::Pixmap {
    let mut pixmap = tiny_skia::Pixmap::new(px, px).unwrap();
    let s = px as f32 / tree.size().width();
    resvg::render(tree, tiny_skia::Transform::from_scale(s, s), &mut pixmap.as_mut());
    pixmap
}

fn main() {
    let root = env!("CARGO_MANIFEST_DIR");
    let dark = load(&format!("{root}/assets/icon.svg"));
    let light = load(&format!("{root}/assets/icon-light.svg"));

    let iconset = format!("{root}/target/AppIcon.iconset");
    let _ = std::fs::remove_dir_all(&iconset);
    std::fs::create_dir_all(&iconset).unwrap();

    // macOS iconset: each base size at 1x and 2x.
    for base in [16u32, 32, 128, 256, 512] {
        for scale in [1u32, 2] {
            let name = if scale == 1 {
                format!("icon_{base}x{base}.png")
            } else {
                format!("icon_{base}x{base}@2x.png")
            };
            render(&dark, base * scale).save_png(format!("{iconset}/{name}")).unwrap();
        }
    }

    render(&dark, 512).save_png(format!("{root}/assets/icon.png")).unwrap();
    render(&dark, 128).save_png(format!("{root}/assets/icon-small.png")).unwrap();
    render(&light, 512).save_png(format!("{root}/assets/icon-light.png")).unwrap();
    render(&light, 128).save_png(format!("{root}/assets/icon-small-light.png")).unwrap();

    let out = format!("{root}/assets/AppIcon.icns");
    let status = Command::new("iconutil")
        .args(["-c", "icns", &iconset, "-o", &out])
        .status()
        .expect("run iconutil");
    assert!(status.success(), "iconutil failed");
    println!("Wrote {out} and PNGs");
}
