use super::*;
use crate::{
    desktop::test_support::{add, reopen, unlocked},
    records::{folder::Folder, views::sidebar::Sidebar},
    settings::{language::Language, state::Settings},
};
use dioxus::{
    dioxus_core::{AttributeValue, ElementId, Mutation},
    html::{EventData, SerializedFormData, SerializedHtmlEventConverter, SerializedMouseData},
};

fn fixture() -> Element {
    let dirs =
        use_hook(|| std::rc::Rc::new([tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()]));
    let source = use_signal(|| {
        let mut workspace = unlocked(&dirs[0]);
        add(&mut workspace, "Selected login", "fictional user", "");
        let folder = Folder::new("Accounts".into(), None);
        workspace.vault.save_folder(folder.clone()).unwrap();
        let child_id = add(&mut workspace, "Inside folder", "fictional user", "");
        let mut child = workspace
            .vault
            .entries()
            .iter()
            .find(|entry| entry.id == child_id)
            .unwrap()
            .clone();
        child.folder_id = Some(folder.id.clone());
        workspace.vault.upsert_entry(child).unwrap();
        let id = add(&mut workspace, "Leave this login", "fictional user", "");
        workspace.select(id);
        Screen::Unlocked(Box::new(workspace))
    });
    let destination = use_signal(|| Screen::Unlocked(Box::new(unlocked(&dirs[1]))));
    let tabs = use_signal(|| Tabs {
        sessions: vec![source, destination],
        active: 0,
    });
    let settings = use_signal(|| {
        let mut settings = Settings {
            config: Default::default(),
            path: None,
        };
        settings.config.language = Language::English;
        settings
    });
    use_context_provider(|| source);
    use_context_provider(|| tabs);
    use_context_provider(|| settings);
    rsx! { Sidebar {} }
}

fn element(edits: &[Mutation], attribute: &str, name: &str) -> ElementId {
    edits
        .iter()
        .find_map(|edit| match edit {
            Mutation::SetAttribute {
                name: key,
                value: AttributeValue::Text(value),
                id,
                ..
            } if *key == attribute && value == name => Some(*id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing element {name}"))
}

#[test]
fn context_menu_starts_sidebar_selection_and_moves_only_checked_items() {
    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = VirtualDom::new(fixture);
    let initial = dom.rebuild_to_vec().edits;
    let html = dioxus_ssr::render(&dom);
    assert!(!html.contains("Move to another vault"));
    assert!(!html.contains("move-tree-checkbox"));

    let id = dom.in_scope(ScopeId::APP, || {
        let source = consume_context::<Session>();
        let screen = source.peek();
        let Screen::Unlocked(source) = &*screen else {
            panic!("locked source")
        };
        source
            .vault
            .entries()
            .iter()
            .find(|entry| entry.title == "Selected login")
            .unwrap()
            .id
            .clone()
    });
    let row = element(&initial, "data-id", &id);
    let click = || {
        Event::new(
            EventData::Mouse(SerializedMouseData::default()).into_any(),
            true,
        )
    };
    dom.runtime().handle_event("contextmenu", click(), row);
    let menu = element(&dom.render_immediate_to_vec().edits, "id", MENU_ACTION_ID);
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Move to another vault"));
    assert!(!html.contains("move-tree-checkbox"));
    dom.runtime().handle_event("click", click(), menu);
    let selection = dom.render_immediate_to_vec().edits;
    assert!(dioxus_ssr::render(&dom).contains("move-tree-checkbox"));
    assert!(!dioxus_ssr::render(&dom).contains("move-records-dialog"));

    let folder = element(&selection, "aria-label", "Select Accounts to move");
    dom.runtime().handle_event(
        "change",
        Event::new(
            EventData::Form(SerializedFormData::new("true".into(), vec![])).into_any(),
            true,
        ),
        folder,
    );
    dom.render_immediate_to_vec();
    {
        use kuchikiki::traits::TendrilSink;
        let document = kuchikiki::parse_html()
            .one(dioxus_ssr::render(&dom))
            .document_node;
        assert!(
            document
                .select_first("input[aria-label='Select Selected login to move'][checked]")
                .is_ok()
        );
        assert!(
            document
                .select_first("input[aria-label='Select Inside folder to move'][checked][disabled]")
                .is_ok()
        );
        assert!(
            document
                .select_first("input[aria-label='Select Leave this login to move'][checked]")
                .is_err()
        );
    }

    let choose_destination = element(&selection, "id", CONTINUE_ID);
    dom.runtime()
        .handle_event("click", click(), choose_destination);
    let dialog = dom.render_immediate_to_vec().edits;
    let picker = element(&dialog, "id", DESTINATION_ID);
    let submit = element(&dialog, "id", SUBMIT_ID);
    {
        use kuchikiki::traits::TendrilSink;
        let document = kuchikiki::parse_html()
            .one(dioxus_ssr::render(&dom))
            .document_node;
        assert!(
            document
                .select_first("dialog input[type=checkbox]")
                .is_err()
        );
    }
    let key = dom.in_scope(ScopeId::APP, || {
        let tabs = consume_context::<Signal<Tabs>>();
        let session = tabs.peek().sessions[1];
        let screen = session.peek();
        let Screen::Unlocked(target) = &*screen else {
            panic!("locked target")
        };
        format!("{}:{:?}", target.vault.id(), target.vault.path())
    });
    dom.runtime().handle_event(
        "change",
        Event::new(
            EventData::Form(SerializedFormData::new(key, vec![])).into_any(),
            true,
        ),
        picker,
    );
    dom.render_immediate_to_vec();
    dom.runtime().handle_event("click", click(), submit);
    dom.render_immediate_to_vec();

    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Selection moved to test"));
    assert!(!html.contains("move-records-dialog"));
    dom.in_scope(ScopeId::APP, || {
        let tabs = consume_context::<Signal<Tabs>>();
        let tabs = tabs.peek();
        let source = tabs.sessions[0].peek();
        let Screen::Unlocked(source) = &*source else {
            panic!("locked source")
        };
        assert_eq!(
            reopen(source)
                .entries
                .iter()
                .map(|entry| entry.title.as_str())
                .collect::<Vec<_>>(),
            ["Leave this login"]
        );
        let target = tabs.sessions[1].peek();
        let Screen::Unlocked(target) = &*target else {
            panic!("locked target")
        };
        let moved = reopen(target);
        assert_eq!(moved.folders[0].name, "Accounts");
        assert_eq!(
            moved
                .entries
                .iter()
                .find(|entry| entry.title == "Inside folder")
                .unwrap()
                .folder_id
                .as_ref(),
            Some(&moved.folders[0].id)
        );
        assert!(
            moved
                .entries
                .iter()
                .any(|entry| entry.title == "Selected login")
        );
        assert_eq!(moved.entries.len(), 2);
    });
}
