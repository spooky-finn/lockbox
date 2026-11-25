use super::*;
use crate::{
    desktop::test_support::{add, reopen, unlocked},
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
        let id = add(&mut workspace, "Selected login", "fictional user", "");
        add(&mut workspace, "Leave this login", "fictional user", "");
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
    rsx! { MoveRecordsButton {} }
}

fn element(edits: &[Mutation], name: &str) -> ElementId {
    edits
        .iter()
        .find_map(|edit| match edit {
            Mutation::SetAttribute {
                name: "id",
                value: AttributeValue::Text(value),
                id,
                ..
            } if value == name => Some(*id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("Missing element {name}"))
}

#[test]
fn dialog_moves_the_current_record_to_another_unlocked_tab() {
    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = VirtualDom::new(fixture);
    let trigger = element(&dom.rebuild_to_vec().edits, TRIGGER_ID);
    let click = || {
        Event::new(
            EventData::Mouse(SerializedMouseData::default()).into_any(),
            true,
        )
    };
    dom.runtime().handle_event("click", click(), trigger);
    let edits = dom.render_immediate_to_vec().edits;
    let picker = element(&edits, DESTINATION_ID);
    let submit = element(&edits, SUBMIT_ID);
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
        for (session, title) in tabs
            .peek()
            .sessions
            .iter()
            .zip(["Leave this login", "Selected login"])
        {
            let screen = session.peek();
            let Screen::Unlocked(workspace) = &*screen else {
                panic!("locked space")
            };
            let records = reopen(workspace);
            assert_eq!(records.entries.len(), 1);
            assert_eq!(records.entries[0].title, title);
        }
    });
}
