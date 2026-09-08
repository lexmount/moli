use url::Url;

fn assert_file_url(input: &str, base: Option<&str>, expected: &str) {
    let base = base.map(|value| Url::parse(value).unwrap());
    let url = Url::options().base_url(base.as_ref()).parse(input).unwrap();
    assert_eq!(url.as_str(), expected, "input {input:?}, base {base:?}");
    assert_eq!(
        Url::parse(url.as_str()).unwrap(),
        url,
        "serialization must round-trip"
    );
    assert!(url.username().is_empty());
    assert_eq!(url.password(), None);
    assert_eq!(url.port(), None);
}

#[test]
fn file_urls_preserve_leading_empty_path_segments() {
    for (input, expected) in [
        ("file://server///", "file://server///"),
        ("file:////one/two", "file:////one/two"),
        ("file://localhost////foo", "file://////foo"),
        (r"file:\\\\", "file:////"),
        ("file://localhost//a//../..//foo", "file://///foo"),
        ("file:.//p", "file:////p"),
        ("file:/.//p", "file:////p"),
        ("file:////?q#f", "file:////?q#f"),
    ] {
        assert_file_url(input, None, expected);
    }
    for (input, base, expected) in [
        ("/////mouse", "file:///elephant", "file://///mouse"),
        (r"\/localhost//pig", "file://lion/", "file:////pig"),
        (
            "/..//localhost//pig",
            "file://lion/",
            "file://lion//localhost//pig",
        ),
        ("file:///.//", "file:////", "file:////"),
    ] {
        assert_file_url(input, Some(base), expected);
    }
}

#[test]
fn file_urls_normalize_drive_letters_without_removing_hosts() {
    for host in ["", "server", "127.0.0.1", "[::1]"] {
        for path in ["C:/", "C|/", "C|", "C|?q#f", "C:/dir/file"] {
            let input = format!("file://{host}/{path}");
            let expected = format!("file://{host}/{}", path.replacen('|', ":", 1));
            assert_file_url(&input, None, &expected);
        }
    }
    assert_file_url("file:///w|/m", None, "file:///w:/m");
    assert_file_url("file:///dir/C|/file", None, "file:///dir/C|/file");
    assert_file_url("file:////C|/file", None, "file:////C|/file");
}

#[test]
fn file_url_relative_paths_inherit_hosts_and_only_preserve_root_drive_letters() {
    for (input, base, expected) in [
        ("/", "file://h/C:/a/b", "file://h/C:/"),
        ("/next", "file://h/C:/a/b", "file://h/C:/next"),
        ("C|", "file://host/dir/file", "file://host/C:"),
        ("C|/", "file://host/D:/dir/file", "file://host/C:/"),
        ("C|\n/", "file://host/dir/file", "file://host/C:/"),
        (r"C|\", "file://host/dir/file", "file://host/C:/"),
        ("/c:/foo/bar", "file://host/path", "file://host/c:/foo/bar"),
        ("file:C:/", "file://host/", "file://host/C:/"),
        ("file:/C:/", "file://host/", "file://host/C:/"),
        ("..", "file://x/C:/", "file://x/C:/"),
        ("../../..", "file://x/C:/a/b", "file://x/C:/"),
        ("..", "file://x/a/C:/", "file://x/a/"),
    ] {
        assert_file_url(input, Some(base), expected);
    }
}

#[test]
fn file_url_pathname_setters_preserve_empty_segments_and_hosts() {
    for (input, value, expected) in [
        ("file://monkey/", r"\\", "file://monkey//"),
        ("file:///unicorn", r"//\/", "file://////"),
        ("file:///unicorn", "//monkey/..//", "file://///"),
        ("file://host/old?q#f", "C|/new", "file://host/C:/new?q#f"),
        ("file://host/old", "", "file://host/"),
    ] {
        let mut url = Url::parse(input).unwrap();
        crate::components::set_pathname(&mut url, value);
        assert_eq!(url.as_str(), expected, "pathname = {value:?}");
        assert_eq!(Url::parse(url.as_str()).unwrap(), url);
    }
}
