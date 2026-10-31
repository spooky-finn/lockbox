use super::button::IconButton;
use super::symbol::{Icon, Symbol};
use super::{
    folder_select::FolderSelect,
    generator::GeneratorView,
    icons::{EntryIcon, IconPicker},
    totp::TotpEditor,
};
use crate::ui::context::{Session, edit_draft, edit_unlocked, set_field};
use crate::{
    app::{editor::Field, session::Screen},
    ui::icons,
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
            div { class: "form-heading",
                div { class: "form-icon", Symbol { icon: Icon::Key } }
                div { p { class: "detail-kind", "Хранилище паролей" } h1 { if draft.is_new { "Новая запись" } else { "Изменение записи" } } }
            }
            label { r#for: "entry-title", "Название" }
            input { id: "entry-title", required: true, value: "{entry.title}", oninput: move |e| set_field(state, Field::Title, e.value()) }
            label { r#for: "entry-folder", "Папка" }
            FolderSelect { id: "entry-folder", value: entry.folder_id.clone(),
                onchange: move |folder| edit_draft(state, |d| d.entry.folder_id = folder),
            }
            label { r#for: "entry-username", "Логин" }
            input { id: "entry-username", autocomplete: "off", value: "{entry.username}", oninput: move |e| set_field(state, Field::Username, e.value()) }
            label { r#for: "entry-password", "Пароль" }
            div { class: "input-actions",
                input { id: "entry-password", r#type: if draft.show_password { "text" } else { "password" }, autocomplete: "off",
                    value: "{entry.password}", oninput: move |e| set_field(state, Field::Password, e.value()) }
                IconButton { icon: if draft.show_password { Icon::EyeOff } else { Icon::Eye },
                    label: if draft.show_password { "Скрыть пароль" } else { "Показать пароль" }, pressed: Some(draft.show_password),
                    onclick: move |_| edit_draft(state, |d| d.show_password = !d.show_password),
                }
                IconButton { icon: Icon::Sparkles, label: "Сгенерировать…", expanded: Some(draft.generator_open),
                    onclick: move |_| edit_draft(state, |d| d.generator_open = !d.generator_open),
                }
            }
            if draft.generator_open { GeneratorView {} }
            label { r#for: "entry-url", "URL" }
            input { id: "entry-url", value: "{entry.url}", oninput: move |e| set_field(state, Field::Url, e.value()) }
            div { class: "input-actions icon-actions",
                EntryIcon { title: entry.title.clone(), slug: icon.map(|i| i.slug.clone()) }
                span { if let Some(icon) = icon { if entry.icon.is_none() { "авто: " } "{icon.title}" } else { "авто: буква" } }
                IconButton { icon: Icon::Image, label: "Выбрать иконку…", expanded: Some(draft.icon_picker_open),
                    onclick: move |_| edit_draft(state, |d| d.icon_picker_open = !d.icon_picker_open),
                }
                if entry.icon.is_some() { IconButton { icon: Icon::Reset, label: "Подобрать иконку автоматически", onclick: move |_| edit_draft(state, |d| d.entry.icon = None) } }
            }
            if draft.icon_picker_open { IconPicker {} }
            label { r#for: "entry-notes", "Заметка" }
            textarea { id: "entry-notes", rows: 3, value: "{entry.notes}", oninput: move |e| set_field(state, Field::Notes, e.value()) }
            TotpEditor {}
            div { class: "actions form-actions",
                IconButton { class: "primary", kind: "submit", icon: Icon::Save, label: "Сохранить" }
                IconButton { icon: Icon::Close, label: "Отмена", onclick: move |_| edit_unlocked(state, |u| { u.draft = None; u.error = None; }) }
            }
        }
    }
}
