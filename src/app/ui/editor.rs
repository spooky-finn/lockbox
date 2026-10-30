use super::{
    generator::GeneratorView,
    icons::{EntryIcon, IconPicker},
    totp::TotpEditor,
};
use crate::app::context::{Session, edit_draft, edit_unlocked, set_field};
use crate::{
    app::{editor::Field, session::Screen},
    icons,
};
use dioxus::prelude::*;

#[component]
pub(super) fn DraftView() -> Element {
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    let Some(draft) = &unlocked.draft else {
        return rsx! {};
    };
    let entry = &draft.entry;
    let icon = icons::catalog().for_entry(entry);
    rsx! {
        form { class: "draft stack", onsubmit: move |event| { event.prevent_default(); edit_unlocked(state, |u| u.error = u.save_draft().err()); },
            h1 { if draft.is_new { "Новая запись" } else { "Изменение записи" } }
            label { r#for: "entry-title", "Название" }
            input { id: "entry-title", required: true, value: "{entry.title}", oninput: move |e| set_field(state, Field::Title, e.value()) }
            label { r#for: "entry-username", "Логин" }
            input { id: "entry-username", autocomplete: "off", value: "{entry.username}", oninput: move |e| set_field(state, Field::Username, e.value()) }
            label { r#for: "entry-password", "Пароль" }
            div { class: "input-actions",
                input { id: "entry-password", r#type: if draft.show_password { "text" } else { "password" }, autocomplete: "off",
                    value: "{entry.password}", oninput: move |e| set_field(state, Field::Password, e.value()) }
                button { r#type: "button", onclick: move |_| edit_draft(state, |d| d.show_password = !d.show_password), if draft.show_password { "Скрыть" } else { "Показать" } }
                button { r#type: "button", onclick: move |_| edit_draft(state, |d| d.generator_open = !d.generator_open), "Сгенерировать…" }
            }
            if draft.generator_open { GeneratorView {} }
            label { r#for: "entry-url", "URL" }
            input { id: "entry-url", value: "{entry.url}", oninput: move |e| set_field(state, Field::Url, e.value()) }
            div { class: "input-actions icon-actions",
                EntryIcon { title: entry.title.clone(), slug: icon.map(|i| i.slug.clone()) }
                span { if let Some(icon) = icon { if entry.icon.is_none() { "авто: " } "{icon.title}" } else { "авто: буква" } }
                button { r#type: "button", onclick: move |_| edit_draft(state, |d| d.icon_picker_open = !d.icon_picker_open), "Выбрать иконку…" }
                if entry.icon.is_some() { button { r#type: "button", onclick: move |_| edit_draft(state, |d| d.entry.icon = None), "Авто" } }
            }
            if draft.icon_picker_open { IconPicker {} }
            label { r#for: "entry-notes", "Заметка" }
            textarea { id: "entry-notes", rows: 4, value: "{entry.notes}", oninput: move |e| set_field(state, Field::Notes, e.value()) }
            TotpEditor {}
            div { class: "actions form-actions",
                button { class: "primary", r#type: "submit", "Сохранить" }
                button { r#type: "button", onclick: move |_| edit_unlocked(state, |u| { u.draft = None; u.error = None; }), "Отмена" }
            }
        }
    }
}
