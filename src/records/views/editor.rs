use crate::desktop::i18n::Text;
use crate::records::dialogs;

use crate::{
    desktop::{
        context::{Session, draft_control, edit_draft, edit_unlocked, set_field},
        icons,
        session::Screen,
        widgets::{
            button::IconButton,
            icons::{EntryIcon, IconPicker},
            symbol::{Icon, Symbol},
        },
    },
    records::{
        editor::{Control, Field},
        views::{folder_select::FolderSelect, generator::GeneratorView, totp::TotpEditor},
    },
};
use dioxus::prelude::*;

pub(crate) fn finish_on_enter(state: Session, event: KeyboardEvent) {
    if !event.is_composing() && !event.is_auto_repeating() && event.key() == Key::Enter {
        event.prevent_default();
        event.stop_propagation();
        edit_unlocked(state, |u| u.error = u.close_draft().err());
    }
}

#[component]
pub(crate) fn DraftView() -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };

    let Some(draft) = &unlocked.draft else {
        return rsx! {};
    };

    let focus_field = draft.focus_field.or(draft.is_new.then_some(Field::Title));
    let entry = &draft.entry;
    let icon = icons::catalog().for_entry(entry);
    let automatic_favicon = entry.icon.is_none() && icons::favicon_url(&entry.url).is_some();
    rsx! {
        form { class: "draft entry-detail", aria_label: if draft.is_new { language.text(Text::NewEntry) } else { language.text(Text::EditEntry) },
            onkeydown: move |event: KeyboardEvent| {
                if !event.is_composing() && !event.is_auto_repeating() && event.key() == Key::Enter && (event.modifiers().meta() || event.modifiers().ctrl()) {
                    finish_on_enter(state, event);
                }
            },
            onsubmit: move |event| { event.prevent_default(); edit_unlocked(state, |u| u.error = u.close_draft().err()); },
            div { class: "entry-close",
                IconButton { class: "quiet", icon: Icon::Close, label: language.text(Text::CloseEditor), onclick: move |_| edit_unlocked(state, |u| u.navigate(crate::desktop::workspace::Destination::Close)) }
                span { class: "record-hint", {language.text(Text::RecordEditHint)} }
            }
            div { class: "detail-heading",
                EntryIcon { title: entry.title.clone(), slug: entry.icon.clone(), url: entry.url.clone(), large: true }
                div { class: "detail-title",
                    p { class: "detail-kind", {language.text(Text::Password)} }
                    input { onkeydown: move |event| finish_on_enter(state, event), id: "entry-title", class: "entry-title-input", aria_label: language.text(Text::Name), required: true, onmounted: move |event| async move { if focus_field == Some(Field::Title) { let _ = event.data.set_focus(true).await; } },
                        value: "{entry.title}", oninput: move |e| set_field(state, Field::Title, e.value()) }
                }
                button { class: "primary record-save", r#type: "button", title: if cfg!(target_os = "macos") { "⌘S" } else { "Ctrl+S" },
                    onclick: move |_| edit_unlocked(state, |u| u.error = u.save_draft().err()),
                    Symbol { icon: Icon::Save } {language.text(Text::Save)}
                }
            }
            dl { class: "fields",
                div { class: "field-row", dt { label { r#for: "entry-username", {language.text(Text::Username2)} } }
                    dd { input { onkeydown: move |event| finish_on_enter(state, event), id: "entry-username", class: "field-value", onmounted: move |event| async move { if focus_field == Some(Field::Username) { let _ = event.data.set_focus(true).await; } }, autocomplete: "off", value: "{entry.username}", oninput: move |e| set_field(state, Field::Username, e.value()) } }
                }
                div { class: "field-row", dt { label { r#for: "entry-password", {language.text(Text::Password)} } }
                    dd {
                        input { onkeydown: move |event| finish_on_enter(state, event), id: "entry-password", class: "field-value password-value", onmounted: move |event| async move { if focus_field == Some(Field::Password) { let _ = event.data.set_focus(true).await; } }, r#type: if draft.show_password { "text" } else { "password" }, autocomplete: "off",
                            value: "{entry.password}", oninput: move |e| set_field(state, Field::Password, e.value()) }
                        IconButton { icon: if draft.show_password { Icon::EyeOff } else { Icon::Eye },
                            label: if draft.show_password { language.text(Text::HidePassword) } else { language.text(Text::ShowPassword) }, pressed: Some(draft.show_password),
                            onclick: move |_| draft_control(state, Control::TogglePassword),
                        }
                        IconButton { icon: Icon::Sparkles, label: language.text(Text::Generate), expanded: Some(draft.generator_open),
                            onclick: move |_| draft_control(state, Control::ToggleGenerator),
                        }
                    }
                }
                div { class: "field-row", dt { label { r#for: "entry-url", {language.text(Text::Url)} } }
                    dd { input { onkeydown: move |event| finish_on_enter(state, event), id: "entry-url", class: "field-value", onmounted: move |event| async move { if focus_field == Some(Field::Url) { let _ = event.data.set_focus(true).await; } }, value: "{entry.url}", oninput: move |e| set_field(state, Field::Url, e.value()) } }
                }
                div { class: "field-row totp-field", dt { "TOTP" } dd { TotpEditor {} } }
                div { class: "field-row notes-row", dt { label { r#for: "entry-notes", {language.text(Text::Note)} } }
                    dd { textarea { id: "entry-notes", class: "field-value", onmounted: move |event| async move { if focus_field == Some(Field::Notes) { let _ = event.data.set_focus(true).await; } }, rows: entry.notes.lines().count().clamp(1, 8), value: "{entry.notes}", oninput: move |e| set_field(state, Field::Notes, e.value()) } }
                }
            }
            if draft.generator_open { GeneratorView {} }
            div { class: "entry-options stack",
            label { r#for: "entry-folder", {language.text(Text::Folder)} }
            FolderSelect { id: "entry-folder", value: entry.folder_id.clone(),
                onchange: move |folder| edit_draft(state, |d| d.entry.folder_id = folder),
            }
            div { class: "input-actions icon-actions",
                EntryIcon { title: entry.title.clone(), slug: entry.icon.clone(), url: entry.url.clone() }
                span { if automatic_favicon { {language.text(Text::Auto)} } else if let Some(icon) = icon { if entry.icon.is_none() { {language.text(Text::Auto)} } "{icon.title}" } else { {language.text(Text::AutoInitial)} } }
                IconButton { icon: Icon::Image, label: language.text(Text::ChooseIcon), expanded: Some(draft.icon_picker_open),
                    onclick: move |_| draft_control(state, Control::ToggleIconPicker),
                }

                if entry.icon.is_some() { IconButton { icon: Icon::Reset, label: language.text(Text::ChooseIconAutomatically), onclick: move |_| edit_draft(state, |d| d.entry.icon = None) } }
            }

            if draft.icon_picker_open { IconPicker {} }
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
                    onclick: move |_| dialogs::add_attachment(),
                }
            }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/editor.rs"]
mod tests;
