use super::{button::IconButton, symbol::Icon};
use crate::app::context::{Session, edit_draft};
use crate::{app::session::Screen, icons};
use dioxus::prelude::*;

#[component]
pub(super) fn EntryIcon(
    title: String,
    slug: Option<String>,
    #[props(default = false)] large: bool,
) -> Element {
    let icon = slug.as_deref().and_then(|slug| icons::catalog().get(slug));
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
    rsx! {
        span { class: if large { "entry-icon large" } else { "entry-icon" }, style: "background:{background};color:{foreground}", aria_hidden: "true",
            if let Some(icon) = icon {
                svg { view_box: "0 0 24 24", path { fill: "currentColor", d: "{icon.path}" } }
            } else { "{icons::letter(&title)}" }
        }
    }
}
#[component]
pub(super) fn IconPicker() -> Element {
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
        section { class: "subpanel stack", aria_label: "Выбор иконки",
            div { class: "input-actions",
                input { aria_label: "Название сервиса", placeholder: "Название сервиса", value: "{draft.icon_query}", oninput: move |e| edit_draft(state, |d| d.icon_query = e.value()) }
                IconButton { icon: Icon::Close, label: "Закрыть выбор иконки", onclick: move |_| edit_draft(state, |d| d.icon_picker_open = false) }
            }
            if draft.icon_query.trim().is_empty() { p { class: "muted", "Начните вводить название — в наборе {icons::catalog().len()} иконок." } }
            else if found.is_empty() { p { class: "muted", "Ничего не найдено" } }
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
