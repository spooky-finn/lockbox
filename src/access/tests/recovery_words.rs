use super::*;

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

#[test]
fn whole_phrase_paste_normalizes_case_and_whitespace_from_any_field() {
    let pasted = PHRASE.to_uppercase().replace(' ', "\n\t ");
    let (words, next) = replace_words("old words", 7, &pasted).unwrap();
    assert_eq!(words.as_str(), PHRASE);
    assert_eq!(next, WORD_COUNT);
    assert!(RecoveryPhrase::parse(&words).is_ok());
}

#[test]
fn editing_and_clearing_keep_other_words_in_their_numbered_positions() {
    let (words, next) = replace_words(PHRASE, 3, "").unwrap();
    assert_eq!(next, 3);
    assert_eq!(slots(&words)[3], "");
    assert_eq!(slots(&words)[4], "abandon");
    assert_eq!(slots(&words)[11], "about");
    assert!(RecoveryPhrase::parse(&words).is_err());
    let (words, next) = replace_words(&words, 3, "ABANDON ").unwrap();
    assert_eq!(next, 4);
    assert_eq!(words.as_str(), PHRASE);
}

#[test]
fn partial_paste_preserves_neighbors_and_never_silently_truncates() {
    let (words, next) = replace_words(PHRASE, 8, "act\nactor").unwrap();
    assert_eq!(next, 10);
    assert_eq!(
        &slots(&words)[7..],
        &["abandon", "act", "actor", "abandon", "about"]
    );
    assert!(replace_words(PHRASE, 11, "act actor").is_none());
    assert!(replace_words(PHRASE, 0, &format!("{PHRASE} extra")).is_none());
    assert_eq!(replace_words("", 0, "notaword ").unwrap().1, 0);
}

#[test]
fn suggestions_only_use_matching_bip39_words_and_stay_bounded() {
    assert!(suggestions("").is_empty());
    assert!(suggestions("notaword").is_empty());
    assert_eq!(suggestions("aban"), ["abandon"]);
    assert_eq!(
        suggestions("act"),
        ["act", "action", "actor", "actress", "actual"]
    );
    assert_eq!(suggestions("a").len(), 6);
    for word in suggestions("ab") {
        assert!(word.starts_with("ab"));
        assert!(Language::English.find_word(word).is_some());
    }
}

mod interaction {
    use super::*;
    use crate::{
        access::auth::Locked,
        settings::{config::Config, state::Settings},
    };
    use dioxus::{
        dioxus_core::{ElementId, Mutation},
        html::{
            EventData, SerializedFocusData, SerializedFormData, SerializedHtmlEventConverter,
            SerializedKeyboardData, SerializedMouseData,
        },
    };

    fn fixture() -> Element {
        let state = use_signal(|| {
            let mut locked = Locked::new(None);
            locked.importing_words = true;
            locked.words_saved = true;
            Screen::Locked(Box::new(locked))
        });
        let settings = use_signal(|| Settings {
            config: Config::default(),
            path: None,
        });
        use_context_provider(|| state);
        use_context_provider(|| settings);
        rsx! { RecoveryWordsInput { readonly: false, disabled: false } }
    }

    struct Editor {
        dom: VirtualDom,
        inputs: Vec<ElementId>,
    }

    impl Editor {
        fn new() -> Self {
            dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
            let mut dom = VirtualDom::new(fixture);
            let changes = dom.rebuild_to_vec();
            let inputs = changes
                .edits
                .iter()
                .filter_map(|edit| match edit {
                    Mutation::NewEventListener { name, id } if name == "input" => Some(*id),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(inputs.len(), WORD_COUNT);
            Self { dom, inputs }
        }

        fn send(&mut self, name: &str, index: usize, data: EventData) -> Event<dyn std::any::Any> {
            let event = Event::new(data.into_any(), true);
            self.dom
                .runtime()
                .handle_event(name, event.clone(), self.inputs[index]);
            let changes = self.dom.render_immediate_to_vec();
            let inputs = changes
                .edits
                .iter()
                .filter_map(|edit| match edit {
                    Mutation::NewEventListener { name, id } if name == "input" => Some(*id),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if inputs.len() == WORD_COUNT {
                self.inputs = inputs;
            }
            event
        }

        fn input(&mut self, index: usize, value: &str) {
            self.send(
                "input",
                index,
                EventData::Form(SerializedFormData::new(value.into(), vec![])),
            );
        }

        fn key(&mut self, index: usize, key: Key) -> Event<dyn std::any::Any> {
            self.send(
                "keydown",
                index,
                EventData::Keyboard(SerializedKeyboardData::new(
                    key,
                    Code::Unidentified,
                    Location::Standard,
                    false,
                    Modifiers::empty(),
                    false,
                )),
            )
        }

        fn words(&self) -> Zeroizing<String> {
            self.dom.in_scope(ScopeId::APP, || {
                let state = consume_context::<Session>();
                let screen = state.read();
                let Screen::Locked(locked) = &*screen else {
                    panic!("locked editor")
                };
                assert!(
                    !locked.words_saved,
                    "editing must clear saved acknowledgment"
                );
                Zeroizing::new(locked.recovery_words.to_string())
            })
        }
    }

    #[test]
    fn keyboard_and_pointer_accept_suggestions_without_submitting() {
        let mut editor = Editor::new();
        editor.input(0, "act");
        assert!(dioxus_ssr::render(&editor.dom).contains("role=\"listbox\""));
        assert!(!editor.key(0, Key::ArrowDown).default_action_enabled());
        assert!(!editor.key(0, Key::Enter).default_action_enabled());
        assert_eq!(slots(&editor.words())[0], "action");

        editor.input(1, "aban");
        // Suggestions already mounted during input; focus moves can close/reopen them.
        editor.send("blur", 1, EventData::Focus(SerializedFocusData {}));
        let event = Event::new(EventData::Focus(SerializedFocusData {}).into_any(), true);
        editor
            .dom
            .runtime()
            .handle_event("focus", event, editor.inputs[1]);
        let changes = editor.dom.render_immediate_to_vec();
        let option = changes
            .edits
            .iter()
            .find_map(|edit| match edit {
                Mutation::NewEventListener { name, id } if name == "click" => Some(*id),
                _ => None,
            })
            .expect("suggestion click listener");
        let event = Event::new(
            EventData::Mouse(SerializedMouseData::default()).into_any(),
            true,
        );
        editor.dom.runtime().handle_event("click", event, option);
        editor.dom.render_immediate_to_vec();
        assert_eq!(slots(&editor.words())[1], "abandon");

        editor.input(2, "abo");
        assert!(!editor.key(2, Key::Tab).default_action_enabled());
        assert_eq!(slots(&editor.words())[2], "about");
        editor.input(3, "aban");
        editor.key(3, Key::Escape);
        assert!(!dioxus_ssr::render(&editor.dom).contains("role=\"listbox\""));
    }

    #[test]
    fn full_phrase_paste_checks_checksum_and_preserves_positions_when_cleared() {
        let mut editor = Editor::new();
        editor.input(5, &PHRASE.to_uppercase().replace(' ', "\n"));
        assert_eq!(editor.words().as_str(), PHRASE);
        assert!(dioxus_ssr::render(&editor.dom).contains("Recovery phrase is valid"));
        editor.input(11, "abandon");
        assert!(
            dioxus_ssr::render(&editor.dom)
                .contains("Enter a valid 12-word English recovery phrase")
        );
        editor.input(3, "");
        assert_eq!(slots(&editor.words())[3], "");
        assert_eq!(slots(&editor.words())[4], "abandon");
        assert!(dioxus_ssr::render(&editor.dom).contains("11 of 12 words entered"));
    }

    #[test]
    fn only_the_focused_word_is_visible_and_blur_masks_all_words() {
        use kuchikiki::traits::TendrilSink;

        let mut editor = Editor::new();
        let field_types = |editor: &Editor| {
            let document = kuchikiki::parse_html().one(dioxus_ssr::render(&editor.dom));
            (0..WORD_COUNT)
                .map(|index| {
                    document
                        .document_node
                        .select_first(&format!("#recovery-word-{index}"))
                        .unwrap()
                        .attributes
                        .borrow()
                        .get("type")
                        .unwrap()
                        .to_owned()
                })
                .collect::<Vec<_>>()
        };
        let expected_types = |focused| {
            (0..WORD_COUNT)
                .map(|index| if index == focused { "text" } else { "password" })
                .collect::<Vec<_>>()
        };

        editor.input(0, PHRASE);
        editor.send("blur", 0, EventData::Focus(SerializedFocusData {}));
        assert!(field_types(&editor).iter().all(|kind| kind == "password"));

        editor.send("focus", 4, EventData::Focus(SerializedFocusData {}));
        assert_eq!(field_types(&editor), expected_types(4));
        editor.key(4, Key::Escape);
        assert_eq!(field_types(&editor), expected_types(4));
        assert!(!dioxus_ssr::render(&editor.dom).contains("role=\"listbox\""));

        editor.send("blur", 4, EventData::Focus(SerializedFocusData {}));
        editor.send("focus", 9, EventData::Focus(SerializedFocusData {}));
        assert_eq!(field_types(&editor), expected_types(9));
        assert_eq!(editor.words().as_str(), PHRASE);

        editor.send("blur", 9, EventData::Focus(SerializedFocusData {}));
        assert!(field_types(&editor).iter().all(|kind| kind == "password"));
    }

    #[test]
    fn oversized_paste_restores_fields_and_editor_remains_usable() {
        let mut editor = Editor::new();
        editor.input(0, PHRASE);
        editor.input(0, &format!("{PHRASE} extra"));
        assert_eq!(editor.words().as_str(), PHRASE);
        assert!(dioxus_ssr::render(&editor.dom).contains("Extra words were not added"));
        assert!(!dioxus_ssr::render(&editor.dom).contains("value=\"extra\""));
        editor.input(0, "aban");
        assert!(!editor.key(0, Key::Enter).default_action_enabled());
        assert_eq!(editor.words().as_str(), PHRASE);
        assert!(dioxus_ssr::render(&editor.dom).contains("Recovery phrase is valid"));
    }
}
