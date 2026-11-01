//! Download website-declared icons without blocking the UI.
use base64::{Engine, engine::general_purpose::STANDARD};
use kuchikiki::traits::TendrilSink;
use std::{
    collections::HashMap,
    io::Read,
    sync::{Mutex, OnceLock},
    time::Duration,
};

fn icon_links(html: &str, page: &url::Url) -> Vec<String> {
    let document = kuchikiki::parse_html().one(html).document_node;
    let base = document
        .select_first("base[href]")
        .ok()
        .and_then(|node| page.join(node.attributes.borrow().get("href")?).ok())
        .unwrap_or_else(|| page.clone());
    let mut icons = Vec::new();
    if let Ok(links) = document.select("link[href][rel]") {
        for link in links {
            let attrs = link.attributes.borrow();
            let rel = attrs.get("rel").unwrap_or_default();
            if !rel.split_ascii_whitespace().any(|value| {
                value.eq_ignore_ascii_case("icon") || value.eq_ignore_ascii_case("apple-touch-icon")
            }) {
                continue;
            }
            if let Ok(mut icon) = base.join(attrs.get("href").unwrap_or_default()) {
                if !matches!(icon.scheme(), "http" | "https") {
                    continue;
                }
                let _ = icon.set_username("");
                let _ = icon.set_password(None);
                icons.push(icon.to_string());
            }
        }
    }
    icons
}

pub(super) fn fetch(fallback: &str) -> Option<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(image) = cache.lock().ok()?.get(fallback).cloned() {
        return image;
    }
    let result = download(fallback);
    let mut cache = cache.lock().ok()?;
    if cache.len() >= 256 {
        cache.clear();
    }
    cache.insert(fallback.to_owned(), result.clone());
    result
}

fn download(fallback: &str) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .ok()?;
    let home = url::Url::parse(fallback).ok()?.join("/").ok()?;
    let mut candidates = Vec::new();
    if let Ok(response) = client.get(home).send().and_then(|r| r.error_for_status()) {
        let page = response.url().clone();
        let mut body = String::new();
        if response.take(512 * 1024).read_to_string(&mut body).is_ok() {
            candidates = icon_links(&body, &page);
        }
    }
    candidates.push(fallback.to_owned());
    candidates.dedup();
    for candidate in candidates.into_iter().take(8) {
        let Ok(response) = client
            .get(candidate)
            .send()
            .and_then(|r| r.error_for_status())
        else {
            continue;
        };
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !mime.starts_with("image/") {
            continue;
        }
        let mut bytes = Vec::new();
        if response
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.is_empty()
            || bytes.len() > 1024 * 1024
        {
            continue;
        }
        return Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovers_real_icons_and_resolves_base_and_entities() {
        let page = url::Url::parse("https://example.com/login/").unwrap();
        assert_eq!(
            icon_links(
                r#"<base href='/assets/'><link rel='stylesheet' href='a.css'><link REL='shortcut ICON' href='brand.png?v=1&amp;s=2'><link rel='apple-touch-icon' href='//cdn.example.com/apple.png'><link rel='icon' href='javascript:bad'>"#,
                &page
            ),
            vec![
                "https://example.com/assets/brand.png?v=1&s=2",
                "https://cdn.example.com/apple.png"
            ]
        );
    }
    #[test]
    fn downloads_declared_image_instead_of_guessed_favicon() {
        use std::io::Write;
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for (path, content_type, body) in [
                ("/", "text/html", "<link rel=icon href=/brand.png>"),
                ("/brand.png", "image/png", "actual-image-bytes"),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let length = stream.read(&mut request).unwrap();
                assert!(
                    String::from_utf8_lossy(&request[..length])
                        .starts_with(&format!("GET {path} HTTP/1.1"))
                );
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        assert_eq!(
            download(&format!("http://{address}/favicon.ico")),
            Some(format!(
                "data:image/png;base64,{}",
                STANDARD.encode("actual-image-bytes")
            ))
        );
        server.join().unwrap();
    }

    #[test]
    fn resolves_relative_icons_after_redirects() {
        let page = url::Url::parse("https://www.example.com/site/").unwrap();
        assert_eq!(
            icon_links("<link rel=icon href=logo.ico>", &page),
            vec!["https://www.example.com/site/logo.ico"]
        );
        assert!(icon_links("<html>No icon</html>", &page).is_empty());
    }
}
