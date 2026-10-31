use crate::app::editor::replace_secret;
use crate::app::session::Screen;
use crate::ui::context::{Session, edit_locked};
use dioxus::prelude::*;
use zeroize::Zeroizing;

#[component]
pub(super) fn CloudSetupView() -> Element {
    let state = use_context::<Session>();
    let mut show_help = use_signal(|| false);
    let screen = state.read();
    let Screen::Locked(locked) = &*screen else {
        return rsx! {};
    };
    if show_help() {
        return rsx! {
            section { class: "cloud-setup", aria_label: "Подготовка Storj",
                h2 { "Подготовка Storj" }
                pre { class: "storj-guide", {include_str!("../../../docs/storj-setup.txt")} }
                a { href: "https://storj.dev/dcs/api/s3/credentials", target: "_blank", rel: "noopener noreferrer", "Документация Storj" }
                button { r#type: "button", class: "setup-button", onclick: move |_| show_help.set(false), "Вернуться к настройке" }
            }
        };
    }
    rsx! {
        form { class: "cloud-setup", onsubmit: move |event: FormEvent| {
                event.prevent_default();
                edit_locked(state, |locked| locked.continue_cloud());
            },
            p { class: "muted", "Подключите подготовленное хранилище Storj." }
            button { r#type: "button", class: "setup-button", onclick: move |_| show_help.set(true), "Как подготовить Storj" }
            label { r#for: "cloud-username", "Имя пользователя" }
            input { id: "cloud-username", value: "{locked.cloud.username}", autocomplete: "off",
                oninput: move |event| edit_locked(state, |locked| locked.cloud.username = event.value()), }
            label { r#for: "cloud-space", "Имя хранилища" }
            input { id: "cloud-space", value: "{locked.cloud.space_name}", autocomplete: "off",
                oninput: move |event| edit_locked(state, |locked| locked.cloud.space_name = event.value()), }
            label { r#for: "cloud-url", "Base URL" }
            input { id: "cloud-url", r#type: "url", value: "{locked.cloud.base_url}", placeholder: "https://…", autocomplete: "off",
                oninput: move |event| edit_locked(state, |locked| locked.cloud.base_url = event.value()), }
            label { r#for: "cloud-bucket", "Bucket" }
            input { id: "cloud-bucket", value: "{locked.cloud.bucket}", autocomplete: "off", placeholder: "lockbox-team",
                oninput: move |event| edit_locked(state, |locked| locked.cloud.bucket = event.value()), }
            label { r#for: "cloud-access", "Access Key" }
            input { id: "cloud-access", r#type: "password", value: locked.cloud.access_key.as_str(), autocomplete: "off",
                oninput: move |event| {
                    let value = Zeroizing::new(event.value());
                    edit_locked(state, |locked| replace_secret(&mut locked.cloud.access_key, &value));
                }, }
            label { r#for: "cloud-secret", "Secret Key" }
            input { id: "cloud-secret", r#type: "password", value: locked.cloud.secret_key.as_str(), autocomplete: "off",
                oninput: move |event| {
                    let value = Zeroizing::new(event.value());
                    edit_locked(state, |locked| replace_secret(&mut locked.cloud.secret_key, &value));
                }, }
            p { class: "muted", "Проверка полей не обращается к Storj." }
            button { r#type: "submit", class: "setup-button", "Продолжить" }
        }
    }
}
