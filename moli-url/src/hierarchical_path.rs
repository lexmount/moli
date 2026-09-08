use url::Url;

fn assert_url(url: &Url, expected: &str, path: &str) {
    assert_eq!(url.as_str(), expected);
    assert_eq!(url.path(), path);
    url.check_invariants().unwrap();
    assert_eq!(Url::parse(url.as_str()).unwrap(), *url);
}

#[test]
fn relative_special_urls_skip_all_authority_slashes() {
    for scheme in ["http", "https", "ws", "wss", "ftp"] {
        let base = Url::parse(&format!("{scheme}://user:pass@old.test:123/a/b?q#f")).unwrap();
        for prefix in ["//", "///", r"///\//\//", r"\\", r"/\", r"\/", "/\t/\n/"] {
            let url = base
                .join(&format!("{prefix}new.test/a/../p?next#frag"))
                .unwrap();
            assert_url(&url, &format!("{scheme}://new.test/p?next#frag"), "/p");
            assert_eq!(url.host_str(), Some("new.test"));
            assert_eq!(url.port(), None);
            assert!(url.username().is_empty());
            assert_eq!(url.password(), None);
        }
        for input in ["///", r"/\\", "////?query", r"\\#fragment"] {
            assert!(base.join(input).is_err(), "{scheme}: {input:?}");
        }
        for input in ["/p", r"\p"] {
            assert_url(
                &base.join(input).unwrap(),
                &format!("{scheme}://user:pass@old.test:123/p"),
                "/p",
            );
        }
    }
}

#[test]
fn relative_non_special_urls_keep_backslashes_in_the_path() {
    for (base, prefix) in [
        ("custom://host/a/b?q#f", "custom://host/a/"),
        ("custom:/a/b?q#f", "custom:/a/"),
        ("custom:/.//a/b?q#f", "custom:/.//a/"),
    ] {
        let base = Url::parse(base).unwrap();
        for input in [r"\p", r"\/p", r"\\p"] {
            let expected = format!("{prefix}{input}");
            let url = base.join(input).unwrap();
            assert_eq!(url.as_str(), expected);
            url.check_invariants().unwrap();
            assert_eq!(url.has_authority(), base.has_authority());
            assert_eq!(url.host_str(), base.host_str());
        }
        for (input, expected, path) in [
            ("//next", "custom://next", ""),
            ("///next", "custom:///next", "/next"),
            ("////next", "custom:////next", "//next"),
        ] {
            assert_url(&base.join(input).unwrap(), expected, path);
        }
        assert!(base.join(r"//\next").is_err());
    }
}

#[test]
fn hierarchical_path_setters_add_and_remove_only_the_serialization_prefix() {
    for input in ["custom:/old?q#f", "custom:/.//old?q#f"] {
        for (value, expected, path) in [
            ("//p", "custom:/.//p?q#f", "//p"),
            ("/.//p", "custom:/.//p?q#f", "//p"),
            ("/..//p", "custom:/.//p?q#f", "//p"),
            ("//", "custom:/.//?q#f", "//"),
            ("p", "custom:/p?q#f", "/p"),
            ("/", "custom:/?q#f", "/"),
        ] {
            let mut url = Url::parse(input).unwrap();
            url.set_path(value);
            assert_url(&url, expected, path);
            assert!(!url.has_authority());
            assert_eq!(url.query(), Some("q"));
            assert_eq!(url.fragment(), Some("f"));
        }
        let mut url = Url::parse(input).unwrap();
        crate::components::set_pathname(&mut url, "");
        assert_url(&url, "custom:/?q#f", "/");
    }
    for authority in ["", "host", "user:pass@host:123"] {
        let mut url = Url::parse(&format!("custom://{authority}/old?q#f")).unwrap();
        url.set_path("//p");
        assert_url(&url, &format!("custom://{authority}//p?q#f"), "//p");
        assert!(url.has_authority());
    }
}

#[test]
fn adding_a_host_drops_the_path_serialization_prefix_without_changing_the_path() {
    for input in ["custom:/.//p?q#f", "custom:/.//?q#f"] {
        for host in ["h", "", "[::1]"] {
            let mut url = Url::parse(input).unwrap();
            let path = url.path().to_owned();
            crate::components::set_hostname(&mut url, host);
            assert_url(&url, &format!("custom://{host}{path}?q#f"), &path);
            assert!(url.has_authority());
            assert_eq!(url.query(), Some("q"));
            assert_eq!(url.fragment(), Some("f"));
        }
        let mut url = Url::parse(input).unwrap();
        let path = url.path().to_owned();
        crate::components::set_host(&mut url, "h:77");
        assert_url(&url, &format!("custom://h:77{path}?q#f"), &path);
        assert_eq!(url.port(), Some(77));
    }
}

#[test]
fn removing_a_host_keeps_leading_empty_segments_out_of_the_authority() {
    for authority in ["h", "", "[::1]", "user:pass@h:77"] {
        for path in ["", "/p", "//p", "///"] {
            let mut url = Url::parse(&format!("custom://{authority}{path}?q#f")).unwrap();
            url.set_host(None).unwrap();
            let path = if path.is_empty() { "/" } else { path };
            let prefix = if path.starts_with("//") { "/." } else { "" };
            assert_url(&url, &format!("custom:{prefix}{path}?q#f"), path);
            assert!(!url.has_authority());
            assert_eq!(url.host_str(), None);
            assert_eq!(url.port(), None);
            assert_eq!(url.query(), Some("q"));
            assert_eq!(url.fragment(), Some("f"));
            url.set_host(Some("next")).unwrap();
            assert_url(&url, &format!("custom://next{path}?q#f"), path);
        }
    }
}

#[test]
fn empty_pathname_distinguishes_empty_hosts_from_missing_hosts() {
    for (input, expected, path) in [
        ("custom:///p?q#f", "custom://?q#f", ""),
        ("custom://host/p?q#f", "custom://host?q#f", ""),
        ("custom:/p?q#f", "custom:/?q#f", "/"),
        ("custom:/.//p?q#f", "custom:/?q#f", "/"),
        ("https://host/p?q#f", "https://host/?q#f", "/"),
        ("file:///p?q#f", "file:///?q#f", "/"),
        ("data:payload?q#f", "data:payload?q#f", "payload"),
    ] {
        let mut url = Url::parse(input).unwrap();
        url::quirks::set_pathname(&mut url, "");
        assert_url(&url, expected, path);
    }
}

#[test]
fn path_segment_mutations_keep_the_path_prefix_and_component_offsets_consistent() {
    let mut url = Url::parse("custom:/.//p?q#f").unwrap();
    url.path_segments_mut().unwrap().pop();
    assert_url(&url, "custom:/?q#f", "/");
    url.set_path("//p");
    url.path_segments_mut()
        .unwrap()
        .clear()
        .push("next")
        .push("");
    assert_url(&url, "custom:/next/?q#f", "/next/");
    url.set_path("///");
    url.path_segments_mut().unwrap().pop_if_empty();
    assert_url(&url, "custom:/.//?q#f", "//");
    url.path_segments_mut().unwrap().pop_if_empty();
    assert_url(&url, "custom:/?q#f", "/");
}
