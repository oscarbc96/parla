//! Renders `assets/icons/tray/{tray,tray-dot}.svg` to 44 px PNG files (22 pt at 2x,
//! the status item height; a taller image is scaled down to it) with resvg. The
//! bubble is 32 px (16 pt) centred in that canvas, so it has the weight of the
//! system's own items. The images are macOS template images: only their alpha
//! matters.
//! Run it with `mise run assets:tray`.

use std::path::Path;

/// Canvas side in pixels (22 pt at 2x) and bubble side in pixels (16 pt at 2x).
const CANVAS: u32 = 44;
const GLYPH: f32 = 32.0;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons/tray");
    for name in ["tray", "tray-dot"] {
        let svg = std::fs::read(dir.join(format!("{name}.svg"))).expect("read svg");
        let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default())
            .expect("parse svg");
        let mut pixmap = resvg::tiny_skia::Pixmap::new(CANVAS, CANVAS).expect("pixmap");
        let scale = GLYPH / tree.size().width();
        let pad = (CANVAS as f32 - GLYPH) / 2.0;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(pad, pad),
            &mut pixmap.as_mut(),
        );
        let out = dir.join(format!("{name}.png"));
        pixmap.save_png(&out).expect("write png");
        println!("wrote {}", out.display());
    }
}
