use crate::app::editor::Field;
use crate::core::entry::{Totp, TotpAlgorithm};
use crate::ui::components::test_support::{mounted, update};
use crate::ui::context::{edit_unlocked, set_field};

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
