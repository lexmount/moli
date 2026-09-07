use crate::browser::web_contents::tests::BrowserFixture;
use moli_browser_profile::BrowserIdentityProfile;
use serde_json::json;

#[tokio::test]
async fn resource_rebuild_without_a_document_only_installs_native_policy() {
    let mut browser = BrowserFixture::new();
    browser.contents.browser_identity_override =
        Some(BrowserIdentityProfile::new("Native/1", "en"));
    browser.contents.tls_verify_host_override = Some(false);
    assert!(
        browser
            .contents
            .start_resource_runtime_rebuild(&browser.inherited())
            .unwrap()
            .is_none()
    );
    let client = browser
        .contents
        .ensure_resource_request_client(&browser.inherited())
        .unwrap();
    assert_eq!(
        client
            .browser_resource_runtime()
            .browser_identity()
            .user_agent(),
        "Native/1"
    );
    assert!(
        !browser
            .contents
            .navigation_fetch_config()
            .unwrap()
            .tls_verify_host()
    );
    assert!(browser.contents.main_frame.current_document.is_none());
    assert!(browser.contents.navigation.pending_document().is_none());
}

#[tokio::test]
async fn resource_rebuild_preserves_document_and_storage_after_engine_invalidation() {
    let mut browser = BrowserFixture::new();
    let document = browser.navigate("current").await;
    browser.evaluate("document.cookie = 'native=value'; localStorage.setItem('key', 'local'); sessionStorage.setItem('key', 'session')").await;
    assert_eq!(
        browser
            .evaluate("[localStorage.getItem('key'), sessionStorage.getItem('key')]")
            .await,
        json!(["local", "session"])
    );
    let original = browser
        .contents
        .ensure_resource_request_client(&browser.inherited())
        .unwrap();
    browser.contents.invalidate_resource_runtime();
    browser.contents.browser_identity_override =
        Some(BrowserIdentityProfile::new("Native/2", "en"));
    browser.contents.tls_verify_host_override = Some(false);
    let pending = browser
        .contents
        .start_resource_runtime_rebuild(&browser.inherited())
        .unwrap()
        .unwrap();
    let completion = pending.wait().await.unwrap();
    browser
        .contents
        .finish_resource_runtime_update(completion)
        .unwrap();
    let current = browser
        .contents
        .ensure_resource_request_client(&browser.inherited())
        .unwrap();
    assert!(!original.shares_resource_runtime_with(&current));
    assert!(std::sync::Arc::ptr_eq(
        &original.cookie_store(),
        &current.cookie_store()
    ));
    assert!(original.shares_page_network_policy_with(&current));
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
    assert_eq!(browser.evaluate("[document.title, navigator.userAgent, document.cookie, localStorage.getItem('key'), sessionStorage.getItem('key')]").await,
        json!(["current", "Native/2", "native=value", "local", "session"]));
}

#[tokio::test]
async fn resource_update_completion_cannot_overwrite_a_replacement_document() {
    let mut browser = BrowserFixture::new();
    let original = browser.navigate("outgoing").await;
    browser.contents.browser_identity_override =
        Some(BrowserIdentityProfile::new("Native/outgoing", "en"));
    let completion = browser
        .contents
        .start_resource_runtime_rebuild(&browser.inherited())
        .unwrap()
        .unwrap()
        .wait()
        .await
        .unwrap();
    browser.contents.browser_identity_override =
        Some(BrowserIdentityProfile::new("Native/current", "en"));
    let current = browser.navigate("current").await;
    assert_ne!(original, current);
    browser
        .contents
        .finish_resource_runtime_update(completion)
        .unwrap();
    let document = browser
        .contents
        .main_frame
        .current_document
        .as_ref()
        .unwrap();
    assert_eq!(document.id, current);
    assert_eq!(
        document.page.document_title(),
        "current",
        "a stale completion must not install its cached renderer snapshot"
    );
    assert_eq!(
        browser
            .evaluate("[document.title, navigator.userAgent]")
            .await,
        json!(["current", "Native/current"])
    );
}

#[tokio::test]
async fn resource_update_completion_cannot_modify_another_web_contents() {
    let mut first = BrowserFixture::new();
    let mut second = BrowserFixture::new();
    first.navigate("first").await;
    let second_document = second.navigate("second").await;
    let original_identity = second.evaluate("navigator.userAgent").await;
    first.contents.browser_identity_override =
        Some(BrowserIdentityProfile::new("Native/first", "en"));
    let completion = first
        .contents
        .start_resource_runtime_rebuild(&first.inherited())
        .unwrap()
        .unwrap()
        .wait()
        .await
        .unwrap();
    second
        .contents
        .finish_resource_runtime_update(completion)
        .unwrap();
    let document = second
        .contents
        .main_frame
        .current_document
        .as_ref()
        .unwrap();
    assert_eq!(document.id, second_document);
    assert_eq!(document.page.document_title(), "second");
    assert_eq!(
        second.evaluate("navigator.userAgent").await,
        original_identity
    );
    assert_eq!(
        first.evaluate("navigator.userAgent").await,
        json!("Native/first")
    );
}

#[tokio::test]
async fn failed_renderer_update_does_not_retire_the_browser_document() {
    let mut browser = BrowserFixture::new();
    let document = browser.navigate("current").await;
    browser
        .contents
        .main_frame
        .current_document
        .as_ref()
        .unwrap()
        .page
        .crash_devtools_target_from_io();
    let result = match browser
        .contents
        .start_resource_runtime_rebuild(&browser.inherited())
    {
        Ok(Some(pending)) => match pending.wait().await {
            Ok(completion) => browser.contents.finish_resource_runtime_update(completion),
            Err(error) => Err(error.to_string()),
        },
        Ok(None) => panic!("the Browser still owns a Document"),
        Err(error) => Err(error),
    };
    assert!(
        result.is_err(),
        "a stopped renderer must reject the resource update"
    );
    // Renderer failure is not resource-maintenance authority to remove the
    // Browser Document. Browser lifecycle termination decides its retirement.
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
    assert!(!browser.contents.crashed);
}
