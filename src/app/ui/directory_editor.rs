use super::{
    button::IconButton,
    folder_select::FolderSelect,
    symbol::{Icon, Symbol},
};
use crate::app::{
    context::{Session, edit_unlocked},
    editor::replace_secret,
    session::Screen,
};
use dioxus::prelude::*;
use zeroize::Zeroizing;

#[component]
pub(super) fn DirectoryEditor() -> Element {
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(workspace) = &*screen else {
        return rsx! {};
    };
    let Some(draft) = &workspace.directories.draft else {
        return rsx! {};
    };
    rsx! {
        form { class: "draft directory-editor stack",
            onsubmit: move |event| {
                event.prevent_default();
                edit_unlocked(state, |u| u.error = u.save_directory().err());
            },
            div { class: "form-heading",
                div { class: "form-icon", Symbol { icon: Icon::Folder } }
                div { p { class: "detail-kind", "Организация хранилища" }
                    h1 { if draft.id.is_none() { "Новая папка" } else { "Изменение папки" } }
                }
            }
            label { r#for: "directory-name", "Название" }
            input { id: "directory-name", required: true, autofocus: true, maxlength: "128", autocomplete: "off",
                placeholder: "Например, Работа", value: "{draft.name}",
                oninput: move |event| {
                    let name = Zeroizing::new(event.value());
                    edit_unlocked(state, |u| { if let Some(draft) = &mut u.directories.draft { replace_secret(&mut draft.name, &name); } });
                },
            }
            label { r#for: "directory-parent", "Расположение" }
            FolderSelect { id: "directory-parent", value: draft.parent_id.clone(), exclude: draft.id.clone(),
                onchange: move |parent| edit_unlocked(state, |u| { if let Some(draft) = &mut u.directories.draft { draft.parent_id = parent; } }),
            }
            div { class: "actions form-actions",
                IconButton { class: "primary", kind: "submit", icon: Icon::Save, label: "Сохранить папку" }
                IconButton { icon: Icon::Close, label: "Отмена", onclick: move |_| edit_unlocked(state, |u| { u.directories.draft = None; u.error = None; }) }
            }
        }
    }
}
