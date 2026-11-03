use crate::desktop::i18n::Text;

use crate::{
    desktop::{
        context::{Session, edit_draft, edit_unlocked, set_field},
        icons,
        session::Screen,
        widgets::{
            button::IconButton,
            icons::{EntryIcon, IconPicker},
            symbol::{Icon, Symbol},
        },
    },
    records::{
        editor::Field,
        views::{folder_select::FolderSelect, generator::GeneratorView, totp::TotpEditor},
    },
};
use dioxus::prelude::*;

#[component]
pub(crate) fn DraftView() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let mut confirm_delete = use_signal(|| false);
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };

    let Some(draft) = &unlocked.draft else {
        return rsx! {};
    };

    let entry = &draft.entry;
    let icon = icons::catalog().for_entry(entry);
    let automatic_favicon = entry.icon.is_none() && icons::favicon_url(&entry.url).is_some();
    rsx! {
        form { class: "draft stack", onsubmit: move |event| { event.prevent_default(); edit_unlocked(state, |u| u.error = u.save_draft().err()); },
            div { class: "form-heading",
                div { class: "form-icon", Symbol { icon: Icon::Key } }
                div { p { class: "detail-kind", {language.text(Text::PasswordVault)} } h1 { if draft.is_new { {language.text(Text::NewEntry)} } else { {language.text(Text::EditEntry)} } } }
            }
            label { r#for: "entry-title", {language.text(Text::Name)} }
            input { id: "entry-title", required: true, value: "{entry.title}", oninput: move |e| set_field(state, Field::Title, e.value()) }
            label { r#for: "entry-folder", {language.text(Text::Folder)} }
            FolderSelect { id: "entry-folder", value: entry.folder_id.clone(),
                onchange: move |folder| edit_draft(state, |d| d.entry.folder_id = folder),
            }
            label { r#for: "entry-username", {language.text(Text::Username2)} }
            input { id: "entry-username", autocomplete: "off", value: "{entry.username}", oninput: move |e| set_field(state, Field::Username, e.value()) }
            label { r#for: "entry-password", {language.text(Text::Password)} }
            div { class: "input-actions",
                input { id: "entry-password", r#type: if draft.show_password { "text" } else { "password" }, autocomplete: "off",
                    value: "{entry.password}", oninput: move |e| set_field(state, Field::Password, e.value()) }
                IconButton { icon: if draft.show_password { Icon::EyeOff } else { Icon::Eye },
                    label: if draft.show_password { language.text(Text::HidePassword) } else { language.text(Text::ShowPassword) }, pressed: Some(draft.show_password),
                    onclick: move |_| edit_draft(state, |d| d.show_password = !d.show_password),
                }
                IconButton { icon: Icon::Sparkles, label: language.text(Text::Generate), expanded: Some(draft.generator_open),
                    onclick: move |_| edit_draft(state, |d| d.generator_open = !d.generator_open),
                }
            }

            if draft.generator_open { GeneratorView {} }
            label { r#for: "entry-url", "URL" }
            input { id: "entry-url", value: "{entry.url}", oninput: move |e| set_field(state, Field::Url, e.value()) }
            div { class: "input-actions icon-actions",
                EntryIcon { title: entry.title.clone(), slug: entry.icon.clone(), url: entry.url.clone() }
                span { if automatic_favicon { {language.text(Text::Auto)} } else if let Some(icon) = icon { if entry.icon.is_none() { {language.text(Text::Auto)} } "{icon.title}" } else { {language.text(Text::AutoInitial)} } }
                IconButton { icon: Icon::Image, label: language.text(Text::ChooseIcon), expanded: Some(draft.icon_picker_open),
                    onclick: move |_| edit_draft(state, |d| d.icon_picker_open = !d.icon_picker_open),
                }

                if entry.icon.is_some() { IconButton { icon: Icon::Reset, label: language.text(Text::ChooseIconAutomatically), onclick: move |_| edit_draft(state, |d| d.entry.icon = None) } }
            }

            if draft.icon_picker_open { IconPicker {} }
            label { r#for: "entry-notes", {language.text(Text::Note)} }
            textarea { id: "entry-notes", rows: 3, value: "{entry.notes}", oninput: move |e| set_field(state, Field::Notes, e.value()) }
            fieldset { class: "stack",
                legend { {language.text(Text::Attachments)} }
                p { class: "muted", {language.text(Text::AttachmentLimit)} }
                for file in &entry.attachments {
                    div { class: "input-actions", key: "{file.id}",
                        span { "{file.name} ({file.data.len()} B)" }
                        IconButton { icon: Icon::Trash, label: language.text(Text::RemoveAttachment),
                            onclick: { let id = file.id.clone(); move |_| edit_draft(state, |d| d.entry.attachments.retain(|f| f.id != id)) },
                        }
                    }
                }
                IconButton { icon: Icon::Plus, label: language.text(Text::AddAttachment),
                    onclick: move |_| crate::desktop::runtime::add_attachment(),
                }
            }
            TotpEditor {}
            footer { class: "actions form-actions draft-footer",
                if *confirm_delete.read() {
                    div { class: "draft-delete-confirm",
                        p { {language.format(Text::MoveEntryToTrash, &[entry.title.to_string()])} }
                        div { class: "actions",
                            IconButton { class: "danger", icon: Icon::Trash, label: language.text(Text::DeleteEntry), disabled: unlocked.vault.signing_error().is_some(),
                                onclick: move |_| edit_unlocked(state, |u| {
                                    u.error = u.delete_selected().err();
                                    if u.error.is_none() { u.draft = None; }
                                }),
                            }
                            IconButton { icon: Icon::Close, label: language.text(Text::Cancel), onclick: move |_| confirm_delete.set(false) }
                        }
                    }
                }
                IconButton { class: "primary", kind: "submit", icon: Icon::Save, label: language.text(Text::Save) }
                IconButton { icon: Icon::Close, label: language.text(Text::Cancel), onclick: move |_| edit_unlocked(state, |u| { u.draft = None; u.error = None; }) }
                if !draft.is_new {
                    IconButton { class: "danger draft-delete", icon: Icon::Trash, label: language.text(Text::Delete), disabled: unlocked.vault.signing_error().is_some(),
                        onclick: move |_| confirm_delete.set(true),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/editor.rs"]
mod tests;
