use super::*;
use crate::conn::{
    CommandOwnerScope, NavigationLoadOutcome, NavigationNetworkError, NavigationRequestLoadPolicy,
};
use moli_core::page::{SubresourceAuthCredentials, SubresourceAuthScheme, SubresourceAuthTarget};

const OFFLINE_ERROR_TEXT: &str = "net::ERR_INTERNET_DISCONNECTED";

fn navigation_fixture() -> (TestContext, NavigationDispatchState) {
    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-1".to_owned());
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    browser_context
        .start_document_navigation_for_active_target("LID-test".to_owned())
        .expect("fixture navigation should start");
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);
    let navigation = NavigationDispatchState {
        navigate_id: Some(1),
        owner: CommandOwnerScope::for_session("SID-1"),
        result_projection: NavigationResultProjection::Cdp(json!({
            "frameId": "TID-1", "loaderId": "LID-test",
        })),
        frame_id: "TID-1".to_owned(),
        session_id: Some("SID-1".to_owned()),
        request_id: Some("LID-test".to_owned()),
        loader_id: "LID-test".to_owned(),
        request_announced: true,
        requested_url: Url::parse("https://original.example/document").unwrap(),
        request_method: "GET".to_owned(),
        request_body: None,
        request_body_bytes: None,
        request_headers: Vec::new(),
        request_load_policy: NavigationRequestLoadPolicy::BrowserInitiated,
        timestamp: 0.0,
        source_document_security: Default::default(),
    };
    (ctx, navigation)
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_navigation_loaders_preserve_typed_error_causes_through_context() {
    let (mut ctx, mut navigation) = navigation_fixture();
    navigation.request_method = "POST".to_owned();
    navigation.request_headers = vec![("x-request".to_owned(), "offline".to_owned())];
    ctx.process_async(json!({
        "id": 2,
        "method": "Network.emulateNetworkConditions",
        "sessionId": "SID-1",
        "params": {
            "offline": true, "latency": 0,
            "downloadThroughput": -1, "uploadThroughput": -1,
        },
    }))
    .await;
    ctx.expect_result(2, json!({}), Some("SID-1"));

    let url = navigation.requested_url.as_str();
    let method = &navigation.request_method;
    let headers = &navigation.request_headers;
    let errors = [
        ctx.conn
            .load_navigation_request_via_runtime_with_network_events_for_navigation_async(
                &navigation,
                Default::default(),
            )
            .await
            .expect_err("offline document load must fail"),
        ctx.conn
            .fetch_navigation_response_async(method, url, None, headers.clone(), None)
            .await
            .expect_err("offline response fetch must fail"),
        ctx.conn
            .fetch_navigation_streaming_raw_response_async(method, url, None, headers.clone(), None)
            .await
            .expect_err("offline streaming response fetch must fail"),
        ctx.conn
            .fetch_navigation_auth_raw_response_for_owner_async(
                &navigation.owner,
                navigation.request_load_policy,
                method,
                url,
                None,
                headers.clone(),
                SubresourceAuthCredentials {
                    target: SubresourceAuthTarget::Server,
                    scheme: SubresourceAuthScheme::Basic,
                    username: "test-user".to_owned(),
                    password: "test-password".to_owned(),
                },
            )
            .await
            .expect_err("offline authenticated response fetch must fail"),
    ];
    for error in errors {
        let error = error
            .context("failed to load document")
            .context("failed to continue intercepted navigation");
        let network_error = error
            .downcast_ref::<NavigationNetworkError>()
            .expect("navigation request identity must survive diagnostic context");
        assert_eq!(network_error.error_text, OFFLINE_ERROR_TEXT);
        assert_eq!(network_error.unreachable_url, navigation.requested_url);
        assert_eq!(network_error.request_method, *method);
        assert_eq!(network_error.request_headers, *headers);
        assert!(
            error.root_cause().is::<NavigationNetworkError>(),
            "network failure must be the error cause, not a context wrapper: {error:#}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_error_document_uses_typed_failure_request_after_context() {
    let (mut ctx, navigation) = navigation_fixture();
    let unreachable_url = Url::parse("https://redirect.example/unreachable").unwrap();
    let request_headers = vec![("x-request".to_owned(), "failed-hop".to_owned())];
    let error = anyhow::Error::new(NavigationNetworkError {
        error_text: OFFLINE_ERROR_TEXT.to_owned(),
        unreachable_url: unreachable_url.clone(),
        request_method: "POST".to_owned(),
        request_headers: request_headers.clone(),
    })
    .context("failed to load document")
    .context("failed to continue intercepted navigation");
    let outcome = ctx
        .conn
        .prepare_navigation_load_error_for_navigation_async(&navigation, error)
        .await
        .expect("typed network failure should prepare an error document");
    let NavigationLoadOutcome::ResponseCommitReady(prepared) = outcome else {
        panic!("network failure must prepare an error document commit");
    };
    let permit = prepared.issue_commit_permit();
    let loaded = prepared
        .commit(permit)
        .await
        .expect("error document should commit");
    assert_eq!(loaded.requested_url, unreachable_url);
    assert_eq!(loaded.request_method, "POST");
    assert_eq!(loaded.request_headers, request_headers);
    let error_page = loaded
        .network_error_page
        .expect("network error page metadata");
    assert_eq!(error_page.unreachable_url(), &unreachable_url);
    assert_eq!(error_page.error_text(), OFFLINE_ERROR_TEXT);
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_error_boundary_does_not_classify_display_text_as_network_failure() {
    let (mut ctx, navigation) = navigation_fixture();
    for error in [
        anyhow::anyhow!(OFFLINE_ERROR_TEXT),
        anyhow::anyhow!(OFFLINE_ERROR_TEXT)
            .context("failed to load document")
            .context("failed to continue intercepted navigation"),
    ] {
        let expected_chain = format!("{error:#}");
        let error = ctx
            .conn
            .prepare_navigation_load_error_for_navigation_async(&navigation, error)
            .await
            .expect_err("a matching display string must remain a non-network error");
        assert!(error.downcast_ref::<NavigationNetworkError>().is_none());
        assert_eq!(format!("{error:#}"), expected_chain);
        assert_eq!(error.root_cause().to_string(), OFFLINE_ERROR_TEXT);
    }
}
