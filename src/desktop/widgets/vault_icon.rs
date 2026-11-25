//! Render protocol glyphs and encode fixed-size pixels for the WebView.

use crate::{
    desktop::widgets::symbol::{Icon, Symbol},
    sync::protocol::{SPACE_ICON_SIZE, SpaceGlyph, SpaceIcon},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use dioxus::prelude::*;
use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};

#[component]
pub(crate) fn VaultIcon(icon: Option<SpaceIcon>, fallback: Icon) -> Element {
    match icon.as_ref() {
        Some(SpaceIcon::Glyph(glyph)) => rsx! { Symbol { icon: glyph_symbol(*glyph) } },
        Some(icon @ SpaceIcon::Image(pixels)) if icon.valid() => {
            let mut png = Vec::new();
            if PngEncoder::new(&mut png)
                .write_image(
                    &pixels.0,
                    SPACE_ICON_SIZE,
                    SPACE_ICON_SIZE,
                    ExtendedColorType::Rgba8,
                )
                .is_ok()
            {
                let src = format!("data:image/png;base64,{}", STANDARD.encode(png));
                return rsx! { img { class: "vault-icon-image", src, alt: "" } };
            }
            rsx! { Symbol { icon: fallback } }
        }
        _ => rsx! { Symbol { icon: fallback } },
    }
}

fn glyph_symbol(glyph: SpaceGlyph) -> Icon {
    match glyph {
        SpaceGlyph::Folder => Icon::Folder,
        SpaceGlyph::Cloud => Icon::Cloud,
        SpaceGlyph::Key => Icon::Key,
        SpaceGlyph::User => Icon::User,
        SpaceGlyph::Sparkles => Icon::Sparkles,
    }
}
