use super::*;
use crate::app::context::{
    BiometricProvider, Clock, Session, edit_draft, edit_unlocked, set_field,
};
use crate::touchid::tests::FakeBiometrics;
use crate::{
    app::{
        editor::Field,
        security::{Security, Status},
        settings::Settings,
        time::unix_now,
    },
    autolock::{AutoLock, ScreenWatch},
    clipboard::Guard,
    config::Config,
    entry::{Entry, Totp, TotpAlgorithm},
};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    time::{Instant, SystemTime},
};

#[derive(Clone, Default)]
struct RenderCounts(Rc<std::cell::Cell<usize>>, Rc<std::cell::Cell<usize>>);

#[component]
fn SessionProbe() -> Element {
    let count = use_context::<RenderCounts>();
    let state = use_context::<Session>();
    let _screen = state.read();
    count.0.set(count.0.get() + 1);
    rsx! {}
}

#[component]
fn ClockProbe() -> Element {
    let count = use_context::<RenderCounts>();
    let _now = (use_context::<Clock>().0)();
    count.1.set(count.1.get() + 1);
    rsx! {}
}

fn fixture() -> Element {
    let dir = use_hook(|| std::rc::Rc::new(tempfile::tempdir().unwrap()));
    let counts = use_hook(RenderCounts::default);
    use_context_provider(|| counts.clone());
    let state = use_signal(|| {
        let mut unlocked = crate::app::tests::unlocked(&dir);
        let mut entry = Entry::new();
        entry.title = "GitLab".into();
        entry.username = "alice".into();
        entry.password = "private-test-password".into();
        entry.url = "https://gitlab.com".into();
        entry.totp = Some(Totp {
            secret: "JBSWY3DPEHPK3PXP".into(),
            algorithm: TotpAlgorithm::Sha1,
            digits: 6,
            period: 30,
        });
        unlocked.selected = Some(entry.id.clone());
        unlocked.vault.entries.push(entry);
        Screen::Unlocked(Box::new(unlocked))
    });
    let settings = use_signal(|| Settings {
        config: Config::default(),
        path: None,
    });
    let status = use_signal(Status::default);
    let clock = use_signal(unix_now);
    let native = use_hook(|| {
        Rc::new(RefCell::new(Security {
            clipboard: Guard::new(Box::new(crate::clipboard::tests::FakeBoard::default())),
            auto_lock: AutoLock::new(5, Instant::now(), SystemTime::now()),
            screen_watch: ScreenWatch::default(),
        }))
    });
    use_context_provider(|| state);
    use_context_provider(|| settings);
    use_context_provider(|| status);
    use_context_provider(|| Clock(clock));
    use_context_provider(|| native.clone());
    use_context_provider(|| BiometricProvider(Arc::new(FakeBiometrics::default())));
    let body = if matches!(&*state.read(), Screen::Locked(_)) {
        rsx! { LockedView {} }
    } else {
        rsx! { UnlockedView {} }
    };
    rsx! { {body} SessionProbe {} ClockProbe {} }
}

fn mounted() -> VirtualDom {
    let mut dom = VirtualDom::new(fixture);
    dom.rebuild_in_place();
    dom
}

fn update(dom: &mut VirtualDom, edit: impl FnOnce(Session)) {
    dom.in_scope(ScopeId::APP, || edit(consume_context::<Session>()));
    dom.render_immediate_to_vec();
}

#[test]
fn selected_entry_hides_secrets_and_escapes_user_text() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.vault.entries[0].title = "<script>bad()</script>".into();
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("alice"));
    assert!(html.contains("bad()"));
    assert!(!html.contains("<script>bad()"));
    assert!(!html.contains("private-test-password"));
    assert!(!html.contains("JBSWY3DPEHPK3PXP"));
    assert!(html.contains("Код TOTP"));
}

#[test]
fn showing_then_locking_removes_the_entry_and_password() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.show_password = true)
    });
    assert!(dioxus_ssr::render(&dom).contains("private-test-password"));
    update(&mut dom, |_| runtime::lock(None));
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Мастер-пароль"));
    assert!(!html.contains("private-test-password"));
    assert!(!html.contains("alice"));
    assert!(!html.contains("entry-detail"));
}

#[test]
fn editor_renders_generator_icons_and_reactive_totp_link_fields() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.start_new());
        edit_draft(state, |d| {
            d.generator_open = true;
            d.icon_picker_open = true;
            d.icon_query = "gitlab".into();
            d.entry.totp = Some(Totp {
                secret: String::new(),
                algorithm: TotpAlgorithm::Sha1,
                digits: 6,
                period: 30,
            });
        });
        set_field(
            state,
            Field::Totp,
            "otpauth://totp/GitLab:alice?secret=JBSWY3DPEHPK3PXP".into(),
        );
    });
    let html = dioxus_ssr::render(&dom);
    for text in [
        "Новая запись",
        "Генератор паролей",
        "Выбор иконки",
        "SHA256",
        "Сохранить",
        "value=\"alice\"",
    ] {
        assert!(html.contains(text), "missing {text}");
    }
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.error = u.save_draft().err())
    });
    assert!(!dioxus_ssr::render(&dom).contains("id=\"entry-title\""));
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        let screen = state.read();
        let Screen::Unlocked(u) = &*screen else {
            panic!("locked");
        };
        let vault = crate::app::tests::reopen(u);
        assert_eq!(vault.entries.len(), 2);
        assert_eq!(vault.entries[1].username, "alice");
    });
}

#[test]
fn search_reacts_and_failed_save_preserves_the_form() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.search = "missing".into();
            u.start_new();
            u.draft.as_mut().unwrap().entry.totp = Some(Totp {
                secret: "invalid!".into(),
                algorithm: TotpAlgorithm::Sha1,
                digits: 6,
                period: 30,
            });
        })
    });
    update(&mut dom, |state| {
        set_field(state, Field::Title, "Unsaved".into());
        edit_unlocked(state, |u| u.error = u.save_draft().err());
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Записей не найдено"));
    assert!(html.contains("value=\"Unsaved\""));
    assert!(html.contains("TOTP:"));
    assert!(html.contains("Сохранить"));
}
#[test]
fn submitting_the_dioxus_form_saves_and_prevents_browser_navigation() {
    use dioxus::dioxus_core::Mutation;
    use dioxus::html::{EventData, SerializedFormData, SerializedHtmlEventConverter};
    dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
    let mut dom = mounted();
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        edit_unlocked(state, |u| u.start_new());
        set_field(state, Field::Title, "From submit event".into());
    });
    let changes = dom.render_immediate_to_vec();
    let submit_id = changes
        .edits
        .iter()
        .find_map(|edit| match edit {
            Mutation::NewEventListener { name, id } if name == "submit" => Some(*id),
            _ => None,
        })
        .expect("editor has a submit listener");
    let event = Event::new(
        EventData::Form(SerializedFormData::new(String::new(), vec![])).into_any(),
        true,
    );
    dom.runtime()
        .handle_event("submit", event.clone(), submit_id);
    assert!(!event.default_action_enabled());
    dom.render_immediate_to_vec();
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("From submit event"));
    assert!(!html.contains("id=\"entry-title\""));
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        let screen = state.read();
        let Screen::Unlocked(u) = &*screen else {
            panic!("locked");
        };
        assert_eq!(
            crate::app::tests::reopen(u).entries[1].title,
            "From submit event"
        );
    });
}

#[test]
fn worker_results_open_the_vault_and_report_failure_or_disconnect() {
    use crate::app::auth::Opened;
    use std::sync::mpsc;
    let mut dom = mounted();
    let dir = tempfile::tempdir().unwrap();
    update(&mut dom, |_| runtime::lock(None));
    update(&mut dom, |state| {
        let (sender, receiver) = mpsc::channel();
        crate::app::context::edit_locked(state, |form| form.pending = Some(receiver));
        runtime::poll_workers();
        assert!(matches!(&*state.peek(), Screen::Locked(_)));
        sender
            .send(Ok(Opened {
                vault: crate::app::tests::unlocked(&dir).vault,
                with_password: true,
            }))
            .unwrap_or_else(|_| panic!("receiver dropped"));
        runtime::poll_workers();
        assert!(matches!(&*state.peek(), Screen::Unlocked(_)));
    });
    update(&mut dom, |_| runtime::lock(None));
    update(&mut dom, |state| {
        let (sender, receiver) = mpsc::channel();
        crate::app::context::edit_locked(state, |form| form.pending = Some(receiver));
        sender
            .send(Err("wrong password".into()))
            .unwrap_or_else(|_| panic!("receiver dropped"));
        runtime::poll_workers();
        let screen = state.peek();
        let Screen::Locked(form) = &*screen else {
            panic!("open after failure");
        };
        assert_eq!(form.error.as_deref(), Some("wrong password"));
        assert!(form.pending.is_none());
    });
    update(&mut dom, |state| {
        let (sender, receiver) = mpsc::channel();
        crate::app::context::edit_locked(state, |form| form.pending = Some(receiver));
        drop(sender);
        runtime::poll_workers();
        let screen = state.peek();
        let Screen::Locked(form) = &*screen else {
            panic!("open after disconnect");
        };
        assert_eq!(form.error.as_deref(), Some("Разблокировка прервалась"));
        assert!(form.pending.is_none());
    });
}

#[test]
fn background_tick_clears_clipboard_and_locks_without_input() {
    use crate::app::context::SecurityHandle;
    use std::time::Duration;
    let mut dom = mounted();
    update(&mut dom, |_| {
        let native = consume_context::<SecurityHandle>();
        native
            .borrow_mut()
            .clipboard
            .copy("secret", Instant::now() - Duration::from_secs(21))
            .unwrap();
        runtime::tick();
        assert!(
            native
                .borrow()
                .clipboard
                .remaining(Instant::now())
                .is_none()
        );
        native.borrow_mut().auto_lock = AutoLock::new(
            1,
            Instant::now() - Duration::from_secs(61),
            SystemTime::now() - Duration::from_secs(61),
        );
        runtime::tick();
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Мастер-пароль"));
    assert!(html.contains("долго не было действий"));
}

#[test]
fn first_input_after_sleep_locks_before_resetting_activity() {
    use crate::app::context::SecurityHandle;
    use std::time::Duration;
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |workspace| workspace.start_new());
        let native = consume_context::<SecurityHandle>();
        native.borrow_mut().auto_lock = AutoLock::new(
            1,
            Instant::now() - Duration::from_secs(61),
            SystemTime::now() - Duration::from_secs(61),
        );
        runtime::record_activity();
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Мастер-пароль"));
    assert!(!html.contains("id=\"entry-title\""));
}

#[test]
fn clock_status_and_activity_do_not_rerender_session_subscribers() {
    use crate::app::context::SecurityHandle;
    let mut dom = mounted();
    let counts = dom.in_scope(ScopeId::APP, consume_context::<RenderCounts>);
    let before = (counts.0.get(), counts.1.get());
    dom.in_scope(ScopeId::APP, || {
        let mut clock = consume_context::<Clock>().0;
        *clock.write() += 1;
        let native = consume_context::<SecurityHandle>();
        native
            .borrow_mut()
            .clipboard
            .copy("secret", Instant::now())
            .unwrap();
        runtime::publish_status();
        runtime::record_activity();
    });
    dom.render_immediate_to_vec();
    assert_eq!(
        counts.0.get(),
        before.0,
        "session subscriber rerendered for an unrelated signal"
    );
    assert!(counts.1.get() > before.1, "clock subscriber did not update");
    assert!(dioxus_ssr::render(&dom).contains("буфер очистится через"));
}

#[test]
fn tree_records_show_only_titles_and_search_reveals_collapsed_ancestors() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            let parent = crate::folder::Folder::new("Работа".into(), None);
            let child = crate::folder::Folder::new("Сервисы".into(), Some(parent.id.clone()));
            u.vault.entries[0].folder_id = Some(child.id.clone());
            u.directories.collapsed.insert(parent.id.clone());
            u.directories.collapsed.insert(child.id.clone());
            u.vault.folders = vec![parent, child];
            u.search = "alice".into();
        })
    });
    let html = dioxus_ssr::render(&dom);
    let tree = html
        .split("aria-label=\"Записи и папки\"")
        .nth(1)
        .unwrap()
        .split("</nav>")
        .next()
        .unwrap();
    for title in ["Работа", "Сервисы", "GitLab"] {
        assert!(tree.contains(title), "missing {title}");
    }
    assert!(
        !tree.contains("alice"),
        "usernames should not appear in record rows"
    );
    assert!(
        !tree.contains("entry-icon"),
        "record rows should not have icons"
    );
    assert!(!tree.contains("<small"));
    update(&mut dom, |state| edit_unlocked(state, |u| u.search.clear()));
    let html = dioxus_ssr::render(&dom);
    let tree = html
        .split("aria-label=\"Записи и папки\"")
        .nth(1)
        .unwrap()
        .split("</nav>")
        .next()
        .unwrap();
    assert!(
        !tree.contains("GitLab"),
        "clearing search should preserve collapsed state"
    );
}

#[test]
fn folder_editor_saves_and_record_editor_uses_the_selected_folder() {
    let mut dom = mounted();
    update(&mut dom, |state| {
        edit_unlocked(state, |u| u.start_directory())
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("directory-name"));
    assert!(html.contains("directory-parent"));
    update(&mut dom, |state| {
        edit_unlocked(state, |u| {
            u.directories.draft.as_mut().unwrap().name = "Работа".into();
            u.save_directory().unwrap();
        })
    });
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("directory-detail"));
    assert!(html.contains("Изменить папку"));
    update(&mut dom, |state| edit_unlocked(state, |u| u.start_new()));
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("entry-folder"));
    assert!(html.contains("Работа"));
    dom.in_scope(ScopeId::APP, || {
        let state = consume_context::<Session>();
        let screen = state.read();
        let Screen::Unlocked(u) = &*screen else {
            panic!("locked")
        };
        assert_eq!(
            u.draft.as_ref().unwrap().entry.folder_id,
            u.directories.selected
        );
        assert_eq!(crate::app::tests::reopen(u).folders[0].name, "Работа");
    });
}
