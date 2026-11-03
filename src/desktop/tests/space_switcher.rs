use super::*;
use crate::{
    access::auth::Locked,
    desktop::{context::BiometricProvider, security::Status},
    platform::touchid::tests::FakeBiometrics,
};
use dioxus::{
    dioxus_core::{AttributeValue, Mutation},
    html::{EventData, SerializedHtmlEventConverter, SerializedMouseData},
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

fn fixture() -> Element {
    let dir = use_hook(|| Rc::new(tempfile::tempdir().unwrap()));
    let personal = dir.path().join("Personal.lbx");
    let team = dir.path().join("profile-random.lbx");
    let missing = dir.path().join("missing.lbx");
    let settings = use_signal(|| {
        std::fs::write(&personal, []).unwrap();
        std::fs::write(&team, []).unwrap();
        Settings {
            config: Config {
                last_vault: Some(personal.clone()),
                recent_vaults: vec![personal.clone(), team.clone(), missing],
                space_names: [(team, "Design team".into())].into(),
                ..Config::default()
            },
            path: None,
        }
    });
    let state = use_signal(|| Screen::Locked(Box::new(Locked::new(Some(personal)))));
    let status = use_signal(Status::default);
    use_context_provider(|| state);
    use_context_provider(|| settings);
    use_context_provider(|| status);
    use_context_provider(|| BiometricProvider(Arc::new(FakeBiometrics::default())));
    use_context_provider(|| Rc::new(RefCell::new(crate::desktop::test_support::security())));
    rsx! { SpaceSwitcher {} }
}

#[test]
fn switcher_opens_lists_local_and_cloud_profiles_and_selects_without_unlocking() {
    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = VirtualDom::new(fixture);
    let initial = dom.rebuild_to_vec();
    let trigger = initial
        .edits
        .iter()
        .find_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "click" => Some(*id),
            _ => None,
        })
        .unwrap();
    let click = || {
        Event::new(
            EventData::Mouse(SerializedMouseData::default()).into_any(),
            true,
        )
    };
    dom.runtime().handle_event("click", click(), trigger);
    let changes = dom.render_immediate_to_vec();
    let html = dioxus_ssr::render(&dom);
    for text in [
        "Your spaces",
        "Personal",
        "Design team",
        "local",
        "cloud",
        "File unavailable",
        "Open another vault",
    ] {
        assert!(html.contains(text), "missing {text}");
    }
    assert!(html.contains("aria-expanded=\"true\""));
    let missing_button = html
        .split("<button")
        .find(|part| part.contains("missing.lbx") && part.contains("space-option"))
        .unwrap();
    assert!(
        missing_button
            .split('>')
            .next()
            .unwrap()
            .contains("disabled")
    );
    let cloud_button = changes
        .edits
        .iter()
        .find_map(|edit| match edit {
            Mutation::SetAttribute {
                name: "title",
                value: AttributeValue::Text(path),
                id,
                ..
            } if path.ends_with("profile-random.lbx") => Some(*id),
            _ => None,
        })
        .expect("cloud profile button");
    dom.runtime().handle_event("click", click(), cloud_button);
    dom.render_immediate_to_vec();
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        let screen = state.peek();
        let Screen::Locked(form) = &*screen else {
            panic!("selected profile must stay locked");
        };
        assert_eq!(form.space_name.as_deref(), Some("Design team"));
        assert!(form.password.is_empty());
    });
    assert!(!dioxus_ssr::render(&dom).contains("id=\"space-switch-panel\""));
}
