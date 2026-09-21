use super::*;
use crate::conn::{
    CommandOwnerScope, DocumentNavigationToken, NavigationLoadOutcome, NavigationNetworkError,
    NavigationNetworkErrorKind, NavigationRequestLoadPolicy,
};
use moli_core::page::{SubresourceAuthCredentials, SubresourceAuthScheme, SubresourceAuthTarget};

const OFFLINE_ERROR_TEXT: &str = "net::ERR_INTERNET_DISCONNECTED";

fn failing_streamed_document(
    navigation: &NavigationDispatchState,
) -> crate::conn::DocumentBodySource {
    let (chunks_tx, chunks_rx) = tokio::sync::mpsc::unbounded_channel();
    chunks_tx.send(b"partial document".to_vec()).unwrap();
    drop(chunks_tx);
    let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();
    completion_tx
        .send(Err(anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "typed response body failure",
        ))
        .context("response transport failed")))
        .unwrap();
    let response = moli_fetch::StreamingRawResponse::new(
        navigation.requested_url.clone(),
        200,
        Vec::new(),
        None,
        Vec::new(),
        false,
        Vec::new(),
        chunks_rx,
        moli_fetch::FetchCancelHandle::new(),
        completion_rx,
    );
    crate::conn::DocumentBodySource::StreamingRaw {
        requested_url: navigation.requested_url.clone(),
        request_method: navigation.request_method.clone(),
        request_headers: navigation.request_headers.clone(),
        response,
        network_observation_journal: Default::default(),
        body_progress_source: Default::default(),
    }
}

#[tokio::test]
async fn fetch_body_materialization_preserves_error_type_and_request_identity() {
    let (_ctx, _token, navigation) = navigation_fixture();
    let (error, body) = failing_streamed_document(&navigation)
        .materialize_body_limited_async(1024)
        .await
        .expect_err("partial transport failure must fail body materialization");
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .expect("materialization must preserve the I/O cause")
            .kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    assert!(format!("{error:#}").contains("response transport failed"));
    assert!(format!("{error:#}").contains("failed to read page body from stream"));
    let crate::conn::DocumentBodySource::CapturedRaw {
        requested_url,
        request_method,
        request_headers,
        ..
    } = body
    else {
        panic!("failed streamed materialization must return its captured source");
    };
    assert_eq!(requested_url, navigation.requested_url);
    assert_eq!(request_method, navigation.request_method);
    assert_eq!(request_headers, navigation.request_headers);
}

#[tokio::test]
async fn fetch_body_stream_read_preserves_error_type_and_paused_transfer() {
    let (_ctx, _token, navigation) = navigation_fixture();
    let body = failing_streamed_document(&navigation);
    let transfer = crate::conn::PausedDocumentTransfer::pending(
        "fetch-typed-error".to_owned(),
        None,
        navigation,
        body,
    )
    .open_body_stream("stream-typed-error".to_owned())
    .expect("streamed response should open")
    .transfer;
    let (transfer, error) = transfer
        .read_body_stream_async(None)
        .await
        .expect_err("partial transport failure must fail the stream read");
    assert_eq!(transfer.fetch_request_id(), "fetch-typed-error");
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .expect("stream read must preserve the I/O cause")
            .kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    assert!(format!("{error:#}").contains("response transport failed"));
    assert!(format!("{error:#}").contains("failed to read page body from stream"));
}

fn navigation_fixture() -> (
    TestContext,
    DocumentNavigationToken,
    NavigationDispatchState,
) {
    let mut ctx = TestContext::new();
    let mut browser_context = BrowserContext::new("BID-1".to_owned());
    browser_context.set_active_target_id("TID-1");
    browser_context.attach_active_session("SID-1");
    let token = browser_context
        .start_document_navigation_for_active_target("LID-test".to_owned())
        .expect("fixture navigation should start");
    ctx.conn
        .install_browser_context_fixture_for_test(browser_context);
    let navigation = NavigationDispatchState {
        redirect_chain: Vec::new(),
        redirect_headers: None,
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
        request_headers: Vec::new().into(),
        request_load_policy: NavigationRequestLoadPolicy::BrowserInitiated,
        timestamp: 0.0,
        source_document_security: Default::default(),
    };
    (ctx, token, navigation)
}

fn streaming_response_job(
    conn: &mut CdpConnection,
    token: &DocumentNavigationToken,
    navigation: &NavigationDispatchState,
) -> (
    Option<crate::conn::runtime_load::BackgroundStreamingResponseNavigationLoadJob>,
    moli_fetch::FetchCancelHandle,
) {
    let crate::conn::DocumentBodySource::StreamingRaw { response, .. } =
        failing_streamed_document(navigation)
    else {
        unreachable!()
    };
    let cancellation = response.cancellation_handle();
    let job = conn.background_streaming_response_navigation_load_job_for_navigation(
        token,
        navigation,
        response,
        Default::default(),
        None,
        Vec::new(),
        Default::default(),
    );
    (job, cancellation)
}

#[tokio::test]
async fn document_navigation_cancellation_requires_the_exact_pending_token() {
    let (mut ctx, _, _) = navigation_fixture();
    ctx.install_navigation_fixture_for_session_owner("about:blank", Some("SID-1"))
        .await;
    let slot = ctx
        .conn
        .browser_context
        .as_mut()
        .unwrap()
        .active_page_target_mut()
        .runtime_slot
        .page_slot_mut();
    let first = slot.start_document_navigation("TID-1".to_owned(), "LID-test".to_owned());
    let first_cancellation = slot
        .document_navigation_cancellation_handle(&first)
        .unwrap();
    assert!(!first_cancellation.is_cancelled());
    let current = slot.start_document_navigation("TID-1".to_owned(), "LID-test".to_owned());
    assert!(first_cancellation.is_cancelled());

    for mismatch in [
        first,
        DocumentNavigationToken {
            target_id: "other target".to_owned(),
            ..current.clone()
        },
        DocumentNavigationToken {
            loader_id: "other loader".to_owned(),
            ..current.clone()
        },
    ] {
        assert!(
            slot.document_navigation_cancellation_handle(&mismatch)
                .is_none()
        );
    }
    let current_cancellation = slot
        .document_navigation_cancellation_handle(&current)
        .unwrap();
    assert!(!current_cancellation.is_cancelled());
    assert!(slot.commit_pending_document_navigation_if_matches(&current));
    assert!(
        slot.document_navigation_cancellation_handle(&current)
            .is_none()
    );
    assert!(!current_cancellation.is_cancelled());
}

#[tokio::test]
async fn superseded_streaming_response_cannot_arm_the_current_navigation() {
    let (mut ctx, stale, navigation) = navigation_fixture();
    // Even reusing the target, loader and URL must not reuse a request's authority.
    let current = ctx
        .conn
        .browser_context
        .as_mut()
        .unwrap()
        .start_document_navigation_for_active_target(navigation.loader_id.clone())
        .unwrap();
    let current_cancellation = ctx
        .conn
        .document_navigation_cancellation_handle(&current)
        .unwrap();
    assert_ne!(stale.request_id, current.request_id);

    let (job, stale_cancellation) = streaming_response_job(&mut ctx.conn, &stale, &navigation);
    assert!(
        job.is_none(),
        "stale responses must not create renderer preparation jobs"
    );
    assert!(stale_cancellation.is_cancelled());
    assert!(!current_cancellation.is_cancelled());
    assert!(!ctx.conn.has_inflight_background_navigation());

    let (job, _) = streaming_response_job(&mut ctx.conn, &current, &navigation);
    assert!(
        job.is_some(),
        "the matching response must still be accepted"
    );
    assert!(ctx.conn.has_inflight_background_navigation());
}

#[tokio::test]
async fn duplicate_streaming_response_does_not_cancel_the_armed_navigation() {
    let (mut ctx, token, navigation) = navigation_fixture();
    let (first, first_cancellation) = streaming_response_job(&mut ctx.conn, &token, &navigation);
    assert!(first.is_some());
    let (duplicate, duplicate_cancellation) =
        streaming_response_job(&mut ctx.conn, &token, &navigation);
    assert!(
        duplicate.is_none(),
        "an already armed request must not spawn another job"
    );
    assert!(duplicate_cancellation.is_cancelled());
    assert!(!first_cancellation.is_cancelled());
    assert!(ctx.conn.has_inflight_background_navigation());
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_navigation_loaders_preserve_typed_error_causes_through_context() {
    let (mut ctx, token, mut navigation) = navigation_fixture();
    navigation.request_method = "POST".to_owned();
    navigation.request_headers =
        moli_fetch::RequestHeaders::from_bytes(vec![("x-request".to_owned(), vec![0xe9, 0xff])]);
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
                Some(&token),
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
            .fetch_navigation_auth_raw_response_for_navigation_async(
                &navigation,
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
        assert_eq!(
            network_error.kind,
            NavigationNetworkErrorKind::InternetDisconnected
        );
        assert_eq!(network_error.to_string(), OFFLINE_ERROR_TEXT);
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
    let (mut ctx, token, navigation) = navigation_fixture();
    let unreachable_url = Url::parse("https://overridden.example/unreachable").unwrap();
    let request_headers = vec![("x-request".to_owned(), "overridden-request".to_owned())];
    let error = anyhow::Error::new(NavigationNetworkError {
        kind: NavigationNetworkErrorKind::InternetDisconnected,
        unreachable_url: unreachable_url.clone(),
        request_method: "POST".to_owned(),
        request_headers: request_headers.clone().into(),
    })
    .context("failed to load document")
    .context("failed to continue intercepted navigation");
    let outcome = ctx
        .conn
        .prepare_navigation_load_error_for_navigation_async(Some(&token), &navigation, error)
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
    assert_eq!(loaded.request_headers, request_headers.into());
    let error_page = loaded
        .network_error_page
        .expect("network error page metadata");
    assert_eq!(error_page.unreachable_url(), &unreachable_url);
    assert_eq!(error_page.error_text(), OFFLINE_ERROR_TEXT);
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_error_boundary_does_not_classify_display_text_as_network_failure() {
    let (mut ctx, token, navigation) = navigation_fixture();
    for error in [
        anyhow::anyhow!(OFFLINE_ERROR_TEXT),
        anyhow::anyhow!(OFFLINE_ERROR_TEXT)
            .context("failed to load document")
            .context("failed to continue intercepted navigation"),
    ] {
        let expected_chain = format!("{error:#}");
        let error = ctx
            .conn
            .prepare_navigation_load_error_for_navigation_async(Some(&token), &navigation, error)
            .await
            .expect_err("a matching display string must remain a non-network error");
        assert!(error.downcast_ref::<NavigationNetworkError>().is_none());
        assert_eq!(format!("{error:#}"), expected_chain);
        assert_eq!(error.root_cause().to_string(), OFFLINE_ERROR_TEXT);
    }
}
