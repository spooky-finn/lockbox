use crate::desktop::i18n::Text;

use crate::{
    desktop::{
        context::{Session, use_records_version},
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
    let version = use_records_version();
    let options = use_memo(use_reactive((&exclude,), move |(exclude,)| {
        let _version = version.read();
        let screen = state.peek();
        let Screen::Unlocked(workspace) = &*screen else {
            return Vec::new();
        };
        #[cfg(test)]
        OPTION_BUILDS.set(OPTION_BUILDS.get() + 1);
        let hierarchy = folder::Hierarchy::new(workspace.vault.folders());
        hierarchy
            .options()
            .into_iter()
            .filter(|(folder, _)| {
                !exclude
                    .as_deref()
                    .is_some_and(|id| hierarchy.contains(id, &folder.id))
            })
            .map(|(folder, path)| (folder.id.clone(), path))
            .collect::<Vec<_>>()
    }));
    rsx! {
        div { class: "select-field",
            select { id, value: value.as_deref().unwrap_or(""),
                onchange: move |event| {
                    let value = event.value();
                    onchange.call((!value.is_empty()).then_some(value));
                },
                option { value: "", selected: value.is_none(), {language.text(Text::VaultRoot)} }
                for (folder_id, path) in options.read().iter() {
                    option { key: "{folder_id}", value: "{folder_id}", selected: value.as_deref() == Some(folder_id.as_str()), "{path}" }
                }
            }
            Symbol { icon: Icon::ChevronRight }
        }
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static OPTION_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
