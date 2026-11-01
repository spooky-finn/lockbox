use crate::desktop::icons::*;
use std::collections::HashSet;

fn slug(url: &str) -> Option<&'static str> {
    catalog().for_url(url).map(|icon| icon.slug.as_str())
}

#[test]
fn whole_set_is_well_formed() {
    let catalog = catalog();
    assert!(catalog.len() > 3000);
    let mut slugs = HashSet::new();
    let mut domains = HashSet::new();
    for icon in &catalog.icons {
        assert!(slugs.insert(&icon.slug), "duplicate {}", icon.slug);
        assert!(
            icon.hex.len() == 6 && icon.hex.chars().all(|c| c.is_ascii_hexdigit()),
            "{}",
            icon.slug
        );
        for domain in &icon.domains {
            assert!(domains.insert(domain), "{domain} belongs to two icons");
        }
    }
}

#[test]
fn every_icon_renders() {
    for icon in &catalog().icons {
        let pixels = render(icon, 16, false).unwrap_or_else(|| panic!("{}", icon.slug));
        assert!(
            pixels.iter().any(|&byte| byte != 0),
            "{} is blank",
            icon.slug
        );
    }
}

#[test]
fn urls_find_their_brand() {
    for (url, expected) in [
        ("https://gitlab.com/users/sign_in", "gitlab"),
        ("https://www.hetzner.com", "hetzner"),
        ("accounts.google.com", "google"),
        ("https://mail.google.com/mail/u/0", "gmail"),
        ("t.me/somebody", "telegram"),
        ("https://e.mail.ru/inbox", "maildotru"),
        ("steamcommunity.com", "steam"),
        ("http://user@GitHub.com:443/x?y#z", "github"),
        ("app.hackthebox.com", "hackthebox"),
        ("https://twitter.com/home", "x"),
    ] {
        assert_eq!(slug(url), Some(expected), "{url}");
    }
}

#[test]
fn unknown_or_empty_urls_have_no_icon() {
    for url in [
        "",
        "intranet",
        "https://my-own-server.example/login",
        "localhost:8080",
    ] {
        assert_eq!(slug(url), None, "{url}");
    }
}

#[test]
fn two_level_suffixes_keep_the_site_name() {
    assert_eq!(registrable("shop.example.co.uk"), "example.co.uk");
    assert_eq!(registrable("a.b.example.com"), "example.com");
}

#[test]
fn site_suffixes_preserve_short_unusual_and_case_sensitive_inputs() {
    for (input, expected) in [
        ("", ""),
        ("local", "local"),
        ("co.uk", "co.uk"),
        ("a.b.example.com.au", "example.com.au"),
        ("a.b.CO.UK", "CO.UK"),
        ("shop.пример.рф", "пример.рф"),
        ("a..com", ".com"),
        ("a...co.uk", ".co.uk"),
        ("co.uk.", "uk."),
    ] {
        assert_eq!(registrable(input), expected, "{input}");
    }
    for (input, expected) in [
        (
            " HTTPS://user:secret@WWW.Example.COM.:443/a?b#c ",
            "example.com",
        ),
        ("WWW.WWW.Example.com", "www.example.com"),
        ("пример.рф", "пример.рф"),
    ] {
        assert_eq!(host(input).as_deref(), Some(expected), "{input}");
    }
    assert!(host("www.").is_none());
    assert!(host(".example.com").is_none());
}

#[test]
fn entry_choice_beats_url_and_title_is_the_last_resort() {
    let mut entry = Entry::new();
    entry.title = "GitHub".into();
    assert_eq!(catalog().for_entry(&entry).unwrap().slug, "github");
    entry.url = "https://gitlab.com".into();
    assert_eq!(catalog().for_entry(&entry).unwrap().slug, "gitlab");
    entry.icon = Some("telegram".into());
    assert_eq!(catalog().for_entry(&entry).unwrap().slug, "telegram");
}

#[test]
fn search_puts_prefix_matches_first() {
    let found = catalog().search("git", 5);
    assert!(found.iter().all(|icon| icon.slug.contains("git")));
    assert_eq!(found[0].slug, "git");
    assert!(catalog().search("   ", 5).is_empty());
}

#[test]
fn search_preserves_title_matches_and_stable_ranking_after_normalization() {
    let catalog = Catalog::parse(
        r#"{"slug":"agitaa","title":"Other","hex":"000000","path":""}
{"slug":"gitx","title":"Other","hex":"000000","path":""}
{"slug":"zzzz","title":"GİT Toolbox","hex":"000000","path":""}
{"slug":"agityy","title":"Other","hex":"000000","path":""}
{"slug":"git","title":"Other","hex":"000000","path":""}
{"slug":"gitq","title":"Other","hex":"000000","path":""}"#,
    );
    let slugs = |query, limit| {
        catalog
            .search(query, limit)
            .into_iter()
            .map(|icon| icon.slug.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        slugs(" G.İ-T! ", 10),
        ["git", "gitx", "gitq", "zzzz", "agitaa", "agityy"]
    );
    assert_eq!(slugs("git", 3), ["git", "gitx", "gitq"]);
    assert!(slugs("git", 0).is_empty());
    assert!(slugs("!!!", 10).is_empty());
}

#[test]
fn letter_tiles_are_stable() {
    assert_eq!(letter("  сбер"), "С");
    assert_eq!(letter(""), "?");
    assert_eq!(letter_colour("Сбер"), letter_colour("сбер"));
}

#[test]
fn glyph_contrast_follows_the_background() {
    assert!(glyph_is_dark(rgb("F0B90B")));
    assert!(!glyph_is_dark(rgb("181717")));
}

#[test]
fn favicon_uses_only_http_origin() {
    assert_eq!(
        favicon_url("https://user:secret@example.com:8443/login?q=secret#token"),
        Some("https://example.com:8443/favicon.ico".into())
    );
    assert_eq!(
        favicon_url("example.com/login"),
        Some("https://example.com/favicon.ico".into())
    );
    assert_eq!(
        favicon_url("http://localhost:8080/login"),
        Some("http://localhost:8080/favicon.ico".into())
    );
    for input in [
        "",
        "file:///tmp/icon",
        "javascript://example.com/test",
        "https://",
    ] {
        assert_eq!(favicon_url(input), None);
    }
}
