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

#[derive(PartialEq)]
struct FolderOption {
    id: Option<String>,
    path: String,
    search_key: String,
}

impl FolderOption {
    fn new(id: Option<String>, path: String) -> Self {
        let search_key = path.to_lowercase();
        Self {
            id,
            path,
            search_key,
        }
    }
}

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
    let mut query = use_signal(|| None::<String>);
    let mut highlighted = use_signal(|| None::<usize>);
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
            .map(|(folder, path)| FolderOption::new(Some(folder.id.clone()), path))
            .collect::<Vec<_>>()
    }));

    use_effect(move || {
        let _ = highlighted();
        if query.read().is_some() {
            // The list lives in the scrolling editor. Keep results within reach even
            // when the combobox opens near the bottom of the visible area.
            let _ = document::eval(
                "requestAnimationFrame(() => {
                    const input = document.activeElement;
                    if (!input?.matches('.folder-autocomplete-input')) return;
                    const option = document.getElementById(input.getAttribute('aria-activedescendant'));
                    const popup = input.parentElement.nextElementSibling;
                    (option ?? popup)?.scrollIntoView({block: 'nearest', inline: 'nearest'});
                })",
            );
        }
    });

    let root = FolderOption::new(None, language.text(Text::VaultRoot).into());
    let options = options.read();
    let all = || std::iter::once(&root).chain(options.iter());
    let current = all().find(|option| option.id == value);
    let search = query.read();
    let expanded = search.is_some();
    let matches: Vec<_> = search.as_ref().map_or_else(Vec::new, |search| {
        let search = search.to_lowercase();
        all()
            .filter(|option| {
                search
                    .split_whitespace()
                    .all(|word| option.search_key.contains(word))
            })
            .collect()
    });
    let count = matches.len();
    let active = highlighted().and_then(|index| count.checked_sub(1).map(|last| index.min(last)));
    let choice = active.map(|index| matches[index].id.clone());
    let input = search
        .as_deref()
        .unwrap_or_else(|| current.map_or("", |option| &option.path));
    let all_count = options.len() + 1;
    let list_id = format!("{id}-options");
    let has_results = count > 0;
    let mut close = move || {
        query.set(None);
        highlighted.set(None);
    };

    rsx! {
        div { class: "folder-autocomplete",
            div { class: "folder-autocomplete-field",
                input { id, class: "folder-autocomplete-input", role: "combobox", value: input,
                    placeholder: language.text(Text::SearchFolders), autocomplete: "off",
                    spellcheck: "false", "autocorrect": "off",
                    aria_autocomplete: "list", aria_expanded: has_results,
                    aria_controls: has_results.then_some(list_id.clone()),
                    aria_activedescendant: active.map(|index| format!("{id}-option-{index}")),
                    onfocus: move |_| { query.set(Some(String::new())); },
                    onclick: move |_| {
                        if query.peek().is_none() { query.set(Some(String::new())); }
                    },
                    onblur: move |_| close(),
                    oninput: move |event| {
                        query.set(Some(event.value()));
                        highlighted.set(Some(0));
                    },
                    onkeydown: move |event| {
                        if event.is_composing() || !event.modifiers().is_empty() { return; }
                        match event.key() {
                            Key::ArrowDown | Key::ArrowUp => {
                                event.prevent_default();
                                let forward = event.key() == Key::ArrowDown;
                                if !expanded {
                                    query.set(Some(String::new()));
                                    highlighted.set(Some(if forward { 0 } else { all_count - 1 }));
                                } else if count > 0 {
                                    let next = match active {
                                        Some(index) if forward => (index + 1) % count,
                                        Some(index) => (index + count - 1) % count,
                                        None if forward => 0,
                                        None => count - 1,
                                    };
                                    highlighted.set(Some(next));
                                }
                            }
                            Key::Enter => {
                                // Selecting a folder must not submit the enclosing editor.
                                event.prevent_default();
                                event.stop_propagation();
                                if !event.is_auto_repeating() {
                                    if let Some(folder) = choice.clone() {
                                        close();
                                        onchange.call(folder);
                                    } else if !expanded {
                                        query.set(Some(String::new()));
                                    }
                                }
                            }
                            Key::Escape if expanded => {
                                event.prevent_default();
                                event.stop_propagation();
                                close();
                            }
                            _ => {}
                        }
                    },
                },
                Symbol { icon: Icon::Search }
            }
            if has_results {
                ul { id: "{list_id}", class: "folder-autocomplete-results", role: "listbox", aria_label: language.text(Text::Folder),
                    for (index, option) in matches.into_iter().enumerate() {
                        {
                            let target = option.id.clone();
                            let selected = target == value;
                            let icon = if target.is_some() { Icon::Folder } else { Icon::FolderOpen };
                            rsx! {
                                li { key: "{option.id.as_deref().unwrap_or_default()}", id: "{id}-option-{index}", role: "option",
                                    aria_selected: selected, "data-active": (Some(index) == active).to_string(),
                                    // Keep the input focused until the option's click is handled.
                                    onmousedown: move |event| event.prevent_default(),
                                    onclick: move |_| { close(); onchange.call(target.clone()); },
                                    Symbol { icon }
                                    span { class: "folder-autocomplete-path", "{option.path}" }
                                    if selected { Symbol { icon: Icon::Check } }
                                }
                            }
                        }
                    }
                }
            } else if expanded {
                p { class: "folder-autocomplete-empty", role: "status", {language.text(Text::NoResults)} }
            }
        }
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static OPTION_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
#[path = "tests/folder_select.rs"]
mod tests;
