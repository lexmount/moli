use super::*;
use crate::browser::web_contents::tests::BrowserFixture;
use crate::{RendererDocumentTitleChanged, page::RendererDocumentLifecycleIdentity};
use serde_json::json;

fn title_change(browser: &BrowserFixture, title: &str) -> RendererDocumentTitleChanged {
    let snapshot = browser
        .contents
        .main_frame
        .current_document
        .as_ref()
        .unwrap()
        .lifecycle
        .snapshot()
        .unwrap();
    RendererDocumentTitleChanged {
        source_document: RendererDocumentLifecycleIdentity {
            frame: snapshot.frame,
            document: snapshot.document,
            epoch: snapshot.epoch,
        },
        title: title.into(),
    }
}

async fn push(browser: &mut BrowserFixture, fragment: &str) {
    let url = format!("https://navigation.example/#{fragment}");
    browser
        .evaluate(&format!("history.pushState(null, '', '{}')", url))
        .await;
    let document = browser
        .contents
        .main_frame
        .current_document
        .as_ref()
        .unwrap()
        .id;
    let committed = browser
        .contents
        .commit_same_document_navigation(
            document,
            Url::parse(&url).unwrap(),
            SameDocumentHistoryUpdate::Push,
        )
        .unwrap();
    assert_eq!(committed.document, document);
    assert_eq!(committed.web_contents, browser.contents.id());
    assert_eq!(committed.url.as_str(), url);
}

#[tokio::test]
async fn history_queries_preserve_observed_native_title_without_page_readback() {
    let mut browser = BrowserFixture::new();
    browser.navigate("cached title").await;
    let original = browser.contents.navigation_history_snapshot();
    let change = title_change(&browser, "observed title");
    assert_eq!(browser.contents.commit_document_title(&change), Some(true));
    assert_eq!(browser.contents.commit_document_title(&change), Some(false));
    let observed = browser.contents.navigation_history_snapshot();
    assert_eq!(observed.1[0].id, original.1[0].id);
    assert_eq!(observed.1[0].title, "observed title");
    assert_eq!(
        browser
            .contents
            .main_frame
            .current_document
            .as_ref()
            .unwrap()
            .page
            .document_title(),
        "cached title"
    );
    assert_eq!(
        browser
            .contents
            .navigation_history_entry_url(original.1[0].id),
        Some(original.1[0].url.clone())
    );
    assert_eq!(browser.contents.navigation_history_snapshot(), observed);

    browser.navigate("replacement").await;
    let replacement = browser.contents.navigation_history_snapshot();
    assert_eq!(browser.contents.commit_document_title(&change), None);
    assert_eq!(browser.contents.navigation_history_snapshot(), replacement);
    assert_eq!(replacement.1[0].title, "observed title");
    assert_eq!(replacement.1[1].title, "replacement");
}

#[tokio::test]
async fn same_document_history_rejects_replacement_foreign_and_pending_navigation() {
    let mut browser = BrowserFixture::new();
    let old_document = browser.navigate("outgoing").await;
    let current = browser.navigate("current").await;
    let before = browser.contents.navigation_history_snapshot();
    for document in [old_document, DocumentId::allocate()] {
        assert!(
            browser
                .contents
                .commit_same_document_navigation(
                    document,
                    Url::parse("https://navigation.example/#stale").unwrap(),
                    SameDocumentHistoryUpdate::Push
                )
                .is_none()
        );
        assert_eq!(browser.contents.navigation_history_snapshot(), before);
    }
    let pending = browser.contents.navigation.start_document_navigation();
    assert!(
        browser
            .contents
            .commit_same_document_navigation(
                current,
                Url::parse("https://navigation.example/#pending").unwrap(),
                SameDocumentHistoryUpdate::Push
            )
            .is_none()
    );
    assert_eq!(browser.contents.navigation_history_snapshot(), before);
    assert!(
        browser
            .contents
            .navigation
            .clear_pending_document_navigation_if_matches(&pending)
    );
    push(&mut browser, "accepted").await;
    let after = browser.contents.navigation_history_snapshot();
    assert_eq!(after.0, 2);
    assert_eq!(after.1.len(), 3);
    assert_eq!(after.1[2].title, "current");
    assert_eq!(
        after.1[1].document_sequence_number,
        after.1[2].document_sequence_number
    );
    assert_eq!(
        browser
            .contents
            .main_frame
            .current_document
            .as_ref()
            .unwrap()
            .id,
        current
    );
}

#[tokio::test]
async fn history_reset_commits_native_and_renderer_state_without_devtools() {
    let mut browser = BrowserFixture::new();
    let document = browser.navigate("current").await;
    push(&mut browser, "one").await;
    push(&mut browser, "two").await;
    let before = browser.contents.navigation_history_snapshot();
    assert_eq!(before.0, 2);
    assert_eq!(
        browser
            .evaluate("[history.length, navigation.entries().length]")
            .await,
        json!([3, 3])
    );
    let completion = browser
        .contents
        .start_reset_navigation_history()
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(
        browser
            .contents
            .finish_reset_navigation_history(completion)
            .unwrap()
    );
    assert_eq!(
        browser.contents.navigation_history_snapshot(),
        (0, vec![before.1[2].clone()])
    );
    assert_eq!(
        browser
            .evaluate("[history.length, navigation.entries().length, location.href]")
            .await,
        json!([1, 1, "https://navigation.example/#two"])
    );
    assert_eq!(
        browser
            .contents
            .main_frame
            .current_document
            .as_ref()
            .unwrap()
            .id,
        document
    );
}

#[tokio::test]
async fn history_reset_rejects_replacement_and_foreign_completions() {
    let mut browser = BrowserFixture::new();
    let old_document = browser.navigate("outgoing").await;
    push(&mut browser, "one").await;
    let stale = browser
        .contents
        .start_reset_navigation_history()
        .unwrap()
        .wait()
        .await
        .unwrap();
    let current = browser.navigate("current").await;
    let mut foreign = BrowserFixture::new();
    foreign.navigate("foreign").await;
    let foreign_completion = foreign
        .contents
        .start_reset_navigation_history()
        .unwrap()
        .wait()
        .await
        .unwrap();
    let before = browser.contents.navigation_history_snapshot();
    for completion in [stale, foreign_completion] {
        assert_eq!(
            browser.contents.finish_reset_navigation_history(completion),
            Err("stale history reset document".into())
        );
        assert_eq!(browser.contents.navigation_history_snapshot(), before);
        let document = browser
            .contents
            .main_frame
            .current_document
            .as_ref()
            .unwrap();
        assert_eq!(document.id, current);
        assert_ne!(document.id, old_document);
        assert_eq!(document.page.document_title(), "current");
    }
}

#[tokio::test]
async fn history_reset_rejects_pending_traversal_before_touching_renderer() {
    let mut browser = BrowserFixture::new();
    browser.navigate("current").await;
    push(&mut browser, "one").await;
    let before = browser.contents.navigation_history_snapshot();
    browser
        .contents
        .navigation
        .mark_next_navigation_history_traverse_to_entry(before.1[0].id);
    let traversal = browser.contents.navigation.start_document_navigation();
    assert!(
        matches!(browser.contents.start_reset_navigation_history(), Err(error) if error == "History cannot be pruned")
    );
    assert_eq!(browser.contents.navigation_history_snapshot(), before);
    assert_eq!(
        browser
            .evaluate("[history.length, navigation.entries().length]")
            .await,
        json!([2, 2])
    );
    assert!(
        browser
            .contents
            .clear_pending_document_navigation_if_matches(&traversal)
    );
    let completion = browser
        .contents
        .start_reset_navigation_history()
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(
        browser
            .contents
            .finish_reset_navigation_history(completion)
            .unwrap()
    );
}

#[tokio::test]
async fn renderer_history_reset_failure_preserves_browser_document_and_history() {
    let mut browser = BrowserFixture::new();
    let document = browser.navigate("current").await;
    push(&mut browser, "one").await;
    let before = browser.contents.navigation_history_snapshot();
    browser
        .contents
        .main_frame
        .current_document
        .as_ref()
        .unwrap()
        .page
        .crash_devtools_target_from_io();
    let result = match browser.contents.start_reset_navigation_history() {
        Ok(pending) => match pending.wait().await {
            Ok(completion) => browser.contents.finish_reset_navigation_history(completion),
            Err(error) => Err(error.to_string()),
        },
        Err(error) => Err(error),
    };
    assert!(result.is_err());
    assert_eq!(browser.contents.navigation_history_snapshot(), before);
    assert_eq!(
        browser
            .contents
            .main_frame
            .current_document
            .as_ref()
            .unwrap()
            .id,
        document
    );
    assert!(
        !browser.contents.crashed,
        "a failed command does not decide Browser retirement"
    );
}
