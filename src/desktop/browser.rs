//! Browser consent lives alongside the normal desktop unlock flow.
use crate::desktop::{context::Session, session::Screen};
use dioxus::prelude::*;

#[cfg(unix)]
use crate::browser::bridge::{Credential, DesktopRequest};
#[cfg(unix)]
static REQUESTS: std::sync::Mutex<std::collections::VecDeque<DesktopRequest>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());
#[cfg(unix)]
static CONNECTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn receive(link: &str) {
    #[cfg(unix)]
    {
        use std::sync::atomic::Ordering;
        if !link.starts_with("lockbox://browser/") || CONNECTING.swap(true, Ordering::SeqCst) {
            return;
        }
        let link = link.to_owned();
        std::thread::spawn(move || {
            if let Ok(request) = DesktopRequest::connect(&link) {
                let mut requests = REQUESTS.lock().unwrap();
                requests.retain(DesktopRequest::active);
                if requests.is_empty() {
                    requests.push_back(request);
                } else {
                    let _ = request.respond(None);
                }
            }
            CONNECTING.store(false, Ordering::SeqCst);
        });
    }
    #[cfg(not(unix))]
    let _ = link;
}

#[component]
pub(crate) fn BrowserConsent() -> Element {
    #[cfg(not(unix))]
    {
        return rsx! {};
    }
    #[cfg(unix)]
    {
        let state = use_context::<Session>();
        let mut pending = use_signal(|| None::<DesktopRequest>);
        // One window claims each request. The selected window is brought forward.
        use_future(move || async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                if pending
                    .peek()
                    .as_ref()
                    .is_some_and(|request| !request.active())
                {
                    pending.set(None);
                }
                if pending.peek().is_none() {
                    let request = REQUESTS.lock().unwrap().pop_front();
                    if let Some(request) = request.filter(DesktopRequest::active) {
                        pending.set(Some(request));
                        let window = dioxus::desktop::window();
                        window.set_visible(true);
                        window.set_minimized(false);
                        window.set_focus();
                    }
                }
            }
        });
        let read = pending.read();
        let Some(request) = read.as_ref() else {
            return rsx! {};
        };
        let origin = request.origin.clone();
        let deadline = request.deadline;
        let session = state.read();
        let entries: Vec<_> = match &*session {
            Screen::Unlocked(unlocked) => unlocked
                .vault
                .entries()
                .iter()
                .filter(|entry| {
                    crate::browser::origin(&entry.url)
                        .is_ok_and(|actual| actual.ascii_serialization() == origin)
                })
                .map(|entry| {
                    (
                        entry.id.clone(),
                        entry.title.clone(),
                        entry.username.clone(),
                    )
                })
                .collect(),
            _ => Vec::new(),
        };
        let locked = matches!(&*session, Screen::Locked(_));
        rsx! {
            section { class: "browser-consent", style: "position:fixed;top:36px;right:20px;width:min(420px,90vw);max-height:45vh;overflow:auto;z-index:10000;padding:18px;border:1px solid #718368;border-radius:12px;background:var(--panel, #242824);color:var(--text, #f4f4ed);box-shadow:0 8px 32px #0006", role: "region", aria_label: "Browser autofill request",
                strong { "Browser autofill" }
                p { "Requested website: " code { "{origin}" } }
                if locked {
                    p { "Unlock Lockbox below to choose an account. Your master password stays in this app." }
                } else if entries.is_empty() {
                    p { "No accounts match this exact website in the current vault." }
                } else {
                    p { "Choose an account to send its login and password to this website." }
                    for (id, title, username) in entries {
                        { let origin = origin.clone(); rsx! {
                        button {
                            onclick: move |_| {
                                // Reject a click left over from a previous consent render.
                                if !pending.peek().as_ref().is_some_and(|request|
                                    request.deadline == deadline && request.origin == origin && request.active()) { return; }
                                // Recheck the current vault and origin at the moment of consent.
                                let credential = {
                                    let current = state.read();
                                    match &*current {
                                        Screen::Unlocked(unlocked) => unlocked.vault.entries().iter()
                                            .find(|entry| entry.id == id && crate::browser::origin(&entry.url).is_ok_and(|actual| actual.ascii_serialization() == origin)
                                                && entry.username.len() <= 4096 && entry.password.len() <= 4096)
                                            .map(|entry| Credential { username: zeroize::Zeroizing::new(entry.username.clone()), password: zeroize::Zeroizing::new(entry.password.clone()) }),
                                        _ => None,
                                    }
                                };
                                if let Some(credential) = credential {
                                    if let Some(request) = pending.write().take() {
                                        std::thread::spawn(move || { let _ = request.respond(Some(credential)); });
                                    }
                                }
                            },
                            "Fill {title} — {username}"
                        } } }
                    }
                }
                button { onclick: move |_| {
                    if let Some(request) = pending.write().take() {
                        std::thread::spawn(move || { let _ = request.respond(None); });
                    }
                }, "Cancel browser request" }
            }
        }
    }
}
