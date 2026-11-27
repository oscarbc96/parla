//! Renders `assets/brand/logo-dark.svg` to `assets/icons/app/icon-{32,128,256,512}.png`
//! with resvg. Run it with `mise run assets:icons`.

use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let svg = std::fs::read(root.join("assets/brand/logo-dark.svg")).expect("read logo-dark.svg");
    let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default())
        .expect("parse logo-dark.svg");
    for px in [32u32, 128, 256, 512] {
        let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px).expect("pixmap");
        let scale = px as f32 / tree.size().width();
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        let out = root.join(format!("assets/icons/app/icon-{px}.png"));
        pixmap.save_png(&out).expect("write png");
        println!("wrote {}", out.display());
    }
}
