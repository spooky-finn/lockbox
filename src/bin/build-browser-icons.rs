//! Rasterize the existing brand vector for browsers that require PNG icons.
//! Run from the repository root: `cargo run --locked --bin build-browser-icons`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let svg = std::fs::read(root.join("assets/lockbox-mark.svg"))?;
    let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default())?;
    let out = root.join("browser-extension/icons");
    std::fs::create_dir_all(&out)?;
    for size in [48, 128] {
        let mut image = resvg::tiny_skia::Pixmap::new(size, size).ok_or("invalid icon size")?;
        let scale = size as f32 / tree.size().width();
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut image.as_mut(),
        );
        image.save_png(out.join(format!("{size}.png")))?;
    }
    Ok(())
}
