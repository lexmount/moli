use super::*;

const PROBE: &str = include_str!("navigation_window_reuse.js");

async fn check_navigation_window_reuse(mode: &str, replace_again: bool) {
    let mut page = SameDocumentPage::new().await;
    let result = page
        .evaluate(&format!(
            "{PROBE}\nnavigationWindowReuseProbe({mode:?}, {replace_again})"
        ))
        .await;
    let reused = matches!(mode, "http" | "srcdoc");
    let mut expected = json!({
        "sameNavigation": reused,
        "sameRealm": reused,
        "documentChanged": true,
        "marker": reused,
        "entries": if mode == "document-open" { 2 } else { 1 },
        "currentURL": true,
        "events": if reused { vec!["property", "listener"] } else { vec![] },
        "canceled": reused,
    });
    if replace_again {
        expected["second"] = json!({
            "newNavigation": true,
            "newRealm": true,
            "marker": false,
            "oldEntries": 0,
            "oldCurrentEntry": true,
            "oldEvents": [],
            "fragmentCommitted": true,
        });
    }
    assert_eq!(result, expected, "{mode}/{replace_again}");
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_window_reuse_preserves_initial_http_listeners() {
    for replace_again in [false, true] {
        check_navigation_window_reuse("http", replace_again).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_window_reuse_preserves_initial_srcdoc_listeners() {
    for replace_again in [false, true] {
        check_navigation_window_reuse("srcdoc", replace_again).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_window_reuse_ends_after_document_open() {
    for replace_again in [false, true] {
        check_navigation_window_reuse("document-open", replace_again).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_window_reuse_ends_after_initial_blank_load() {
    for replace_again in [false, true] {
        check_navigation_window_reuse("blank", replace_again).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_window_reuse_ends_after_initial_blank_fragment_navigation() {
    for replace_again in [false, true] {
        check_navigation_window_reuse("fragment", replace_again).await;
    }
}
