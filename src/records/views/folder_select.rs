use crate::desktop::i18n::Text;

use crate::{
    desktop::{
        context::Session,
        session::Screen,
        widgets::symbol::{Icon, Symbol},
    },
    records::folder,
};
use dioxus::prelude::*;

#[component]
pub(crate) fn FolderSelect(
    id: &'static str,
    value: Option<String>,
    exclude: Option<String>,
    onchange: EventHandler<Option<String>>,
) -> Element {
    let language = crate::desktop::i18n::use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(workspace) = &*screen else {
        return rsx! {};
    };

    let options = folder::options(workspace.vault.folders());
    rsx! {
        div { class: "select-field",
            select { id, value: value.as_deref().unwrap_or(""),
                onchange: move |event| {
                    let value = event.value();
                    onchange.call((!value.is_empty()).then_some(value));
                },
                option { value: "", selected: value.is_none(), {language.text(Text::VaultRoot)} }
                for (folder, path) in options {
                    if !exclude.as_deref().is_some_and(|id| folder::contains(workspace.vault.folders(), id, &folder.id)) {
                        option { value: "{folder.id}", selected: value.as_deref() == Some(folder.id.as_str()), "{path}" }
                    }
                }
            }
            Symbol { icon: Icon::ChevronRight }
        }
    }
}
