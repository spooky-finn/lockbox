use crate::desktop::i18n::Text;
use crate::records::dialogs;
use crate::settings::actions;

use crate::{
    desktop::{
        context::{Session, edit_unlocked},
        i18n::use_language,
        profiles,
        session::Screen,
        widgets::{
            symbol::{Icon, Symbol},
            theme::{ResolvedTheme, mode_name, preset_style},
        },
        workspace::Page,
    },
    platform::autolock,
    records::editor::replace_secret,
    settings::{
        appearance::{ThemeMode, ThemePreset},
        language::Language,
        state::Settings,
        vault_icon::VaultIconSetting,
    },
};
use dioxus::prelude::*;
use zeroize::Zeroizing;

#[component]
fn LanguageSelect(id: &'static str) -> Element {
    let language = use_language();
    rsx! {
        select { id, aria_label: language.text(Text::InterfaceLanguage), value: language.code(),
            onchange: move |event| match event.value().as_str() {
                "en" => actions::set_language(Language::English),
                "ru" => actions::set_language(Language::Russian),
                _ => {},
            },
            option { value: "en", selected: language == Language::English, lang: "en", "English" }
            option { value: "ru", selected: language == Language::Russian, lang: "ru", "Русский" }
        }
    }
}

#[component]
pub(crate) fn SettingsSidebar(mut section: Signal<u8>) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    rsx! {
        aside { class: "sidebar settings-sidebar",
            button { class: "quiet settings-nav-back", r#type: "button", onclick: move |_| edit_unlocked(state, |u| u.page = Page::Vault),
                Symbol { icon: Icon::ArrowLeft } {language.text(Text::BackToVault)}
            }
            nav { aria_label: language.text(Text::Settings),
                for (index, label, icon) in [
                    (0, Text::General, Icon::Settings),
                    (1, Text::VaultSettings, Icon::Folder),
                    (2, Text::Appearance, Icon::Sparkles),
                    (3, Text::Security, Icon::Lock),
                    (4, Text::ImportVault, Icon::Import),
                ] {
                    button { class: "quiet settings-nav-item", r#type: "button", aria_current: if section() == index { "page" } else { "false" }, onclick: move |_| section.set(index),
                        Symbol { icon } {language.text(label)}
                    }
                }
            }
        }
    }
}

#[component]
pub(crate) fn SettingsView(section: Signal<u8>) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let settings = use_context::<Signal<Settings>>();
    let preferences = settings.read();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    rsx! {
        article { class: "settings-page stack", aria_label: language.text(Text::Settings),
            div { class: "detail-heading",
                div { class: "directory-icon", Symbol { icon: Icon::Settings } }
                div {
                    p { class: "detail-kind", "lockbox" }
                    h1 { {language.text(Text::Settings)} }
                }
            }
            p { class: "muted", {language.text(Text::SettingsAreSavedAutomaticallyAndAppliedImmediately)} }
            if section() == 0 { section { class: "settings-section stack",
                h2 { {language.text(Text::General)} }
                div { class: "preference-row",
                    label { r#for: "settings-language", {language.text(Text::InterfaceLanguage)} }
                    LanguageSelect { id: "settings-language" }
                }
                if let Some(profile) = unlocked.vault.profile_summary() {
                    UsernameSetting {
                        username: profile.username.to_owned(),
                        disabled: unlocked.saving_draft() || unlocked.vault.signing_error().is_some(),
                    }
                }
            }
            }
            if section() == 1 { VaultNames {} }
            if section() == 2 { AppearanceSection {} }
            if section() == 3 { section { class: "settings-section stack",
                h2 { {language.text(Text::Security)} }
                div { class: "preference-row",
                    div { class: "preference-copy",
                        label { r#for: "website-icons", {language.text(Text::WebsiteIcons)} }
                        p { id: "website-icons-description", class: "muted", {language.text(Text::WebsiteIconsDescription)} }
                    }
                    input { id: "website-icons", class: "switch", r#type: "checkbox",
                        checked: preferences.config.website_icons, aria_describedby: "website-icons-description",
                        onchange: move |event| actions::set_website_icons(event.checked()),
                    }
                }
                div { class: "preference-row",
                    div { class: "preference-copy",
                        label { r#for: "auto-lock", {language.text(Text::AutoLock)} }
                        p { id: "auto-lock-description", class: "muted", {language.text(Text::LockTheVaultAfterAPeriodOfInactivity)} }
                    }
                    select { id: "auto-lock", aria_label: language.text(Text::AutoLockTimeout),
                        aria_describedby: "auto-lock-description",
                        value: "{preferences.config.auto_lock_minutes}",
                        onchange: move |event| {
                            if let Ok(minutes) = event.value().parse() { actions::set_auto_lock(minutes); }
                        },
                        for minutes in autolock::CHOICES {
                            option { value: "{minutes}", selected: preferences.config.auto_lock_minutes == minutes,
                                {language.minutes_label(minutes)}
                            }
                        }
                    }
                }
                if unlocked.touch_id.available {
                    div { class: "preference-row",
                        div { class: "preference-copy",
                            label { r#for: "settings-touch-id", "Touch ID" }
                            p { id: "touch-id-description", class: "muted", {language.text(Text::UnlockThisVaultWithYourFingerprint)} }
                        }
                        input { id: "settings-touch-id", class: "switch", r#type: "checkbox", checked: unlocked.touch_id.enabled,
                            aria_describedby: "touch-id-description",
                            disabled: unlocked.touch_id.pending.is_some(),
                            onchange: move |event| actions::set_touch_id(event.checked()),
                        }
                    }
                } else {
                    p { class: "muted", {language.text(Text::TouchIDIsUnavailableOnThisDevice)} }
                }
            }
            }
            if section() == 4 { ImportSection {} }
        }
    }
}

#[component]
fn VaultNames() -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let settings = use_context::<Signal<Settings>>();
    let screen = state.read();
    let Screen::Unlocked(workspace) = &*screen else {
        return rsx! {};
    };

    let path = workspace.vault.path();
    let alias = settings
        .read()
        .config
        .vault_aliases
        .get(path)
        .cloned()
        .unwrap_or_default();
    let disabled = !screen.can_switch_profile();
    let profile = workspace.vault.profile_summary();
    let shared = profile.is_some();
    let editable = !shared || workspace.vault.can_edit_shared_metadata();
    rsx! {
        section { class: "settings-section stack",
            h2 { {language.text(Text::VaultSettings)} }
            VaultIconSetting { icon: workspace.vault.icon().cloned(), shared, editable, disabled }
            if let Some(profile) = profile {
                if editable {
                    VaultNameSetting { kind: VaultName::Shared, saved: profile.space_name.to_owned(), disabled }
                }
            } else {
                VaultNameSetting {
                    kind: VaultName::File,
                    saved: path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                    disabled,
                }
            }
            VaultNameSetting { kind: VaultName::Personal, saved: alias, disabled }
            if workspace.vault.profile_summary().is_none() {
                DeleteLocalVault {
                    key: "{workspace.vault.id()}:{path.display()}",
                    path: path.to_path_buf(),
                    disabled: disabled || workspace.touch_id.pending.is_some(),
                }
            }
        }
    }
}

#[component]
fn DeleteLocalVault(path: std::path::PathBuf, disabled: bool) -> Element {
    let language = use_language();
    let mut confirming = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let prompt = language.format(Text::ConfirmDeleteLocalVault, &[path.to_string_lossy()]);
    rsx! {
        button { class: "quiet danger vault-delete-trigger", r#type: "button", disabled,
            onclick: move |_| { error.set(None); confirming.set(true); },
            Symbol { icon: Icon::Trash }
            {language.text(Text::DeleteLocalVault)}
        }
        if confirming() {
            dialog { id: "vault-delete-dialog", class: "entry-trash-dialog",
                aria_labelledby: "vault-delete-prompt",
                onmounted: move |_| {
                    let _ = document::eval("document.getElementById('vault-delete-dialog')?.showModal();");
                },
                onkeydown: move |event| {
                    if event.key() == Key::Escape { event.stop_propagation(); }
                },
                oncancel: move |event| {
                    event.prevent_default();
                    confirming.set(false);
                },
                p { id: "vault-delete-prompt", "{prompt}" }
                div { class: "actions",
                    button { r#type: "button", autofocus: true,
                        onclick: move |_| confirming.set(false), {language.text(Text::Cancel)}
                    }
                    button { class: "danger", r#type: "button", disabled,
                        onclick: move |_| {
                            confirming.set(false);
                            if let Err(message) = profiles::delete_local_vault(&path) {
                                error.set(Some(message));
                            }
                        },
                        {language.text(Text::Delete)}
                    }
                }
            }
        }
        if let Some(message) = error() {
            p { class: "error", role: "alert", "{message}" }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum VaultName {
    File,
    Shared,
    Personal,
}

#[component]
fn VaultNameSetting(kind: VaultName, saved: String, disabled: bool) -> Element {
    let language = use_language();
    let mut draft = use_signal(|| None::<String>);
    let mut error = use_signal(|| None::<String>);
    let name = draft().unwrap_or_else(|| saved.clone());
    let personal = kind == VaultName::Personal;
    let (id, label) = match kind {
        VaultName::File => ("settings-vault-filename", Text::VaultFilename),
        VaultName::Shared => ("settings-shared-vault-name", Text::SharedVaultName),
        VaultName::Personal => ("settings-personal-vault-name", Text::PersonalVaultName),
    };
    let can_save = !disabled && name.trim() != saved && (personal || !name.trim().is_empty());
    rsx! {
        form { class: "stack",
            onsubmit: move |event: FormEvent| {
                event.prevent_default();
                if can_save && let Some(name) = draft() {
                    let result = match kind {
                        VaultName::File => profiles::rename_local_vault(&name),
                        VaultName::Shared => profiles::rename_shared_vault(&name),
                        VaultName::Personal => profiles::rename_vault_for_me(&name),
                    };
                    match result {
                        Ok(()) => { draft.set(None); error.set(None); }
                        Err(message) => error.set(Some(message)),
                    }
                }
            },
            label { r#for: id, {language.text(label)} }
            if personal {
                p { id: "settings-personal-vault-name-help", class: "muted", {language.text(Text::PersonalVaultNameHelp)} }
            }
            div { class: "inline vault-name-field",
                input { id, value: "{name}", disabled, autocomplete: "off",
                    aria_describedby: personal.then_some("settings-personal-vault-name-help"),
                    oninput: move |event| { draft.set(Some(event.value())); error.set(None); },
                }
                button { r#type: "submit", disabled: !can_save, {language.text(Text::Save)} }
            }
            if let Some(message) = error() {
                p { class: "error", role: "alert", "{message}" }
            }
        }
    }
}

#[component]
fn UsernameSetting(username: String, disabled: bool) -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let mut draft = use_signal(|| None::<String>);
    let name = draft().unwrap_or_else(|| username.clone());
    rsx! {
        form { class: "preference-row",
            onsubmit: move |event: FormEvent| {
                event.prevent_default();
                if !disabled && let Some(name) = draft() {
                    edit_unlocked(state, |workspace| {
                        workspace.error = workspace.vault.set_username(&name).err().map(|error| error.to_string());
                        if workspace.error.is_none() {
                            workspace.sync.saved();
                            draft.set(None);
                        }
                    });
                }
            },
            label { r#for: "settings-username", {language.text(Text::RenameMyName)} }
            input { id: "settings-username", value: "{name}", disabled, autocomplete: "off",
                oninput: move |event| draft.set(Some(event.value())),
            }
            button { r#type: "submit", disabled: disabled || name.trim().is_empty() || name.trim() == username,
                {language.text(Text::Save)}
            }
        }
    }
}

#[component]
fn AppearanceSection() -> Element {
    let language = use_language();
    let settings = use_context::<Signal<Settings>>();
    let state = use_context::<Session>();
    let screen = state.read();
    let config = &settings.read().config;
    let appearance = config.appearance_for(screen.vault_id().as_deref());
    let dark = try_use_context::<ResolvedTheme>()
        .map_or_else(|| appearance.theme.is_dark(false), ResolvedTheme::is_dark);
    rsx! {
        section { class: "settings-section stack",
            h2 { id: "settings-appearance", {language.text(Text::Appearance)} }
            p { class: "muted", {language.text(Text::VaultAppearanceScope)} }
            div { class: "preference-row",
                label { r#for: "settings-theme", {language.text(Text::ColorTheme)} }
                select { id: "settings-theme", value: appearance.theme.code(),
                    onchange: move |event| match event.value().as_str() {
                        "system" => actions::set_theme(ThemeMode::System),
                        "light" => actions::set_theme(ThemeMode::Light),
                        "dark" => actions::set_theme(ThemeMode::Dark),
                        _ => {},
                    },
                    for (theme, label) in [
                        (ThemeMode::System, Text::FollowSystem),
                        (ThemeMode::Light, Text::LightTheme),
                        (ThemeMode::Dark, Text::DarkTheme),
                    ] {
                        option { value: theme.code(), selected: appearance.theme == theme, {language.text(label)} }
                    }
                }
            }
            div { class: "preference-row",
                label { r#for: "settings-border-radius", {language.text(Text::BorderRadius)} }
                select { id: "settings-border-radius", value: match config.appearance.border_radius.unwrap_or(6) { 0..=3 => "2", 4..=9 => "6", _ => "12" },
                    onchange: move |event| {
                        if let Ok(radius @ (2 | 6 | 12)) = event.value().parse::<u8>() {
                            actions::set_border_radius(Some(radius));
                        }
                    },
                    option { value: "2", {language.text(Text::CornersSquare)} }
                    option { value: "6", {language.text(Text::CornersSemiRounded)} }
                    option { value: "12", {language.text(Text::CornersRounded)} }
                }
            }

            div { id: "settings-presets", class: "theme-grid", role: "group", aria_labelledby: "settings-appearance",
                for preset in ThemePreset::ALL {
                    ThemeCard { key: "{preset.code()}", preset, dark, selected: appearance.preset == preset }
                }
            }
        }
    }
}

#[component]
fn ThemeCard(preset: ThemePreset, dark: bool, selected: bool) -> Element {
    let language = use_language();
    let name = match preset {
        ThemePreset::Classic => Text::ThemeTypewriter,
        ThemePreset::Paper => Text::ThemeNotebook,
        ThemePreset::Forest => Text::ThemeFieldNotes,
        ThemePreset::Ocean => Text::ThemeBlueprint,
        ThemePreset::Slate => Text::ThemeGraphPaper,
        ThemePreset::Amber => Text::ThemeKraft,
        ThemePreset::Rose => Text::ThemeRedMargin,
        ThemePreset::Teal => Text::ThemeIndexCards,
        ThemePreset::Plum => Text::ThemeInk,
        ThemePreset::Indigo => Text::ThemeTerminal,
    };
    let sample = if preset == ThemePreset::Indigo {
        ">_"
    } else {
        "Aa"
    };
    rsx! {
        button {
            class: "theme-card",
            r#type: "button",
            aria_pressed: selected,
            "data-theme-choice": preset.code(),
            onclick: move |_| actions::set_theme_preset(preset),
            span {
                class: "theme-preview",
                "data-preset": preset.code(),
                "data-theme": mode_name(dark),
                style: preset_style(preset, dark),
                aria_hidden: true,
                span { class: "theme-sample", "{sample}" }
            }
            span { class: "theme-card-caption",
                span { {language.text(name)} }
                if selected {
                    span { class: "theme-card-check", aria_hidden: true, Symbol { icon: Icon::Check } }
                }
            }
        }
    }
}

/// Copies another vault file into this one; see `records::import`.
#[component]
fn ImportSection() -> Element {
    let language = use_language();
    let state = use_context::<Session>();
    let screen = state.read();
    let Screen::Unlocked(unlocked) = &*screen else {
        return rsx! {};
    };
    let import = &unlocked.import;
    let busy = import.busy();
    let ready = !busy && import.path.is_some() && !import.password.is_empty();
    let file_name = import.file_name();
    rsx! {
        section { class: "settings-section stack", aria_label: language.text(Text::ImportVault),
            h2 { {language.text(Text::ImportVault)} }
            p { class: "muted", {language.text(Text::ImportDescription)} }
            div { class: "preference-row",
                span { class: "import-file",
                    match &file_name {
                        Some(name) => rsx! { "{name}" },
                        None => rsx! { {language.text(Text::NoFileChosen)} },
                    }
                }
                button { r#type: "button", disabled: busy,
                    onclick: move |_| dialogs::choose_import_file(),
                    {language.text(Text::ChooseVaultFile)}
                }
            }
            if file_name.is_some() {
                form { class: "stack",
                    onsubmit: move |event: FormEvent| {
                        event.prevent_default();
                        edit_unlocked(state, |u| u.start_import());
                    },
                    label { class: "sr-only", r#for: "import-password", {language.text(Text::ImportPassword)} }
                    input { id: "import-password", r#type: "password", autocomplete: "off",
                        placeholder: language.text(Text::ImportPassword),
                        disabled: busy, value: import.password.as_str(),
                        oninput: move |event| {
                            let value = Zeroizing::new(event.value());
                            edit_unlocked(state, |u| replace_secret(&mut u.import.password, &value));
                        },
                    }
                    button { class: "primary", r#type: "submit", disabled: !ready,
                        if busy { {language.text(Text::ImportBusy)} } else { {language.text(Text::ImportStart)} }
                    }
                }
            }
            if busy && unlocked.is_editing() {
                p { class: "notice", {language.text(Text::ImportAfterEditing)} }
            }
            if let Some(notice) = &import.notice {
                p { class: "notice", role: "status", {language.message(notice)} }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/view.rs"]
mod tests;
