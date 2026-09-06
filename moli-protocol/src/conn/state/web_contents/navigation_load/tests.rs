use super::*;
use crate::conn::state::web_contents::tests::{BrowserFixture, prepare};

#[test]
fn stale_load_admission_preserves_engine_policy_and_current_reservation() {
    rejected_admission(false);
}

#[test]
fn canceled_load_admission_preserves_engine_policy_and_current_reservation() {
    rejected_admission(true);
}

fn rejected_admission(cancel: bool) {
    let mut browser = BrowserFixture::new();
    let navigation = browser.contents.navigation.start_document_navigation();
    let load = browser.start(navigation).unwrap();
    let renderer = load.renderer_page();
    let document = load.document_id();
    browser.contents.tls_verify_host_override = Some(false);
    let rejected = if cancel {
        browser
            .contents
            .navigation
            .document_navigation_cancellation_handle(&navigation)
            .unwrap()
            .cancel();
        navigation
    } else {
        NavigationId::allocate()
    };
    assert!(browser.start(rejected).is_err());
    assert!(
        browser
            .contents
            .navigation_engine
            .as_ref()
            .unwrap()
            .fetch_config()
            .tls_verify_host()
    );
    assert_eq!(
        browser.contents.navigation.pending_document(),
        Some((navigation, document))
    );
    assert_eq!(
        browser
            .contents
            .navigation
            .accepts_document_preparation(navigation, renderer),
        !cancel
    );
    assert_eq!(load.identity().is_cancelled(), cancel);
}

#[tokio::test]
async fn response_preparation_consumes_its_admission_once() {
    let mut browser = BrowserFixture::new();
    let navigation = browser.contents.navigation.start_document_navigation();
    let mut load = browser.start(navigation).unwrap();
    let response = prepare(&mut load, "first").await.unwrap();
    assert!(
        matches!(prepare(&mut load, "second").await, Err(error) if error.to_string().contains("admission already consumed"))
    );
    let built = browser
        .materialization(navigation, response)
        .unwrap()
        .materialize()
        .await
        .unwrap();
    let committed = browser
        .contents
        .commit_document_navigation(built.page)
        .unwrap();
    assert_eq!(committed.info.title, "first");
    committed.retirement.close().await;
}

#[test]
fn canceling_response_transport_does_not_revoke_navigation_preparation() {
    let mut browser = BrowserFixture::new();
    let navigation = browser.contents.navigation.start_document_navigation();
    let load = browser.start(navigation).unwrap();
    load.request_cancellation.cancel();
    assert!(!load.identity().is_cancelled());
    assert!(
        browser
            .contents
            .navigation
            .accepts_document_preparation(navigation, load.renderer_page(),)
    );
    assert!(load.validate_request("https://navigation.example/").is_ok());
}

#[test]
fn replacing_navigation_revokes_all_admitted_transports() {
    let mut browser = BrowserFixture::new();
    let navigation = browser.contents.navigation.start_document_navigation();
    let first = browser.start(navigation).unwrap();
    let second = browser.start(navigation).unwrap();
    assert!(!first.request_cancellation.is_cancelled());
    assert!(!second.request_cancellation.is_cancelled());
    browser.contents.navigation.start_document_navigation();
    assert!(first.request_cancellation.is_cancelled());
    assert!(second.request_cancellation.is_cancelled());
    assert!(first.identity().is_cancelled());
    assert!(second.identity().is_cancelled());
}

#[tokio::test]
async fn replacement_reservation_cancels_unprepared_work_without_canceling_navigation() {
    let mut browser = BrowserFixture::new();
    let navigation = browser.contents.navigation.start_document_navigation();
    let mut first = browser.start(navigation).unwrap();
    let mut second = browser.start(navigation).unwrap();
    assert_ne!(first.renderer_page(), second.renderer_page());
    assert_eq!(first.document_id(), second.document_id());
    assert!(
        matches!(prepare(&mut first, "obsolete").await, Err(error) if error.to_string().contains("canceled navigation"))
    );
    let response = prepare(&mut second, "current").await.unwrap();
    let built = browser
        .materialization(navigation, response)
        .unwrap()
        .materialize()
        .await
        .unwrap();
    let committed = browser
        .contents
        .commit_document_navigation(built.page)
        .unwrap();
    assert_eq!(committed.info.title, "current");
    committed.retirement.close().await;
}

#[tokio::test]
async fn prepared_response_cannot_be_retargeted_to_a_new_navigation() {
    let mut browser = BrowserFixture::new();
    let first = browser.contents.navigation.start_document_navigation();
    let response = prepare(&mut browser.start(first).unwrap(), "obsolete")
        .await
        .unwrap();
    let second = browser.contents.navigation.start_document_navigation();
    let current = browser.start(second).unwrap();
    browser.contents.tls_verify_host_override = Some(false);
    assert!(
        matches!(browser.materialization(second, response), Err(error) if error == "stale navigation document candidate")
    );
    assert!(
        browser
            .contents
            .navigation_engine
            .as_ref()
            .unwrap()
            .fetch_config()
            .tls_verify_host()
    );
    assert_eq!(
        browser.contents.navigation.pending_document(),
        Some((second, current.document_id()))
    );
    assert!(browser.contents.main_frame.current_document.is_none());
}

#[tokio::test]
async fn prepared_response_cannot_be_retargeted_to_another_web_contents() {
    let mut first = BrowserFixture::new();
    let mut second = BrowserFixture::new();
    let navigation = first.contents.navigation.start_document_navigation();
    let response = prepare(&mut first.start(navigation).unwrap(), "first")
        .await
        .unwrap();
    let other = second.contents.navigation.start_document_navigation();
    let current = second.start(other).unwrap();
    second.contents.tls_verify_host_override = Some(false);
    assert!(
        matches!(second.materialization(other, response), Err(error) if error == "stale navigation document candidate")
    );
    assert!(
        second
            .contents
            .navigation_engine
            .as_ref()
            .unwrap()
            .fetch_config()
            .tls_verify_host()
    );
    assert_eq!(
        second.contents.navigation.pending_document(),
        Some((other, current.document_id()))
    );
    assert!(second.contents.main_frame.current_document.is_none());
}

#[tokio::test]
async fn superseded_prepared_response_cannot_mutate_policy_during_materialization() {
    let mut browser = BrowserFixture::new();
    let navigation = browser.contents.navigation.start_document_navigation();
    let response = prepare(&mut browser.start(navigation).unwrap(), "obsolete")
        .await
        .unwrap();
    let current = browser.start(navigation).unwrap();
    browser.contents.tls_verify_host_override = Some(false);
    assert!(
        matches!(browser.materialization(navigation, response), Err(error) if error == "stale navigation document candidate")
    );
    assert!(
        browser
            .contents
            .navigation_engine
            .as_ref()
            .unwrap()
            .fetch_config()
            .tls_verify_host()
    );
    assert!(
        browser
            .contents
            .navigation
            .accepts_document_preparation(navigation, current.renderer_page())
    );
}

#[tokio::test]
async fn superseded_materialization_cannot_execute_or_commit_the_old_candidate() {
    let mut browser = BrowserFixture::new();
    let navigation = browser.contents.navigation.start_document_navigation();
    let response = prepare(&mut browser.start(navigation).unwrap(), "obsolete")
        .await
        .unwrap();
    let materialization = browser.materialization(navigation, response).unwrap();
    let current = browser.start(navigation).unwrap();
    assert!(
        matches!(materialization.materialize().await, Err(error) if error.to_string().contains("canceled navigation"))
    );
    assert!(
        browser
            .contents
            .navigation
            .accepts_document_preparation(navigation, current.renderer_page())
    );
    assert!(browser.contents.main_frame.current_document.is_none());
}

#[tokio::test]
async fn superseded_materialized_candidate_cannot_commit() {
    let mut browser = BrowserFixture::new();
    let navigation = browser.contents.navigation.start_document_navigation();
    let response = prepare(&mut browser.start(navigation).unwrap(), "obsolete")
        .await
        .unwrap();
    let built = browser
        .materialization(navigation, response)
        .unwrap()
        .materialize()
        .await
        .unwrap();
    let current = browser.start(navigation).unwrap();
    assert!(matches!(
        browser.contents.commit_document_navigation(built.page),
        Err("stale navigation document candidate")
    ));
    assert!(
        browser
            .contents
            .navigation
            .accepts_document_preparation(navigation, current.renderer_page())
    );
    assert!(browser.contents.main_frame.current_document.is_none());
}

#[tokio::test]
async fn closing_web_contents_revokes_an_admitted_load_without_a_devtools_session() {
    let mut browser = BrowserFixture::new();
    let navigation = browser.contents.navigation.start_document_navigation();
    let mut load = browser.start(navigation).unwrap();
    drop(browser);
    assert!(
        matches!(prepare(&mut load, "closed").await, Err(error) if error.to_string().contains("canceled navigation"))
    );
}
