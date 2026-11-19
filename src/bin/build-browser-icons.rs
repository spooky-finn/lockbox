//! Rasterize the brand mark for browsers and, on macOS, the desktop bundle.
//! Run from the repository root: `cargo run --locked --bin build-browser-icons`.

use resvg::{tiny_skia, usvg};
use std::{error::Error, fs, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let svg = fs::read(root.join("assets/lockbox-mark.svg"))?;
    let tree = usvg::Tree::from_data(&svg, &usvg::Options::default())?;
    let out = root.join("ext/icons");
    fs::create_dir_all(&out)?;
    for size in [48, 128] {
        render_png(&tree, size, &out.join(format!("{size}.png")))?;
    }

    #[cfg(target_os = "macos")]
    build_macos_icon(root, &tree)?;

    Ok(())
}

fn render_png(tree: &usvg::Tree, size: u32, path: &Path) -> Result<(), Box<dyn Error>> {
    let mut image = tiny_skia::Pixmap::new(size, size).ok_or("invalid icon size")?;
    let scale = size as f32 / tree.size().width();
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut image.as_mut(),
    );
    image.save_png(path)?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn build_macos_icon(root: &Path, tree: &usvg::Tree) -> Result<(), Box<dyn Error>> {
    let iconset = root.join("target/Lockbox.iconset");
    fs::create_dir_all(&iconset)?;
    for size in [16, 32, 128, 256, 512] {
        for (suffix, scale) in [("", 1), ("@2x", 2)] {
            let path = iconset.join(format!("icon_{size}x{size}{suffix}.png"));
            render_png(tree, size * scale, &path)?;
        }
    }

    let status = std::process::Command::new("iconutil")
        .args(["-c", "icns", "-o"])
        .arg(root.join("assets/lockbox.icns"))
        .arg(iconset)
        .status()?;
    if !status.success() {
        return Err("iconutil could not create the Lockbox app icon".into());
    }
    Ok(())
}
