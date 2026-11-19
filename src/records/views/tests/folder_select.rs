use crate::{
    desktop::{
        context::{Session, edit_unlocked},
        session::Screen,
        view_test_support::mounted,
    },
    records::folder::Folder,
};
use dioxus::{
    dioxus_core::{AttributeValue, ElementId, Mutation},
    html::{
        EventData, SerializedFocusData, SerializedFormData, SerializedHtmlEventConverter,
        SerializedKeyboardData, SerializedMouseData,
    },
    prelude::*,
};
use kuchikiki::traits::TendrilSink;

fn element(edits: &[Mutation], target: &str) -> ElementId {
    edits
        .iter()
        .find_map(|edit| match edit {
            Mutation::SetAttribute {
                name: "id",
                value: AttributeValue::Text(value),
                id,
                ..
            } if value == target => Some(*id),
            _ => None,
        })
        .unwrap()
}

fn send(dom: &VirtualDom, target: ElementId, name: &str, data: EventData) -> bool {
    let event = Event::new(data.into_any(), true);
    dom.runtime().handle_event(name, event.clone(), target);
    event.default_action_enabled()
}

fn key(dom: &mut VirtualDom, input: ElementId, key: Key) {
    assert!(!send(
        dom,
        input,
        "keydown",
        EventData::Keyboard(SerializedKeyboardData::new(
            key,
            Code::Unidentified,
            Location::Standard,
            false,
            Modifiers::empty(),
            false,
        )),
    ));
    dom.render_immediate_to_vec();
}

fn search(dom: &mut VirtualDom, input: ElementId, value: &str) {
    send(
        dom,
        input,
        "input",
        EventData::Form(SerializedFormData::new(value.into(), vec![])),
    );
    dom.render_immediate_to_vec();
}

fn paths(dom: &VirtualDom) -> Vec<String> {
    kuchikiki::parse_html()
        .one(dioxus_ssr::render(dom))
        .document_node
        .select(".folder-autocomplete-results .folder-autocomplete-path")
        .unwrap()
        .map(|path| path.text_contents())
        .collect()
}

#[test]
fn record_folder_search_commits_only_on_selection() {
    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = mounted();
    let parent = Folder::new("PrimeGate".into(), None);
    let child = Folder::new("Dev".into(), Some(parent.id.clone()));
    let original = parent.id.clone();
    let destination = child.id.clone();
    dom.in_scope(ScopeId::APP, || {
        edit_unlocked(consume_context(), |u| {
            u.vault.test_folders_mut().extend([parent, child]);
            u.vault.test_entries_mut()[0].folder_id = Some(original.clone());
            u.start_edit();
        });
    });
    let input = element(&dom.render_immediate_to_vec().edits, "entry-folder");
    let selected = |dom: &VirtualDom| {
        dom.in_scope(ScopeId::APP, || {
            let state = consume_context::<Session>();
            let screen = state.peek();
            let Screen::Unlocked(u) = &*screen else {
                panic!("locked");
            };
            u.draft.as_ref().unwrap().entry.folder_id.clone()
        })
    };

    search(&mut dom, input, "missing");
    assert!(paths(&dom).is_empty());
    assert!(dioxus_ssr::render(&dom).contains("folder-autocomplete-empty"));
    key(&mut dom, input, Key::Enter);
    key(&mut dom, input, Key::Escape);
    assert_eq!(selected(&dom), Some(original.clone()));
    assert!(dioxus_ssr::render(&dom).contains("value=\"PrimeGate\""));

    search(&mut dom, input, "PRIMEGATE dev");
    assert_eq!(paths(&dom), ["PrimeGate / Dev"]);
    search(&mut dom, input, "primegate");
    assert_eq!(paths(&dom), ["PrimeGate", "PrimeGate / Dev"]);
    key(&mut dom, input, Key::ArrowDown);
    assert_eq!(selected(&dom), Some(original));
    key(&mut dom, input, Key::Enter);
    assert_eq!(selected(&dom), Some(destination));
    assert!(dioxus_ssr::render(&dom).contains("value=\"PrimeGate / Dev\""));

    key(&mut dom, input, Key::ArrowDown);
    key(&mut dom, input, Key::Enter);
    assert_eq!(selected(&dom), None);
}

#[test]
fn parent_folder_search_excludes_the_current_subtree_and_accepts_clicks() {
    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = mounted();
    let parent = Folder::new("Infra".into(), None);
    let child = Folder::new("Nested".into(), Some(parent.id.clone()));
    let target = Folder::new("Email".into(), None);
    let destination = target.id.clone();
    dom.in_scope(ScopeId::APP, || {
        edit_unlocked(consume_context(), |u| {
            let id = parent.id.clone();
            u.vault.test_folders_mut().extend([parent, child, target]);
            u.select_directory(Some(id));
            u.edit_directory();
        });
    });
    let input = element(&dom.render_immediate_to_vec().edits, "directory-parent");
    send(
        &dom,
        input,
        "focus",
        EventData::Focus(SerializedFocusData {}),
    );
    let edits = dom.render_immediate_to_vec().edits;
    assert_eq!(paths(&dom), ["Корень хранилища", "Email"]);
    let email = element(&edits, "directory-parent-option-1");
    let mouse = || EventData::Mouse(SerializedMouseData::default());
    assert!(!send(&dom, email, "mousedown", mouse()));
    send(&dom, email, "click", mouse());
    dom.render_immediate_to_vec();
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        let screen = state.peek();
        let Screen::Unlocked(u) = &*screen else {
            panic!("locked");
        };
        assert_eq!(
            u.directories.draft.as_ref().unwrap().parent_id.as_deref(),
            Some(destination.as_str())
        );
    });
    assert!(paths(&dom).is_empty());
}
