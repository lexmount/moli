use url::Url;

/// Resolve upstream browser-test child documents from the renderer fixture
/// corpus. This is intentionally available only to tests and debug builds;
/// production HTTP(S) navigations must use the network loader.
pub(super) fn child_upstream_fixture_text(url: &Url) -> Option<String> {
    #[cfg(any(test, debug_assertions))]
    {
        let path = url.path().strip_prefix("/src/browser/tests/")?;
        let relative_path = path.replace("%20", " ");
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/lightpanda/upstream")
            .join(relative_path);
        std::fs::read_to_string(fixture_path).ok()
    }
    #[cfg(not(any(test, debug_assertions)))]
    {
        let _ = url;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_upstream_fixture_lookup_is_path_scoped() {
        let fixture = Url::parse(
            "https://example.test/src/browser/tests/frames/support/target_fragment_child.html",
        )
        .unwrap();
        assert!(
            child_upstream_fixture_text(&fixture)
                .is_some_and(|source| source.contains("id=\"target\""))
        );

        let unrelated = Url::parse(
            "https://example.test/src/browser/test/frames/support/target_fragment_child.html",
        )
        .unwrap();
        assert!(child_upstream_fixture_text(&unrelated).is_none());
    }
}
