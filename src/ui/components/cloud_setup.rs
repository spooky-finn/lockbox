use crate::ui::i18n::Text;

use crate::app::editor::replace_secret;
use crate::app::session::Screen;
use crate::ui::context::{Session, edit_locked};
use dioxus::prelude::*;
use zeroize::Zeroizing;

#[component]
pub(super) fn CloudSetupView() -> Element {
    let language = crate::ui::i18n::use_language();
    let state = use_context::<Session>();
    let mut show_help = use_signal(|| false);
    let screen = state.read();
    let Screen::Locked(locked) = &*screen else {
        return rsx! {};
    };

    if show_help() {
        return rsx! {
            section { class: "cloud-setup", aria_label: language.text(Text::StorjSetup),
                h2 { {language.text(Text::StorjSetup)} }
                pre { class: "storj-guide",
                    {match language {
                        crate::core::language::Language::English => include_str!("../../../docs/storj-setup.en.txt"),
                        crate::core::language::Language::Russian => include_str!("../../../docs/storj-setup.txt"),
                    }}
                }
                a { href: "https://storj.dev/dcs/api/s3/credentials", target: "_blank", rel: "noopener noreferrer", {language.text(Text::StorjDocumentation)} }
                button { r#type: "button", class: "setup-button", onclick: move |_| show_help.set(false), {language.text(Text::BackToSetup)} }
            }
        };
    }
    rsx! {
        form { class: "cloud-setup", onsubmit: move |event: FormEvent| {
                event.prevent_default();
                edit_locked(state, |locked| locked.continue_cloud());
            },
            p { class: "muted", {language.text(Text::ConnectYourStorjStorage)} }
            button { r#type: "button", class: "setup-button", onclick: move |_| show_help.set(true), {language.text(Text::HowToSetUpStorj)} }
            label { r#for: "cloud-username", {language.text(Text::Username)} }
            input { id: "cloud-username", value: "{locked.cloud.username}", autocomplete: "off",
                oninput: move |event| edit_locked(state, |locked| locked.cloud.username = event.value()), }
            if !locked.joining {
                label { r#for: "cloud-space", {language.text(Text::VaultName)} }
                input { id: "cloud-space", value: "{locked.cloud.space_name}", autocomplete: "off",
                    oninput: move |event| edit_locked(state, |locked| locked.cloud.space_name = event.value()), }
            }
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
            p { class: "muted", {language.text(Text::FieldValidationDoesNotConnectToStorj)} }
            button { r#type: "submit", class: "setup-button", {language.text(Text::Continue)} }
        }
    }
}
