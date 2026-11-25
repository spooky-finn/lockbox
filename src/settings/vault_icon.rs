//! Native image selection and controls for encrypted vault icons.

use crate::{
    desktop::{
        context::Session,
        i18n::{Text, use_language},
        session::Screen,
        widgets::{symbol::Icon, vault_icon::VaultIcon},
    },
    settings::state::Settings,
    sync::protocol::{Blob, SPACE_ICON_SIZE, SpaceGlyph, SpaceIcon},
};
use dioxus::prelude::*;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits, RgbaImage};
use std::{
    fs::OpenOptions,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

const MAX_IMAGE_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 4096;
const CHOICES: [(SpaceGlyph, &str, Text); 5] = [
    (SpaceGlyph::Folder, "folder", Text::Folder),
    (SpaceGlyph::Cloud, "cloud", Text::VaultIconCloud),
    (SpaceGlyph::Key, "key", Text::VaultIconKey),
    (SpaceGlyph::User, "user", Text::VaultIconPerson),
    (SpaceGlyph::Sparkles, "sparkles", Text::VaultIconSparkles),
];

#[component]
pub(super) fn VaultIconSetting(
    icon: Option<SpaceIcon>,
    shared: bool,
    editable: bool,
    disabled: bool,
) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let loading = use_signal(|| false);
    let custom_image = matches!(icon, Some(SpaceIcon::Image(_)));
    let selected = match icon.as_ref() {
        None => "default",
        Some(SpaceIcon::Image(_)) => "image",
        Some(SpaceIcon::Glyph(glyph)) => {
            CHOICES.iter().find(|choice| choice.0 == *glyph).unwrap().1
        }
    };
    let disabled = disabled || loading();
    let fallback = if shared { Icon::Cloud } else { Icon::Folder };
    rsx! {
        div { class: "preference-row vault-icon-setting",
            span { class: "vault-icon-preview", aria_hidden: "true", VaultIcon { icon, fallback } }
            div { class: "preference-copy vault-icon-copy",
                if editable {
                    label { r#for: "settings-vault-icon", {language.text(Text::VaultIcon)} }
                } else {
                    span { {language.text(Text::VaultIcon)} }
                }
                if shared {
                    p { id: "vault-icon-scope", class: "muted",
                        {language.text(if editable { Text::SharedVaultIcon } else { Text::OwnerSetsVaultIcon })}
                    }
                }
            }
            if editable {
                div { class: "vault-icon-controls",
                    select { id: "settings-vault-icon", value: selected, disabled,
                        aria_describedby: shared.then_some("vault-icon-scope"),
                        onchange: move |event| {
                            let value = event.value();
                            if value == "default" {
                                apply_icon(state, Ok(None));
                            } else if let Some((glyph, _, _)) = CHOICES.iter().find(|choice| choice.1 == value) {
                                apply_icon(state, Ok(Some(SpaceIcon::Glyph(*glyph))));
                            }
                        },
                        option { value: "default", selected: selected == "default", {language.text(Text::DefaultVaultIcon)} }
                        for (_, value, label) in CHOICES {
                            option { value, selected: selected == value, {language.text(label)} }
                        }
                        if custom_image {
                            option { value: "image", selected: true, {language.text(Text::CustomVaultImage)} }
                        }
                    }
                    button { id: "settings-upload-vault-image", r#type: "button", disabled,
                        onclick: move |_| choose_image(state, loading, language.text(Text::InvalidVaultImage)),
                        {language.text(Text::UploadVaultImage)}
                    }
                }
            }
        }
    }
}

fn active_vault(state: Session) -> Option<(PathBuf, String)> {
    let screen = state.peek();
    match &*screen {
        Screen::Unlocked(u) if screen.can_switch_profile() => {
            Some((u.vault.path().to_path_buf(), u.vault.id()))
        }
        _ => None,
    }
}

fn choose_image(state: Session, mut loading: Signal<bool>, invalid: &'static str) {
    let Some(target) = active_vault(state) else {
        return;
    };
    if loading() {
        return;
    }
    loading.set(true);
    spawn(async move {
        if let Some(file) = rfd::AsyncFileDialog::new()
            .add_filter("PNG, JPEG, WebP", &["png", "jpg", "jpeg", "webp"])
            .pick_file()
            .await
            && active_vault(state).as_ref() == Some(&target)
        {
            let path = file.path().to_path_buf();
            let icon = tokio::task::spawn_blocking(move || read_image(&path))
                .await
                .ok()
                .flatten();
            if active_vault(state).as_ref() == Some(&target) {
                apply_icon(state, icon.map(Some).ok_or_else(|| invalid.to_owned()));
            }
        }
        loading.set(false);
    });
}

fn apply_icon(mut state: Session, icon: Result<Option<SpaceIcon>, String>) {
    let mut screen = state.write();
    if screen.can_switch_profile()
        && let Screen::Unlocked(workspace) = &mut *screen
    {
        workspace.error = icon
            .and_then(|icon| {
                workspace
                    .vault
                    .set_icon(icon)
                    .map_err(|error| error.to_string())
            })
            .err();
        if workspace.error.is_none() {
            workspace.sync.saved();
            let mut settings = consume_context::<Signal<Settings>>();
            workspace.error = settings.write().cache_vault_icon(&workspace.vault).err();
        }
    }
}

fn read_image(path: &Path) -> Option<SpaceIcon> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_IMAGE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_IMAGE_FILE_BYTES {
        return None;
    }
    normalize_image(&bytes)
}

fn normalize_image(bytes: &[u8]) -> Option<SpaceIcon> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)
    ) {
        return None;
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().ok()?;
    limits.reserve(decoder.total_bytes()).ok()?;
    decoder.set_limits(limits).ok()?;
    let orientation = decoder.orientation().ok()?;
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    image.apply_orientation(orientation);
    let thumb = image
        .thumbnail(SPACE_ICON_SIZE, SPACE_ICON_SIZE)
        .into_rgba8();
    let mut pixels = RgbaImage::new(SPACE_ICON_SIZE, SPACE_ICON_SIZE);
    image::imageops::replace(
        &mut pixels,
        &thumb,
        i64::from((SPACE_ICON_SIZE - thumb.width()) / 2),
        i64::from((SPACE_ICON_SIZE - thumb.height()) / 2),
    );
    Some(SpaceIcon::Image(Blob(pixels.into_raw())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ExtendedColorType, ImageEncoder, Rgba, codecs::png::PngEncoder};

    #[test]
    fn image_upload_preserves_aspect_and_rejects_unsupported_or_oversized_input() {
        let png = |width, height| {
            let image = RgbaImage::from_pixel(width, height, Rgba([255, 8, 32, 255]));
            let mut bytes = Vec::new();
            PngEncoder::new(&mut bytes)
                .write_image(&image, width, height, ExtendedColorType::Rgba8)
                .unwrap();
            bytes
        };
        let icon = normalize_image(&png(SPACE_ICON_SIZE, SPACE_ICON_SIZE / 2)).unwrap();
        let SpaceIcon::Image(pixels) = &icon else {
            panic!("expected pixels")
        };
        assert!(icon.valid());
        assert_eq!(&pixels.0[..4], &[0, 0, 0, 0]);
        let middle = (SPACE_ICON_SIZE * SPACE_ICON_SIZE / 2 * 4) as usize;
        assert_eq!(&pixels.0[middle..middle + 4], &[255, 8, 32, 255]);
        assert!(normalize_image(&png(MAX_IMAGE_DIMENSION + 1, 1)).is_none());
        assert!(normalize_image(b"<svg></svg>").is_none());
    }
}
