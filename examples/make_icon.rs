//! Renders assets/icon.svg into assets/AppIcon.icns (app bundle icon)
//! assets/icon.png (Dock icon set by the app at runtime) and
//! assets/icon-small.png (logo used inside the app's sidebar and headers).
//! Run after editing the SVG: `cargo run --example make_icon`
use resvg::{tiny_skia, usvg};
use std::process::Command;

fn main() {
    let root = env!("CARGO_MANIFEST_DIR");
    let svg = std::fs::read(format!("{root}/assets/icon.svg")).expect("read icon.svg");
    let tree = usvg::Tree::from_data(&svg, &usvg::Options::default()).expect("parse icon.svg");

    let iconset = format!("{root}/target/AppIcon.iconset");
    let _ = std::fs::remove_dir_all(&iconset);
    std::fs::create_dir_all(&iconset).unwrap();

    // macOS iconset: each base size at 1x and 2x.
    for base in [16u32, 32, 128, 256, 512] {
        for scale in [1u32, 2] {
            let px = base * scale;
            let mut pixmap = tiny_skia::Pixmap::new(px, px).unwrap();
            let s = px as f32 / tree.size().width();
            resvg::render(&tree, tiny_skia::Transform::from_scale(s, s), &mut pixmap.as_mut());
            let name = if scale == 1 {
                format!("icon_{base}x{base}.png")
            } else {
                format!("icon_{base}x{base}@2x.png")
            };
            pixmap.save_png(format!("{iconset}/{name}")).unwrap();
            if px == 512 {
                pixmap.save_png(format!("{root}/assets/icon.png")).unwrap();
            }
            if px == 128 && scale == 1 {
                pixmap.save_png(format!("{root}/assets/icon-small.png")).unwrap();
            }
        }
    }

    let out = format!("{root}/assets/AppIcon.icns");
    let status = Command::new("iconutil")
        .args(["-c", "icns", &iconset, "-o", &out])
        .status()
        .expect("run iconutil");
    assert!(status.success(), "iconutil failed");
    println!("Wrote {out}");
}
