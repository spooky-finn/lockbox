use super::{
    button::IconButton,
    symbol::{Icon, Symbol},
};
use crate::app::context::{Session, edit_draft, set_field};
use crate::{
    app::{
        editor::{Field, algorithm_name},
        session::Screen,
    },
    entry::{Totp, TotpAlgorithm},
    totp,
};
use dioxus::prelude::*;

#[component]
pub(super) fn TotpEditor() -> Element {
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    let Some(draft) = &unlocked.draft else {
        return rsx! {};
    };
    rsx! {
        section { class: if draft.entry.totp.is_some() { "totp-editor stack" } else { "totp-editor stack is-empty" },
            h2 { "TOTP" }
            if let Some(settings) = &draft.entry.totp {
                label { r#for: "totp-secret", "Секрет или ссылка otpauth://" }
                div { class: "input-actions",
                    input { id: "totp-secret", r#type: if draft.show_totp { "text" } else { "password" }, autocomplete: "off",
                        value: "{settings.secret}", oninput: move |e| set_field(state, Field::Totp, e.value()) }
                    IconButton { icon: if draft.show_totp { Icon::EyeOff } else { Icon::Eye },
                        label: if draft.show_totp { "Скрыть секрет TOTP" } else { "Показать секрет TOTP" }, pressed: Some(draft.show_totp),
                        onclick: move |_| edit_draft(state, |d| d.show_totp = !d.show_totp),
                    }
                }
                details { summary { class: "icon-button", title: "Настройки TOTP", aria_label: "Настройки TOTP", Symbol { icon: Icon::Settings } }
                    div { class: "totp-options",
                        label { "Алгоритм"
                            select { value: algorithm_name(settings.algorithm), onchange: move |e| edit_draft(state, |d| {
                                if let Some(totp) = &mut d.entry.totp { totp.algorithm = match e.value().as_str() { "SHA256" => TotpAlgorithm::Sha256, "SHA512" => TotpAlgorithm::Sha512, _ => TotpAlgorithm::Sha1 }; }
                            }),
                                for algorithm in [TotpAlgorithm::Sha1, TotpAlgorithm::Sha256, TotpAlgorithm::Sha512] {
                                    option { value: algorithm_name(algorithm), "{algorithm_name(algorithm)}" }
                                }
                            }
                        }
                        label { "Цифр"
                            input { r#type: "number", min: "6", max: "8", value: "{settings.digits}", oninput: move |e| {
                                if let Ok(digits) = e.value().parse::<u8>() { edit_draft(state, |d| { if let Some(t) = &mut d.entry.totp { t.digits = digits; } }); }
                            } }
                        }
                        label { "Период, с"
                            input { r#type: "number", min: "{totp::PERIOD.start()}", max: "{totp::PERIOD.end()}", value: "{settings.period}", oninput: move |e| {
                                if let Ok(period) = e.value().parse::<u32>() { edit_draft(state, |d| { if let Some(t) = &mut d.entry.totp { t.period = period; } }); }
                            } }
                        }
                    }
                }
                IconButton { class: "danger quiet", icon: Icon::Trash, label: "Убрать TOTP", onclick: move |_| edit_draft(state, |d| { d.entry.totp = None; d.totp_error = None; }) }
            } else {
                IconButton { icon: Icon::Clock, label: "Добавить TOTP", onclick: move |_| edit_draft(state, |d| d.entry.totp = Some(Totp { secret: String::new(), algorithm: TotpAlgorithm::Sha1, digits: 6, period: 30 })) }
            }
            if let Some(error) = &draft.totp_error { p { class: "error", role: "alert", "{error}" } }
        }
    }
}
