use crate::ui::components::test_support::{mounted, update};
use crate::ui::runtime;

#[test]
fn creation_starts_with_storage_choice_and_cloud_has_separate_fields() {
    let mut dom = mounted();
    update(&mut dom, |_| runtime::begin_create());
    let html = dioxus_ssr::render(&dom);
    assert!(html.contains("Однопользовательское"));
    assert!(html.contains("Многопользовательское"));
    assert!(!html.contains("id=\"master-password\""));
    update(&mut dom, |state| {
        crate::ui::context::edit_locked(state, |form| {
            form.storage_kind = Some(crate::app::auth::StorageKind::Cloud);
        });
    });
    let html = dioxus_ssr::render(&dom);
    for label in [
        "Имя пользователя",
        "Имя хранилища",
        "Base URL",
        "Access Key",
        "Secret Key",
    ] {
        assert!(html.contains(label));
    }
    assert!(!html.contains("id=\"master-password\""));
    update(&mut dom, |_| runtime::begin_create());
    assert!(!dioxus_ssr::render(&dom).contains("id=\"cloud-secret\""));
}
