use super::symbol::{Icon, Symbol};
use crate::ui::context::Session;
use crate::{app::session::Screen, core::folder};
use dioxus::prelude::*;

#[component]
pub(super) fn FolderSelect(
    id: &'static str,
    value: Option<String>,
    exclude: Option<String>,
    onchange: EventHandler<Option<String>>,
) -> Element {
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(workspace) = &*screen else {
        return rsx! {};
    };
    let options = folder::options(&workspace.vault.folders);
    rsx! {
        div { class: "select-field",
            select { id, value: value.as_deref().unwrap_or(""),
                onchange: move |event| {
                    let value = event.value();
                    onchange.call((!value.is_empty()).then_some(value));
                },
                option { value: "", selected: value.is_none(), "Корень хранилища" }
                for (folder, path) in options {
                    if !exclude.as_deref().is_some_and(|id| folder::contains(&workspace.vault.folders, id, &folder.id)) {
                        option { value: "{folder.id}", selected: value.as_deref() == Some(folder.id.as_str()), "{path}" }
                    }
                }
            }
            Symbol { icon: Icon::ChevronRight }
        }
    }
}
