use crate::desktop::i18n::Text;

use crate::desktop::{
    context::{Session, edit_draft},
    icons,
    session::Screen,
    widgets::{button::IconButton, symbol::Icon},
};
use dioxus::prelude::*;

#[component]
pub(crate) fn EntryIcon(
    title: String,
    slug: Option<String>,
    #[props(default)] url: String,
    #[props(default = false)] large: bool,
) -> Element {
    let mut failed_favicon = use_signal(|| None::<String>);
    let website = slug.is_none().then(|| icons::favicon_url(&url)).flatten();
    let fetched = use_resource(use_reactive((&website,), |(website,)| async move {
        let website = website?;
        let origin = website.clone();
        let image = tokio::runtime::Handle::try_current()
            .ok()?
            .spawn_blocking(move || crate::records::favicon::fetch(&website))
            .await
            .ok()
            .flatten()?;
        Some((origin, image))
    }));
    let favicon = fetched.read().as_ref().and_then(|result| {
        result
            .as_ref()
            .filter(|(origin, _)| website.as_ref() == Some(origin))
            .map(|(_, image)| image.clone())
    });
    let favicon = favicon.filter(|url| failed_favicon.read().as_ref() != Some(url));
    let icon = slug
        .as_deref()
        .and_then(|slug| icons::catalog().get(slug))
        .or_else(|| icons::catalog().for_url(&url))
        .or_else(|| icons::catalog().get(&icons::normalized(&title)));
    let (background, foreground) = match icon {
        Some(icon) => (
            format!("#{}", icon.hex),
            if icons::glyph_is_dark(icons::rgb(&icon.hex)) {
                "#000"
            } else {
                "#fff"
            },
        ),
        None => {
            let [r, g, b] = icons::letter_colour(&title);
            (format!("#{r:02x}{g:02x}{b:02x}"), "#fff")
        }
    };
    if let Some(favicon) = favicon {
        return rsx! {
            span { class: if large { "entry-icon favicon large" } else { "entry-icon favicon" }, aria_hidden: "true",
                img { src: "{favicon}", alt: "", referrerpolicy: "no-referrer",
                    onerror: move |_| failed_favicon.set(Some(favicon.clone())) }
            }
        };
    }
    rsx! {
        span { class: if large { "entry-icon large" } else { "entry-icon" }, style: "background:{background};color:{foreground}", aria_hidden: "true",
            if let Some(icon) = icon {
                svg { view_box: "0 0 24 24", path { fill: "currentColor", d: "{icon.path}" } }
            } else { "{icons::letter(&title)}" }
        }
    }
}

#[component]
pub(crate) fn IconPicker() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };

    let Some(draft) = &unlocked.draft else {
        return rsx! {};
    };

    let found = icons::catalog().search(&draft.icon_query, 60);
    rsx! {
        section { class: "subpanel stack", aria_label: language.text(Text::ChooseIcon2),
            div { class: "input-actions",
                input { aria_label: language.text(Text::ServiceName), placeholder: language.text(Text::ServiceName), value: "{draft.icon_query}", oninput: move |e| edit_draft(state, |d| d.icon_query = e.value()) }
                IconButton { icon: Icon::Close, label: language.text(Text::CloseIconPicker), onclick: move |_| edit_draft(state, |d| d.icon_picker_open = false) }
            }

            if draft.icon_query.trim().is_empty() { p { class: "muted", {language.format(Text::StartTypingANameIconsAvailable, &[icons::catalog().len().to_string()])} } }
            else if found.is_empty() { p { class: "muted", {language.text(Text::NoResults)} } }
            div { class: "icon-grid",
                for icon in found {
                    {
                        let slug = icon.slug.clone();
                        rsx! { button { key: "{slug}", r#type: "button", title: "{icon.title}", aria_label: "{icon.title}",
                            onclick: move |_| edit_draft(state, |d| { d.entry.icon = Some(slug.clone()); d.icon_picker_open = false; }),
                            EntryIcon { title: icon.title.clone(), slug: Some(icon.slug.clone()) }
                        } }
                    }
                }
            }
        }
    }
}
