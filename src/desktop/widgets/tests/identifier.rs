use super::*;
use crate::{
    desktop::{context::SecurityHandle, runtime},
    platform::clipboard::{Guard, tests::FakeBoard},
    settings::language::Language,
};
use dioxus::{
    dioxus_core::{AttributeValue, Mutation},
    html::{EventData, SerializedHtmlEventConverter, SerializedMouseData},
};
use kuchikiki::traits::TendrilSink;

const FULL: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn abbreviates_by_characters_and_preserves_short_values() {
    for value in ["", "1234", "12345678"] {
        assert_eq!(abbreviate(value), value);
    }
    assert_eq!(abbreviate(FULL), "0123…cdef");
    assert_eq!(abbreviate("123456789"), "1234…6789");
    assert_eq!(abbreviate("абвгдежзийкл"), "абвг…ийкл");
    assert_eq!(abbreviate("abcdef0123456789:0"), "abcd…89:0");
}

fn fixture() -> Element {
    let workspace = crate::desktop::view_test_support::fixture();
    let value = use_signal(|| FULL.to_owned());
    use_context_provider(|| value);
    rsx! { {workspace} Identifier { value: value() } }
}

#[test]
fn button_shows_short_value_and_copies_current_full_value() {
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let _guard = executor.enter();
    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = VirtualDom::new(fixture);
    let changes = dom.rebuild_to_vec();
    let board = FakeBoard::default();
    dom.in_scope(ScopeId::APP, || {
        consume_context::<SecurityHandle>().borrow_mut().clipboard =
            Guard::new(Box::new(board.clone()));
    });
    let html = dioxus_ssr::render(&dom);
    let document = kuchikiki::parse_html().one(html.clone()).document_node;
    assert_eq!(
        document
            .select_first(".identifier code")
            .unwrap()
            .text_contents(),
        "0123…cdef"
    );
    assert!(!html.contains(FULL));
    let label = Language::Russian.format(Text::CopyIdentifier, &[abbreviate(FULL)]);
    let button = changes
        .edits
        .iter()
        .find_map(|edit| match edit {
            Mutation::SetAttribute {
                name: "aria-label",
                value: AttributeValue::Text(value),
                id,
                ..
            } if value == &label => Some(*id),
            _ => None,
        })
        .unwrap();
    let click = |dom: &mut VirtualDom| {
        let mouse = Event::new(
            EventData::Mouse(SerializedMouseData::default()).into_any(),
            true,
        );
        dom.runtime().handle_event("click", mouse, button);
        dom.render_immediate_to_vec();
    };
    click(&mut dom);
    assert_eq!(board.0.borrow().as_deref(), Some(FULL));

    let updated = "fedcba9876543210fedcba9876543210";
    dom.in_scope(ScopeId::APP, || {
        consume_context::<Signal<String>>().set(updated.to_owned())
    });
    dom.render_immediate_to_vec();
    click(&mut dom);
    assert_eq!(board.0.borrow().as_deref(), Some(updated));

    dom.in_scope(ScopeId::APP, || runtime::lock(None));
    dom.render_immediate_to_vec();
    assert!(board.0.borrow().is_none());
    click(&mut dom);
    assert!(board.0.borrow().is_none());
}
