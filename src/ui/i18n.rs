//! Local, bundled translations. Vault content never passes through this catalog.

use crate::{app::settings::Settings, core::language::Language};
use dioxus::prelude::*;

mod catalog;
use catalog::CATALOG;
pub(crate) use catalog::Text;

pub(crate) fn use_language() -> Language {
    use_context::<Signal<Settings>>().read().config.language
}

impl Language {
    pub(crate) fn text(self, key: Text) -> &'static str {
        let (ru, en) = key.translations();
        match self {
            Self::English => en,
            Self::Russian => ru,
        }
    }

    /// Interpolate only the template; inserted values can contain arbitrary braces.
    pub(crate) fn format(self, key: Text, values: &[String]) -> String {
        interpolate(self.text(key), values)
    }

    /// Translate domain diagnostics at the presentation boundary, including nested errors.
    pub(crate) fn message(self, message: &str) -> String {
        for key in CATALOG {
            let (ru, en) = key.translations();
            let (source, translated) = match self {
                Self::English => (ru, en),
                Self::Russian => (en, ru),
            };
            if source == message {
                return translated.to_owned();
            }
            if let Some(values) = captures(source, message) {
                let values = values
                    .into_iter()
                    .map(|value| self.message(value))
                    .collect::<Vec<_>>();
                return interpolate(translated, &values);
            }
        }
        message.to_owned()
    }

    pub(crate) fn minutes_label(self, minutes: u32) -> String {
        if minutes == 0 {
            self.text(Text::Never).to_owned()
        } else {
            self.format(Text::Min, &[minutes.to_string()])
        }
    }
}

fn interpolate(template: &str, values: &[String]) -> String {
    let mut output = String::new();
    let mut rest = template;
    for value in values {
        let Some((prefix, placeholder)) = rest.split_once('{') else {
            break;
        };
        let Some((_, suffix)) = placeholder.split_once('}') else {
            break;
        };
        output.push_str(prefix);
        output.push_str(value);
        rest = suffix;
    }
    output.push_str(rest);
    output
}

fn captures<'a>(template: &str, mut message: &'a str) -> Option<Vec<&'a str>> {
    let (prefix, mut template) = template.split_once('{')?;
    message = message.strip_prefix(prefix)?;
    let mut values = Vec::new();
    loop {
        let (_, suffix) = template.split_once('}')?;
        if let Some((separator, next)) = suffix.split_once('{') {
            let (value, rest) = message.split_once(separator)?;
            values.push(value);
            message = rest;
            template = next;
        } else {
            values.push(message.strip_suffix(suffix)?);
            return Some(values);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_unique_and_templates_keep_the_same_arguments() {
        let mut sources = std::collections::BTreeSet::new();
        for key in CATALOG {
            let (ru, en) = key.translations();
            assert!(sources.insert(ru), "duplicate translation: {ru}");
            assert!(!en.is_empty(), "missing translation: {ru}");
            assert_eq!(ru.matches('{').count(), en.matches('{').count(), "{ru}");
            assert_eq!(ru.matches('}').count(), en.matches('}').count(), "{ru}");
        }
    }

    #[test]
    fn messages_translate_nested_diagnostics_and_preserve_unknown_details() {
        assert_eq!(
            Language::English.message("Хранилище заблокировано: долго не было действий"),
            "Vault locked: no activity for a while"
        );
        assert_eq!(
            Language::English
                .message("Не удалось сохранить: файл хранилища недоступен: /tmp/Работа.lbx"),
            "Could not save: vault file is unavailable: /tmp/Работа.lbx"
        );
        assert_eq!(
            Language::English.message("TOTP: секрет TOTP пустой"),
            "TOTP: TOTP secret is empty"
        );
        assert_eq!(
            Language::English.message("Touch ID: палец не распознан"),
            "Touch ID: fingerprint not recognized"
        );
        assert_eq!(
            Language::Russian.message("Could not save: vault file is unavailable: /tmp/Работа.lbx"),
            "Не удалось сохранить: файл хранилища недоступен: /tmp/Работа.lbx"
        );
        assert_eq!(
            Language::English.message("network timeout: 42"),
            "network timeout: 42"
        );
        assert_eq!(
            Language::Russian.message("incorrect master password"),
            "неверный мастер-пароль"
        );
    }

    #[test]
    fn interpolation_preserves_user_content_and_braces() {
        assert_eq!(
            Language::English.format(
                Text::DeleteEntryThisCannotBeUndone,
                &["Работа {secret}".into()]
            ),
            "Delete entry ‘Работа {secret}’? This cannot be undone."
        );
        assert_eq!(Language::English.minutes_label(0), "never");
        assert_eq!(Language::English.minutes_label(5), "5 min");
        assert_eq!(Language::Russian.minutes_label(15), "15 мин");
    }
}
