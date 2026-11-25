//! Advisory update checks, independent of vault state and storage.

use crate::desktop::i18n::{Text, use_language};
use dioxus::prelude::*;
use semver::Version;
use serde::Deserialize;
use std::time::Duration;

const LATEST_RELEASE_API: &str = "https://api.github.com/repos/spooky-finn/lockbox/releases/latest";
pub(crate) const RELEASES_PAGE: &str = "https://github.com/spooky-finn/lockbox/releases";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

fn newer_version(installed: &str, tag: &str) -> Option<Version> {
    let installed = Version::parse(installed).ok()?;
    let latest = Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()?;
    (latest.pre.is_empty() && latest.cmp_precedence(&installed).is_gt()).then_some(latest)
}

async fn check() -> Option<Version> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("lockbox-release-check")
        .build()
        .ok()?;
    let mut response = client
        .get(LATEST_RELEASE_API)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if chunk.len() > MAX_RESPONSE_BYTES - body.len() {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    let release: Release = serde_json::from_slice(&body).ok()?;
    newer_version(env!("CARGO_PKG_VERSION"), &release.tag_name)
}

#[component]
pub(super) fn UpdateNotice(locked: bool) -> Element {
    let language = use_language();
    let mut checked = use_signal(|| false);
    let mut available = use_signal(|| None::<Version>);
    use_effect(use_reactive!(|locked| {
        if !locked && !*checked.peek() {
            checked.set(true);
            spawn(async move { available.set(check().await) });
        }
    }));

    if locked {
        return rsx! {};
    }
    let version = available.read();
    let Some(version) = version.as_ref() else {
        return rsx! {};
    };
    rsx! {
        div { class: "actions update-notice",
            span { role: "status", {language.format(Text::UpdateAvailable, &[version.to_string()])} }
            a { href: RELEASES_PAGE, rel: "noopener noreferrer", {language.text(Text::ViewGitHubReleases)} }
            button { class: "quiet", r#type: "button",
                onclick: move |_| available.set(None),
                {language.text(Text::DismissUpdateNotice)}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::newer_version;

    #[test]
    fn compares_stable_versions_by_semver_precedence() {
        for tag in ["v0.10.0", "1.0.0"] {
            assert!(newer_version("0.9.0", tag).is_some(), "{tag}");
        }
        for tag in ["v0.8.9", "v0.9.0", "v0.9.0+build", "v1.0.0-rc.1", "latest"] {
            assert!(newer_version("0.9.0", tag).is_none(), "{tag}");
        }
        assert!(newer_version("0.9.0-rc.1", "v0.9.0").is_some());
    }
}
