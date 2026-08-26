use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use moli_core::page::{
    ScriptNetworkOutputItem, SubresourceBodyFinished, SubresourceNetworkRequestHandle,
    SubresourceRequestInitiatorType, SubresourceRequestStarted, SubresourceResourceType,
    SubresourceResponseBody, SubresourceResponseStarted,
};

use crate::domains::network::{
    NetworkBacklogProjectionContext, NetworkPreparedOutputs, PendingSubresourceNetworkActivity,
    PendingSubresourceNetworkActivitySession, TargetNetworkBacklogRequestIdResolver,
    TargetNetworkOutputQueue, TargetSubresourcePlanOutput,
    emit_pending_network_backlog_activity_background_events,
    start_observed_main_document_navigation_progress_background_events,
};

use super::*;

fn bidi_network_context(session_id: &str) -> crate::devtools_runtime::DevToolsCommandContext {
    crate::devtools_runtime::DevToolsCommandContext {
        protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
        session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(session_id)),
        target_id: None,
        browser_context_id: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_respects_recorded_session_visibility() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("SID-primary".to_owned());
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned()));
    bc.enable_auxiliary_network_events("SID-aux");
    bc.record_captured_response_body(
        "REQ-aux-only".to_owned(),
        "aux-only body".to_owned(),
        [Some("SID-aux".to_owned())],
    );
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 7_280,
        "method": "Network.getResponseBody",
        "sessionId": "SID-primary",
        "params": { "requestId": "REQ-aux-only" }
    }))
    .await;
    ctx.expect_error(7_280, -32000, "No resource with given identifier found");

    ctx.process_async(json!({
        "id": 7_281,
        "method": "Network.getResponseBody",
        "sessionId": "SID-aux",
        "params": { "requestId": "REQ-aux-only" }
    }))
    .await;
    ctx.expect_result(
        7_281,
        json!({ "body": "aux-only body", "base64Encoded": false }),
        Some("SID-aux"),
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_requires_calling_session_network_listener() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("SID-primary".to_owned());
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned()));
    bc.record_captured_response_body(
        "REQ-shared".to_owned(),
        "shared body".to_owned(),
        [Some("SID-primary".to_owned()), Some("SID-aux".to_owned())],
    );
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 7_282,
        "method": "Network.getResponseBody",
        "sessionId": "SID-aux",
        "params": { "requestId": "REQ-shared" }
    }))
    .await;
    ctx.expect_error(7_282, -32000, "No resource with given identifier found");

    ctx.process_async(json!({
        "id": 7_283,
        "method": "Network.enable",
        "sessionId": "SID-aux",
        "params": {}
    }))
    .await;
    ctx.expect_result(7_283, json!({}), Some("SID-aux"));

    ctx.process_async(json!({
        "id": 7_284,
        "method": "Network.getResponseBody",
        "sessionId": "SID-aux",
        "params": { "requestId": "REQ-shared" }
    }))
    .await;
    ctx.expect_result(
        7_284,
        json!({ "body": "shared body", "base64Encoded": false }),
        Some("SID-aux"),
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_reports_pending_body_as_existing_without_data() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    bc.record_pending_response_body("REQ-pending".to_owned(), [None::<String>]);
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 7_281,
        "method": "Network.getResponseBody",
        "params": { "requestId": "REQ-pending" }
    }))
    .await;
    ctx.expect_error(
        7_281,
        -32000,
        "No data found for resource with given identifier",
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_ready_body_replaces_pending_body() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    bc.record_pending_response_body("REQ-ready".to_owned(), [None::<String>]);
    bc.record_captured_response_body(
        "REQ-ready".to_owned(),
        "ready body".to_owned(),
        [None::<String>],
    );
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 7_282,
        "method": "Network.getResponseBody",
        "params": { "requestId": "REQ-ready" }
    }))
    .await;
    ctx.expect_result(
        7_282,
        json!({ "body": "ready body", "base64Encoded": false }),
        None,
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_rejects_bodies_over_materialization_limit() {
    let mut config = FetchConfig::default();
    config.set_connection_limits(None, None, Some(4));
    let mut ctx = TestContext::new();
    ctx.conn = CdpConnection::new_with_fetch_config(config);
    let mut bc = BrowserContext::new("BID-1".into());
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    bc.record_captured_response_body("REQ-large".to_owned(), "hello".to_owned(), [None::<String>]);
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 7_282,
        "method": "Network.getResponseBody",
        "params": { "requestId": "REQ-large" }
    }))
    .await;
    ctx.expect_error(
        7_282,
        -32000,
        "response body is 5 bytes, exceeds CDP materialization limit of 4 bytes",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_reports_default_single_resource_budget_eviction() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    bc.record_captured_response_body_source(
        "REQ-over-budget".to_owned(),
        CapturedBody::from_bytes(vec![b'x'; 2_000_001]),
        [None::<String>],
    );
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 7_285,
        "method": "Network.getResponseBody",
        "params": { "requestId": "REQ-over-budget" }
    }))
    .await;
    ctx.expect_error(
        7_285,
        -32000,
        "Request content was evicted from inspector cache",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_base64_encodes_non_utf8_captured_bytes() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    bc.record_captured_response_body_source(
        "REQ-binary".to_owned(),
        CapturedBody::from_bytes(vec![0x00, 0xff, b'a']),
        [None::<String>],
    );
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 7_283,
        "method": "Network.getResponseBody",
        "params": { "requestId": "REQ-binary" }
    }))
    .await;
    ctx.expect_result(
        7_283,
        json!({ "body": "AP9h", "base64Encoded": true }),
        None,
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_request_post_data_matches_chromium_errors_and_binary_encoding() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    bc.record_pending_response_body("REQ-get".to_owned(), [None::<String>]);
    bc.record_pending_response_body("REQ-empty".to_owned(), [None::<String>]);
    bc.active_target
        .runtime_slot
        .record_captured_request_body_with_collector_scope(
            "REQ-empty".to_owned(),
            Vec::new(),
            [None::<String>],
            std::iter::empty::<String>(),
            false,
        );
    bc.record_pending_response_body("REQ-binary".to_owned(), [None::<String>]);
    bc.active_target
        .runtime_slot
        .record_captured_request_body_with_collector_scope(
            "REQ-binary".to_owned(),
            vec![0x00, 0xff, b'a'],
            [None::<String>],
            std::iter::empty::<String>(),
            false,
        );
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 7_284,
        "method": "Network.getRequestPostData",
        "params": { "requestId": "REQ-missing" }
    }))
    .await;
    ctx.expect_error(7_284, -32000, "No resource with given id was found");

    ctx.process_async(json!({
        "id": 7_285,
        "method": "Network.getRequestPostData",
        "params": { "requestId": "REQ-get" }
    }))
    .await;
    ctx.expect_error(7_285, -32000, "No post data available for the request");

    ctx.process_async(json!({
        "id": 7_286,
        "method": "Network.getRequestPostData",
        "params": { "requestId": "REQ-empty" }
    }))
    .await;
    ctx.expect_error(7_286, -32000, "No post data available for the request");

    ctx.process_async(json!({
        "id": 7_287,
        "method": "Network.getRequestPostData",
        "params": { "requestId": "REQ-binary" }
    }))
    .await;
    ctx.expect_result(
        7_287,
        json!({ "postData": "AP9h", "base64Encoded": true }),
        None,
    );
}
#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_returns_partial_body_after_staged_loading_failed() {
    struct StableRequestIds;

    impl TargetNetworkBacklogRequestIdResolver for StableRequestIds {
        fn request_id_for_subresource_output(
            &mut self,
            output: &TargetSubresourcePlanOutput,
        ) -> String {
            output
                .request_handle()
                .map(|handle| format!("REQ-H{}", handle.get()))
                .unwrap_or_else(|| format!("REQ-{}", output.index() + 1))
        }

        fn request_id_for_websocket_socket(&mut self, socket_id: u64) -> String {
            format!("REQ-WS{socket_id}")
        }
    }

    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("SID-1".to_owned());
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    let handle = SubresourceNetworkRequestHandle::new(41);
    let document_url = Url::parse("https://example.test/page").unwrap();
    let request_url = Url::parse("https://example.test/sw-stream").unwrap();
    let request = SubresourceRequestStarted::new(
        handle,
        Some("FRAME-1".to_owned()),
        document_url,
        request_url.clone(),
        "GET".to_owned(),
        Vec::new(),
        None,
        SubresourceResourceType::Fetch,
        SubresourceRequestInitiatorType::Script,
        None,
    );
    let response = SubresourceResponseStarted::new(
        handle,
        Vec::new(),
        request_url.clone(),
        200,
        vec![("content-type".to_owned(), "text/plain".to_owned())],
        Vec::new(),
    );
    let body = SubresourceBodyFinished::failed_with_partial_body(
        handle,
        "net::ERR_ABORTED".to_owned(),
        SubresourceResponseBody::from_text("partial body".to_owned()),
    );
    let items = vec![
        ScriptNetworkOutputItem::SubresourceRequestStarted(Box::new(request)),
        ScriptNetworkOutputItem::SubresourceResponseStarted(Box::new(response)),
        ScriptNetworkOutputItem::SubresourceBodyFinished(Box::new(body)),
    ];
    let mut output_queue = TargetNetworkOutputQueue::default();
    for item in &items {
        output_queue.append_renderer_output_item_for_loader(item, "LOADER-1");
    }
    let activity = PendingSubresourceNetworkActivity::from_sessions(vec![
        PendingSubresourceNetworkActivitySession::new(Some("SID-1".to_owned()), 0),
    ])
    .expect("test activity should contain one session");
    let mut request_ids = StableRequestIds;
    let backlog =
        output_queue.backlog_prepared_delivery_for_activity(Some(activity), None, &mut request_ids);
    let mut prepared_outputs = NetworkPreparedOutputs::default();
    *prepared_outputs.backlog_mut() = backlog;

    let mut emitted_events = Vec::new();
    emit_pending_network_backlog_activity_background_events(
        &mut ctx.conn,
        &mut emitted_events,
        NetworkBacklogProjectionContext::new(Some("SID-1"))
            .with_base_timestamp(Some(100.0))
            .with_prepared_outputs(Some(&mut prepared_outputs)),
    );
    let emitted = emitted_events
        .into_iter()
        .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
        .collect::<Vec<_>>();

    let request = emitted
        .iter()
        .find(|message| message["method"] == json!("Network.requestWillBeSent"))
        .expect("staged fetch request should emit requestWillBeSent");
    assert_eq!(request["sessionId"], "SID-1");
    assert_eq!(request["params"]["type"], "Fetch");
    assert_eq!(request["params"]["request"]["url"], request_url.as_str());
    let request_id = request["params"]["requestId"]
        .as_str()
        .expect("staged fetch request should have a request id")
        .to_owned();

    assert!(emitted.iter().any(|message| {
        message["method"] == json!("Network.responseReceived")
            && message["params"]["requestId"] == json!(request_id)
            && message["params"]["response"]["status"] == json!(200)
    }));
    let failed = emitted
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFailed")
                && message["params"]["requestId"] == json!(request_id)
        })
        .expect("a failed stream should emit loadingFailed");
    assert_eq!(failed["params"]["errorText"], json!("net::ERR_ABORTED"));
    assert_eq!(failed["params"]["canceled"], json!(true));
    assert!(
        !emitted.iter().any(|message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(request_id)
        }),
        "a failed stream must still emit loadingFailed rather than loadingFinished"
    );

    ctx.process_async(json!({
        "id": 7_285,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": request_id }
    }))
    .await;
    ctx.expect_result(
        7_285,
        json!({ "body": "partial body", "base64Encoded": false }),
        Some("SID-1"),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_request_post_data_respects_recorded_session_visibility() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("SID-primary".to_owned());
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    assert!(bc.assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned()));
    bc.enable_auxiliary_network_events("SID-aux");
    bc.record_pending_response_body("REQ-aux-only".to_owned(), [Some("SID-aux".to_owned())]);
    bc.active_target
        .runtime_slot
        .record_captured_request_body_with_collector_scope(
            "REQ-aux-only".to_owned(),
            b"aux-only body".to_vec(),
            [Some("SID-aux".to_owned())],
            std::iter::empty::<String>(),
            false,
        );
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 7_288,
        "method": "Network.getRequestPostData",
        "sessionId": "SID-primary",
        "params": { "requestId": "REQ-aux-only" }
    }))
    .await;
    ctx.expect_error(7_288, -32000, "No resource with given id was found");

    ctx.process_async(json!({
        "id": 7_289,
        "method": "Network.getRequestPostData",
        "sessionId": "SID-aux",
        "params": { "requestId": "REQ-aux-only" }
    }))
    .await;
    ctx.expect_result(
        7_289,
        json!({ "postData": "aux-only body", "base64Encoded": false }),
        Some("SID-aux"),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_request_post_data_returns_main_document_navigation_post_body() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("SID-1".to_owned());
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    let requested_url = Url::parse("http://127.0.0.1:1/post").unwrap();
    let navigation_state = NavigationDispatchState {
        navigate_id: Some(1),
        navigate_session_id: Some("SID-1".to_owned()),
        result_projection: crate::conn::NavigationResultProjection::Cdp(
            json!({"frameId": "TID-1", "loaderId": LOADER_ID}),
        ),
        frame_id: "TID-1".to_owned(),
        session_id: Some("SID-1".to_owned()),
        request_id: Some("REQ-main-post".to_owned()),
        loader_id: LOADER_ID.to_owned(),
        request_announced: false,
        requested_url,
        request_method: "POST".to_owned(),
        request_body: Some("username=alice&pw=s3cret".to_owned()),
        request_body_bytes: Some(b"username=alice&pw=s3cret".to_vec()),
        request_headers: Vec::new(),
        request_load_policy: crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
        timestamp: 0.0,
        source_document_security: Default::default(),
    };

    let mut events = Vec::new();
    start_observed_main_document_navigation_progress_background_events(
        &mut ctx.conn,
        &mut events,
        &navigation_state,
        None,
    );

    ctx.process_async(json!({
        "id": 7_290,
        "method": "Network.getRequestPostData",
        "sessionId": "SID-1",
        "params": { "requestId": "REQ-main-post" }
    }))
    .await;
    ctx.expect_result(
        7_290,
        json!({ "postData": "username=alice&pw=s3cret", "base64Encoded": false }),
        Some("SID-1"),
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_network_data_returns_bidi_response_body_bytes() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("bidi-session-1".to_owned());
    bc.active_target
        .runtime_slot
        .enable_primary_network_events();
    ctx.conn.browser_context = Some(bc);

    let (result, _) = ctx
        .conn
        .execute_devtools_command(
            crate::devtools_runtime::DevToolsCommand::AddNetworkDataCollector(
                crate::devtools_runtime::DevToolsAddNetworkDataCollectorCommand {
                    context: bidi_network_context("bidi-session-1"),
                    collector_id: crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                        "collector-bidi-bytes",
                    ),
                    data_types: vec![
                        crate::devtools_runtime::DevToolsNetworkDataType::Response,
                        crate::devtools_runtime::DevToolsNetworkDataType::Request,
                    ],
                    max_encoded_data_size: 1000,
                    target_ids: Vec::new(),
                    browser_context_ids: Vec::new(),
                },
            ),
        )
        .await
        .into_parts();
    assert!(matches!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::AddNetworkDataCollector(_))
    ));

    let response_collector_ids = ctx.conn.network_data_collector_ids_for_session_owner_body(
        Some("bidi-session-1"),
        crate::devtools_runtime::DevToolsNetworkDataType::Response,
        "bidi body".len(),
    );
    ctx.conn
        .runtime_session_owner_slot_mut(Some("bidi-session-1"))
        .expect("bidi session runtime slot")
        .record_captured_response_body_source_with_collector_scope(
            "REQ-bidi-text".to_owned(),
            CapturedBody::from_string("bidi body".to_owned()),
            [Some("bidi-session-1".to_owned())],
            response_collector_ids,
            false,
        );

    let binary_response = CapturedBody::from_bytes(vec![0x00, 0xff]);
    let binary_response_collector_ids = ctx.conn.network_data_collector_ids_for_session_owner_body(
        Some("bidi-session-1"),
        crate::devtools_runtime::DevToolsNetworkDataType::Response,
        binary_response.len(),
    );
    ctx.conn
        .runtime_session_owner_slot_mut(Some("bidi-session-1"))
        .expect("bidi session runtime slot")
        .record_captured_response_body_source_with_collector_scope(
            "REQ-bidi-binary".to_owned(),
            binary_response,
            [Some("bidi-session-1".to_owned())],
            binary_response_collector_ids,
            false,
        );

    let primary_response_collector_ids =
        ctx.conn.network_data_collector_ids_for_session_owner_body(
            Some("bidi-session-1"),
            crate::devtools_runtime::DevToolsNetworkDataType::Response,
            "primary body".len(),
        );
    ctx.conn
        .runtime_session_owner_slot_mut(Some("bidi-session-1"))
        .expect("bidi session runtime slot")
        .record_captured_response_body_source_with_collector_scope(
            "REQ-bidi-primary".to_owned(),
            CapturedBody::from_string("primary body".to_owned()),
            [None::<String>],
            primary_response_collector_ids,
            false,
        );

    let request_collector_ids = ctx.conn.network_data_collector_ids_for_session_owner_body(
        Some("bidi-session-1"),
        crate::devtools_runtime::DevToolsNetworkDataType::Request,
        "bidi request body".len(),
    );
    ctx.conn
        .runtime_session_owner_slot_mut(Some("bidi-session-1"))
        .expect("bidi session runtime slot")
        .record_captured_request_body_with_collector_scope(
            "REQ-bidi-request".to_owned(),
            "bidi request body".as_bytes().to_vec(),
            [Some("bidi-session-1".to_owned())],
            request_collector_ids,
            false,
        );

    let binary_request = vec![0x00, 0xff, b'a'];
    let binary_request_collector_ids = ctx.conn.network_data_collector_ids_for_session_owner_body(
        Some("bidi-session-1"),
        crate::devtools_runtime::DevToolsNetworkDataType::Request,
        binary_request.len(),
    );
    ctx.conn
        .runtime_session_owner_slot_mut(Some("bidi-session-1"))
        .expect("bidi session runtime slot")
        .record_captured_request_body_with_collector_scope(
            "REQ-bidi-request-binary".to_owned(),
            binary_request,
            [Some("bidi-session-1".to_owned())],
            binary_request_collector_ids,
            false,
        );

    let (result, scheduler_events) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: crate::devtools_runtime::DevToolsCommandContext {
                    protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
                    session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(
                        "bidi-session-1",
                    )),
                    target_id: None,
                    browser_context_id: None,
                },
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-bidi-text"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: None,
                disown: false,
            },
        ))
        .await
        .into_parts();

    assert!(scheduler_events.is_empty());
    assert_eq!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::NetworkData(
            crate::devtools_runtime::DevToolsNetworkDataResult {
                bytes_type: crate::devtools_runtime::DevToolsNetworkDataBytesType::String,
                value: "bidi body".to_owned(),
            },
        ))
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: crate::devtools_runtime::DevToolsCommandContext {
                    protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
                    session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(
                        "bidi-session-1",
                    )),
                    target_id: None,
                    browser_context_id: None,
                },
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-bidi-request"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Request,
                collector: None,
                disown: false,
            },
        ))
        .await
        .into_parts();

    assert_eq!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::NetworkData(
            crate::devtools_runtime::DevToolsNetworkDataResult {
                bytes_type: crate::devtools_runtime::DevToolsNetworkDataBytesType::String,
                value: "bidi request body".to_owned(),
            },
        ))
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: crate::devtools_runtime::DevToolsCommandContext {
                    protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
                    session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(
                        "bidi-session-1",
                    )),
                    target_id: None,
                    browser_context_id: None,
                },
                request_id: crate::devtools_runtime::DevToolsRequestId::from(
                    "REQ-bidi-request-binary",
                ),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Request,
                collector: None,
                disown: false,
            },
        ))
        .await
        .into_parts();

    assert_eq!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::NetworkData(
            crate::devtools_runtime::DevToolsNetworkDataResult {
                bytes_type: crate::devtools_runtime::DevToolsNetworkDataBytesType::Base64,
                value: "AP9h".to_owned(),
            },
        ))
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: crate::devtools_runtime::DevToolsCommandContext {
                    protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
                    session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(
                        "bidi-session-1",
                    )),
                    target_id: None,
                    browser_context_id: None,
                },
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-bidi-binary"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: None,
                disown: false,
            },
        ))
        .await
        .into_parts();

    assert_eq!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::NetworkData(
            crate::devtools_runtime::DevToolsNetworkDataResult {
                bytes_type: crate::devtools_runtime::DevToolsNetworkDataBytesType::Base64,
                value: "AP8=".to_owned(),
            },
        ))
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: crate::devtools_runtime::DevToolsCommandContext {
                    protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
                    session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(
                        "bidi-session-1",
                    )),
                    target_id: None,
                    browser_context_id: None,
                },
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-bidi-primary"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: None,
                disown: false,
            },
        ))
        .await
        .into_parts();

    assert_eq!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::NetworkData(
            crate::devtools_runtime::DevToolsNetworkDataResult {
                bytes_type: crate::devtools_runtime::DevToolsNetworkDataBytesType::String,
                value: "primary body".to_owned(),
            },
        ))
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: crate::devtools_runtime::DevToolsCommandContext {
                    protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
                    session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(
                        "bidi-session-2",
                    )),
                    target_id: None,
                    browser_context_id: None,
                },
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-bidi-text"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: None,
                disown: false,
            },
        ))
        .await
        .into_parts();

    assert_eq!(
        result,
        Err(crate::devtools_runtime::DevToolsError::new(
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
            "no such network data",
        ))
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: crate::devtools_runtime::DevToolsCommandContext {
                    protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
                    session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(
                        "bidi-session-2",
                    )),
                    target_id: None,
                    browser_context_id: None,
                },
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-bidi-request"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Request,
                collector: None,
                disown: false,
            },
        ))
        .await
        .into_parts();

    assert_eq!(
        result,
        Err(crate::devtools_runtime::DevToolsError::new(
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
            "no such network data",
        ))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn network_data_collectors_gate_get_data_disown_and_remove() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("bidi-session-1".to_owned());
    bc.record_captured_response_body(
        "REQ-before-collector".to_owned(),
        "pre collector body".to_owned(),
        [Some("bidi-session-1".to_owned())],
    );
    ctx.conn.browser_context = Some(bc);

    for (collector, max_encoded_data_size) in [
        ("collector-ok", 1000),
        ("collector-disown-command", 1000),
        ("collector-small", 1),
    ] {
        let (result, _) = ctx
            .conn
            .execute_devtools_command(
                crate::devtools_runtime::DevToolsCommand::AddNetworkDataCollector(
                    crate::devtools_runtime::DevToolsAddNetworkDataCollectorCommand {
                        context: bidi_network_context("bidi-session-1"),
                        collector_id: crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                            collector,
                        ),
                        data_types: vec![
                            crate::devtools_runtime::DevToolsNetworkDataType::Response,
                        ],
                        max_encoded_data_size,
                        target_ids: Vec::new(),
                        browser_context_ids: Vec::new(),
                    },
                ),
            )
            .await
            .into_parts();
        assert_eq!(
            result,
            Ok(
                crate::devtools_runtime::DevToolsCommandResult::AddNetworkDataCollector(
                    crate::devtools_runtime::DevToolsAddNetworkDataCollectorResult {
                        collector_id: crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                            collector,
                        ),
                    },
                )
            )
        );
    }

    let body_text = "collector body";
    let collector_ids = ctx.conn.network_data_collector_ids_for_session_owner_body(
        Some("bidi-session-1"),
        crate::devtools_runtime::DevToolsNetworkDataType::Response,
        body_text.len(),
    );
    ctx.conn
        .runtime_session_owner_slot_mut(Some("bidi-session-1"))
        .expect("bidi session runtime slot")
        .record_captured_response_body_source_with_collector_scope(
            "REQ-collected".to_owned(),
            CapturedBody::from_string(body_text.to_owned()),
            [Some("bidi-session-1".to_owned())],
            collector_ids,
            false,
        );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from(
                    "REQ-before-collector",
                ),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: Some(
                    crate::devtools_runtime::DevToolsNetworkDataCollectorId::from("collector-ok"),
                ),
                disown: false,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Err(crate::devtools_runtime::DevToolsError::new(
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
            "no such network data",
        ))
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-collected"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: Some(
                    crate::devtools_runtime::DevToolsNetworkDataCollectorId::from("collector-ok"),
                ),
                disown: false,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::NetworkData(
            crate::devtools_runtime::DevToolsNetworkDataResult {
                bytes_type: crate::devtools_runtime::DevToolsNetworkDataBytesType::String,
                value: "collector body".to_owned(),
            },
        ))
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-collected"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: Some(
                    crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                        "collector-small",
                    ),
                ),
                disown: false,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Err(crate::devtools_runtime::DevToolsError::new(
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
            "no such network data",
        ))
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::DisownNetworkData(
            crate::devtools_runtime::DevToolsDisownNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-collected"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector_id: crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                    "collector-disown-command",
                ),
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::Empty)
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-collected"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: Some(
                    crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                        "collector-disown-command",
                    ),
                ),
                disown: false,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Err(crate::devtools_runtime::DevToolsError::new(
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
            "no such network data",
        ))
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-collected"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: None,
                disown: false,
            },
        ))
        .await
        .into_parts();
    assert!(matches!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::NetworkData(
            _
        ))
    ));

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-collected"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: Some(
                    crate::devtools_runtime::DevToolsNetworkDataCollectorId::from("collector-ok"),
                ),
                disown: true,
            },
        ))
        .await
        .into_parts();
    assert!(matches!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::NetworkData(
            _
        ))
    ));

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-collected"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: Some(
                    crate::devtools_runtime::DevToolsNetworkDataCollectorId::from("collector-ok"),
                ),
                disown: false,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Err(crate::devtools_runtime::DevToolsError::new(
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
            "no such network data",
        ))
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-collected"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: None,
                disown: false,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Err(crate::devtools_runtime::DevToolsError::new(
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
            "no such network data",
        ))
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(
            crate::devtools_runtime::DevToolsCommand::RemoveNetworkDataCollector(
                crate::devtools_runtime::DevToolsRemoveNetworkDataCollectorCommand {
                    context: bidi_network_context("bidi-session-1"),
                    collector_id: crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                        "collector-small",
                    ),
                },
            ),
        )
        .await
        .into_parts();
    assert_eq!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::Empty)
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(
            crate::devtools_runtime::DevToolsCommand::RemoveNetworkDataCollector(
                crate::devtools_runtime::DevToolsRemoveNetworkDataCollectorCommand {
                    context: bidi_network_context("bidi-session-1"),
                    collector_id: crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                        "collector-small",
                    ),
                },
            ),
        )
        .await
        .into_parts();
    assert_eq!(
        result,
        Err(crate::devtools_runtime::DevToolsError::new(
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkCollector,
            "no such network collector",
        ))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn network_data_collector_body_persists_after_target_artifact_cleanup() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("bidi-session-1".to_owned());
    ctx.conn.browser_context = Some(bc);

    let collector_id =
        crate::devtools_runtime::DevToolsNetworkDataCollectorId::from("collector-persist");
    let (result, _) = ctx
        .conn
        .execute_devtools_command(
            crate::devtools_runtime::DevToolsCommand::AddNetworkDataCollector(
                crate::devtools_runtime::DevToolsAddNetworkDataCollectorCommand {
                    context: bidi_network_context("bidi-session-1"),
                    collector_id: collector_id.clone(),
                    data_types: vec![crate::devtools_runtime::DevToolsNetworkDataType::Response],
                    max_encoded_data_size: 1000,
                    target_ids: Vec::new(),
                    browser_context_ids: Vec::new(),
                },
            ),
        )
        .await
        .into_parts();
    assert!(matches!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::AddNetworkDataCollector(_))
    ));

    let request_id = "REQ-persist";
    let data_type = crate::devtools_runtime::DevToolsNetworkDataType::Response;
    let body = CapturedBody::from_string("persistent collector body".to_owned());
    let collector_ids = ctx.conn.network_data_collector_ids_for_session_owner_body(
        Some("bidi-session-1"),
        data_type,
        body.len(),
    );
    ctx.conn.record_collected_network_data_body(
        request_id.to_owned(),
        data_type,
        body.clone(),
        collector_ids.iter().cloned(),
        false,
    );
    ctx.conn
        .runtime_session_owner_slot_mut(Some("bidi-session-1"))
        .expect("bidi session runtime slot")
        .record_captured_response_body_source_with_collector_scope(
            request_id.to_owned(),
            body,
            [Some("bidi-session-1".to_owned())],
            collector_ids,
            false,
        );

    fn get_collected_command(
        request_id: &str,
        data_type: crate::devtools_runtime::DevToolsNetworkDataType,
        collector_id: crate::devtools_runtime::DevToolsNetworkDataCollectorId,
        disown: bool,
    ) -> crate::devtools_runtime::DevToolsCommand {
        crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from(request_id),
                data_type,
                collector: Some(collector_id),
                disown,
            },
        )
    }

    let (result, _) = ctx
        .conn
        .execute_devtools_command(get_collected_command(
            request_id,
            data_type,
            collector_id.clone(),
            false,
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::NetworkData(
            crate::devtools_runtime::DevToolsNetworkDataResult {
                bytes_type: crate::devtools_runtime::DevToolsNetworkDataBytesType::String,
                value: "persistent collector body".to_owned(),
            },
        ))
    );

    ctx.conn
        .runtime_session_owner_slot_mut(Some("bidi-session-1"))
        .expect("bidi session runtime slot")
        .clear_network_body_artifacts();

    let (result, _) = ctx
        .conn
        .execute_devtools_command(get_collected_command(
            request_id,
            data_type,
            collector_id.clone(),
            false,
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::NetworkData(
            crate::devtools_runtime::DevToolsNetworkDataResult {
                bytes_type: crate::devtools_runtime::DevToolsNetworkDataBytesType::String,
                value: "persistent collector body".to_owned(),
            },
        ))
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(get_collected_command(
            request_id,
            data_type,
            collector_id.clone(),
            true,
        ))
        .await
        .into_parts();
    assert!(matches!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::NetworkData(
            _
        ))
    ));

    let (result, _) = ctx
        .conn
        .execute_devtools_command(get_collected_command(
            request_id,
            data_type,
            collector_id,
            false,
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Err(crate::devtools_runtime::DevToolsError::new(
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
            "no such network data",
        ))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn network_data_explicit_collector_prefers_collected_body_over_stale_target_artifact() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("bidi-session-1".to_owned());
    ctx.conn.browser_context = Some(bc);

    let collector_id =
        crate::devtools_runtime::DevToolsNetworkDataCollectorId::from("collector-shadow");
    let data_type = crate::devtools_runtime::DevToolsNetworkDataType::Response;
    let (result, _) = ctx
        .conn
        .execute_devtools_command(
            crate::devtools_runtime::DevToolsCommand::AddNetworkDataCollector(
                crate::devtools_runtime::DevToolsAddNetworkDataCollectorCommand {
                    context: bidi_network_context("bidi-session-1"),
                    collector_id: collector_id.clone(),
                    data_types: vec![data_type],
                    max_encoded_data_size: 1000,
                    target_ids: Vec::new(),
                    browser_context_ids: Vec::new(),
                },
            ),
        )
        .await
        .into_parts();
    assert!(matches!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::AddNetworkDataCollector(_))
    ));

    let request_id = "REQ-shadow";
    let collected_body = CapturedBody::from_string("collector-owned body".to_owned());
    let collector_ids = ctx.conn.network_data_collector_ids_for_session_owner_body(
        Some("bidi-session-1"),
        data_type,
        collected_body.len(),
    );
    ctx.conn.record_collected_network_data_body(
        request_id.to_owned(),
        data_type,
        collected_body,
        collector_ids.iter().cloned(),
        false,
    );

    ctx.conn
        .runtime_session_owner_slot_mut(Some("bidi-session-1"))
        .expect("bidi session runtime slot")
        .record_captured_response_body_source_with_collector_scope(
            request_id.to_owned(),
            CapturedBody::from_string("stale target artifact".to_owned()),
            [Some("bidi-session-1".to_owned())],
            std::iter::empty::<String>(),
            false,
        );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from(request_id),
                data_type,
                collector: Some(collector_id),
                disown: false,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::NetworkData(
            crate::devtools_runtime::DevToolsNetworkDataResult {
                bytes_type: crate::devtools_runtime::DevToolsNetworkDataBytesType::String,
                value: "collector-owned body".to_owned(),
            },
        ))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn network_data_explicit_collector_rejects_unconfigured_data_type() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("bidi-session-1".to_owned());
    ctx.conn.browser_context = Some(bc);

    let collector_id =
        crate::devtools_runtime::DevToolsNetworkDataCollectorId::from("collector-request-only");
    let (result, _) = ctx
        .conn
        .execute_devtools_command(
            crate::devtools_runtime::DevToolsCommand::AddNetworkDataCollector(
                crate::devtools_runtime::DevToolsAddNetworkDataCollectorCommand {
                    context: bidi_network_context("bidi-session-1"),
                    collector_id: collector_id.clone(),
                    data_types: vec![crate::devtools_runtime::DevToolsNetworkDataType::Request],
                    max_encoded_data_size: 1000,
                    target_ids: Vec::new(),
                    browser_context_ids: Vec::new(),
                },
            ),
        )
        .await
        .into_parts();
    assert!(matches!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::AddNetworkDataCollector(_))
    ));

    ctx.conn
        .runtime_session_owner_slot_mut(Some("bidi-session-1"))
        .expect("bidi session runtime slot")
        .record_captured_response_body_source_with_collector_scope(
            "REQ-type-mismatch".to_owned(),
            CapturedBody::from_string("response body".to_owned()),
            [Some("bidi-session-1".to_owned())],
            [collector_id.as_str().to_owned()],
            true,
        );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-type-mismatch"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: Some(collector_id),
                disown: false,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Err(crate::devtools_runtime::DevToolsError::new(
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
            "no such network data",
        ))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn network_data_without_collector_requires_matching_collected_data_type() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1".to_owned());
    bc.attach_active_session("bidi-session-1".to_owned());
    ctx.conn.browser_context = Some(bc);

    let (result, _) = ctx
        .conn
        .execute_devtools_command(
            crate::devtools_runtime::DevToolsCommand::AddNetworkDataCollector(
                crate::devtools_runtime::DevToolsAddNetworkDataCollectorCommand {
                    context: bidi_network_context("bidi-session-1"),
                    collector_id: crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                        "collector-request-only",
                    ),
                    data_types: vec![crate::devtools_runtime::DevToolsNetworkDataType::Request],
                    max_encoded_data_size: 1000,
                    target_ids: Vec::new(),
                    browser_context_ids: Vec::new(),
                },
            ),
        )
        .await
        .into_parts();
    assert!(matches!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::AddNetworkDataCollector(_))
    ));

    let data_type = crate::devtools_runtime::DevToolsNetworkDataType::Response;
    assert!(
        !ctx.conn
            .network_data_collection_is_gated_for_body(data_type),
        "request-only collectors must not gate response bodies"
    );
    ctx.conn
        .runtime_session_owner_slot_mut(Some("bidi-session-1"))
        .expect("bidi session runtime slot")
        .record_captured_response_body_source_with_collector_scope(
            "REQ-response-not-collected".to_owned(),
            CapturedBody::from_string("response body".to_owned()),
            [Some("bidi-session-1".to_owned())],
            std::iter::empty::<String>(),
            false,
        );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from(
                    "REQ-response-not-collected",
                ),
                data_type,
                collector: None,
                disown: false,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Err(crate::devtools_runtime::DevToolsError::new(
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
            "no such network data",
        ))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn network_data_collector_membership_uses_recorded_target_scope() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("bidi-session-1".to_owned());
    bc.background_targets.push(BackgroundTarget::with_url(
        "TID-other".to_owned(),
        None,
        "about:blank".to_owned(),
    ));
    ctx.conn.browser_context = Some(bc);

    for (collector, target_id) in [
        ("collector-active", "TID-active"),
        ("collector-other", "TID-other"),
    ] {
        let (result, _) = ctx
            .conn
            .execute_devtools_command(
                crate::devtools_runtime::DevToolsCommand::AddNetworkDataCollector(
                    crate::devtools_runtime::DevToolsAddNetworkDataCollectorCommand {
                        context: bidi_network_context("bidi-session-1"),
                        collector_id: crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                            collector,
                        ),
                        data_types: vec![
                            crate::devtools_runtime::DevToolsNetworkDataType::Response,
                        ],
                        max_encoded_data_size: 1000,
                        target_ids: vec![crate::devtools_runtime::DevToolsTargetId::from(
                            target_id,
                        )],
                        browser_context_ids: Vec::new(),
                    },
                ),
            )
            .await
            .into_parts();
        assert_eq!(
            result,
            Ok(
                crate::devtools_runtime::DevToolsCommandResult::AddNetworkDataCollector(
                    crate::devtools_runtime::DevToolsAddNetworkDataCollectorResult {
                        collector_id: crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                            collector,
                        ),
                    },
                )
            )
        );
    }

    let body_text = "scoped body";
    let collector_ids = ctx.conn.network_data_collector_ids_for_session_owner_body(
        Some("bidi-session-1"),
        crate::devtools_runtime::DevToolsNetworkDataType::Response,
        body_text.len(),
    );
    ctx.conn
        .runtime_session_owner_slot_mut(Some("bidi-session-1"))
        .expect("bidi session runtime slot")
        .record_captured_response_body_source_with_collector_scope(
            "REQ-scoped".to_owned(),
            CapturedBody::from_string(body_text.to_owned()),
            [Some("bidi-session-1".to_owned())],
            collector_ids,
            false,
        );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-scoped"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: Some(
                    crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                        "collector-active",
                    ),
                ),
                disown: false,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::NetworkData(
            crate::devtools_runtime::DevToolsNetworkDataResult {
                bytes_type: crate::devtools_runtime::DevToolsNetworkDataBytesType::String,
                value: body_text.to_owned(),
            },
        ))
    );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-scoped"),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Response,
                collector: Some(
                    crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                        "collector-other",
                    ),
                ),
                disown: false,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Err(crate::devtools_runtime::DevToolsError::new(
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
            "no such network data",
        ))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn network_data_collector_gated_body_without_match_is_not_readable() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("bidi-session-1".to_owned());
    bc.background_targets.push(BackgroundTarget::with_url(
        "TID-other".to_owned(),
        None,
        "about:blank".to_owned(),
    ));
    ctx.conn.browser_context = Some(bc);

    let (result, _) = ctx
        .conn
        .execute_devtools_command(
            crate::devtools_runtime::DevToolsCommand::AddNetworkDataCollector(
                crate::devtools_runtime::DevToolsAddNetworkDataCollectorCommand {
                    context: bidi_network_context("bidi-session-1"),
                    collector_id: crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                        "collector-other",
                    ),
                    data_types: vec![crate::devtools_runtime::DevToolsNetworkDataType::Response],
                    max_encoded_data_size: 1000,
                    target_ids: vec![crate::devtools_runtime::DevToolsTargetId::from("TID-other")],
                    browser_context_ids: Vec::new(),
                },
            ),
        )
        .await
        .into_parts();
    assert!(matches!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::AddNetworkDataCollector(_))
    ));

    let body_text = "active body";
    let data_type = crate::devtools_runtime::DevToolsNetworkDataType::Response;
    let collection_was_gated = ctx
        .conn
        .network_data_collection_is_gated_for_body(data_type);
    let collector_ids = ctx.conn.network_data_collector_ids_for_session_owner_body(
        Some("bidi-session-1"),
        data_type,
        body_text.len(),
    );
    assert!(collection_was_gated);
    assert!(
        collector_ids.is_empty(),
        "active target should not match the collector scoped to TID-other"
    );

    ctx.conn
        .runtime_session_owner_slot_mut(Some("bidi-session-1"))
        .expect("bidi session runtime slot")
        .record_captured_response_body_source_with_collector_scope(
            "REQ-unmatched".to_owned(),
            CapturedBody::from_string(body_text.to_owned()),
            [Some("bidi-session-1".to_owned())],
            collector_ids,
            collection_was_gated,
        );

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("bidi-session-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from("REQ-unmatched"),
                data_type,
                collector: None,
                disown: false,
            },
        ))
        .await
        .into_parts();
    assert_eq!(
        result,
        Err(crate::devtools_runtime::DevToolsError::new(
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
            "no such network data",
        ))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn get_network_data_reports_unimplemented_or_missing_data_with_bidi_errors() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.record_pending_response_body("REQ-pending".to_owned(), [None::<String>]);
    ctx.conn.browser_context = Some(bc);

    for (request_id, data_type, collector, expected_kind) in [
        (
            "REQ-pending",
            crate::devtools_runtime::DevToolsNetworkDataType::Response,
            None,
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
        ),
        (
            "REQ-missing",
            crate::devtools_runtime::DevToolsNetworkDataType::Response,
            None,
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
        ),
        (
            "REQ-pending",
            crate::devtools_runtime::DevToolsNetworkDataType::Request,
            None,
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkData,
        ),
        (
            "REQ-pending",
            crate::devtools_runtime::DevToolsNetworkDataType::Response,
            Some("collector-1"),
            crate::devtools_runtime::DevToolsErrorKind::NoSuchNetworkCollector,
        ),
    ] {
        let (result, _) = ctx
            .conn
            .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
                crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                    context: crate::devtools_runtime::DevToolsCommandContext {
                        protocol: crate::devtools_runtime::DevToolsProtocol::WebDriverBidi,
                        session_id: Some(crate::devtools_runtime::DevToolsSessionId::from(
                            "bidi-session-1",
                        )),
                        target_id: None,
                        browser_context_id: None,
                    },
                    request_id: crate::devtools_runtime::DevToolsRequestId::from(request_id),
                    data_type,
                    collector: collector
                        .map(crate::devtools_runtime::DevToolsNetworkDataCollectorId::from),
                    disown: false,
                },
            ))
            .await
            .into_parts();
        let error = result.expect_err("network.getData should fail");
        assert_eq!(error.kind, expected_kind);
    }
}
#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_rejects_invalid_params() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 3,
        "method": "Network.getResponseBody",
        "params": {}
    }))
    .await;
    ctx.expect_error(3, -32602, "InvalidParams");
}
#[tokio::test(flavor = "multi_thread")]
async fn get_response_body_requires_browser_context() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({
        "id": 3_1,
        "method": "Network.getResponseBody",
        "params": { "requestId": "REQ-1" }
    }))
    .await;
    ctx.expect_error(3_1, -31998, "BrowserContextNotLoaded");
}
#[tokio::test(flavor = "multi_thread")]
async fn main_document_navigation_get_response_body_preserves_binary_bytes() {
    async fn handler() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            vec![0x00_u8, 0xff, b'a'],
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/page", get(handler)))
            .await
            .unwrap();
    });

    let url = format!("http://{addr}/page");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 7_284,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7_284, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 7_285,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": url }
    }))
    .await;

    wait_until_messages(
        &mut ctx,
        Some("SID-1"),
        "binary main document navigation finished",
        |messages| {
            messages.iter().any(|message| {
                message["method"] == json!("Network.loadingFinished")
                    && message["params"]["requestId"].as_str().is_some()
            })
        },
    )
    .await;

    ctx.process_async(json!({
        "id": 7_286,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": LOADER_ID }
    }))
    .await;
    ctx.expect_result(
        7_286,
        json!({ "body": "AP9h", "base64Encoded": true }),
        Some("SID-1"),
    );

    server.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_get_response_body_preserves_binary_bytes() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
fetch('/binary')
  .then(response => response.bytes())
  .then(bytes => {
    document.body.setAttribute('data-bytes', Array.from(bytes).join(','));
  });
</script>
</body></html>"#,
        )
    }

    async fn binary() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "application/octet-stream")],
            vec![0x00_u8, 0xff, b'a'],
        )
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/binary", get(binary)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let binary_url = format!("http://{addr}/binary");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 7_287,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7_287, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 7_288,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(&mut ctx, "Fetch", 1, "binary page fetch network completion")
        .await;

    let messages = ctx.take_all();
    let fetch_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
                && message["params"]["request"]["url"] == json!(binary_url)
        })
        .expect("binary fetch request event");
    let fetch_request_id = fetch_request["params"]["requestId"]
        .as_str()
        .expect("binary fetch request id")
        .to_owned();
    let loading_finished = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.loadingFinished")
                && message["params"]["requestId"] == json!(fetch_request_id)
        })
        .expect("binary fetch loadingFinished event");
    assert_eq!(loading_finished["params"]["encodedDataLength"], json!(3));

    ctx.process_async(json!({
        "id": 7_289,
        "method": "Network.getResponseBody",
        "sessionId": "SID-1",
        "params": { "requestId": fetch_request_id }
    }))
    .await;
    ctx.expect_result(
        7_289,
        json!({ "body": "AP9h", "base64Encoded": true }),
        Some("SID-1"),
    );

    let mut observed = None;
    for poll_id in 7_292..7_312 {
        ctx.process_async(json!({
            "id": poll_id,
            "method": "Runtime.evaluate",
            "sessionId": "SID-1",
            "params": {
                "expression": "document.body.getAttribute('data-bytes') || 'pending'"
            }
        }))
        .await;
        let result = ctx.take_response_by_id(poll_id);
        let value = result["result"]["result"]["value"]
            .as_str()
            .unwrap_or_default();
        if value != "pending" {
            observed = Some(value.to_owned());
            break;
        }
        ctx.complete_one_ready_scheduler_input_for_test().await;
        sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(observed.as_deref(), Some("0,255,97"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn page_fetch_get_network_data_request_preserves_staged_binary_body_bytes() {
    async fn page() -> impl IntoResponse {
        (
            [(CONTENT_TYPE.as_str(), "text/html")],
            r#"<!doctype html>
<html><body>
<script>
const formData = new FormData();
const bytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0xff]);
formData.append("test_image", new Blob([bytes], {type: "image/png"}), "image.png");
fetch("/upload", {method: "POST", body: formData})
  .then(response => response.text())
  .then(text => {
    document.body.setAttribute("data-upload", text);
  });
</script>
</body></html>"#,
        )
    }

    async fn upload() -> impl IntoResponse {
        "uploaded"
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/page", get(page))
                .route("/upload", post(upload)),
        )
        .await
        .unwrap();
    });

    let page_url = format!("http://{addr}/page");
    let upload_url = format!("http://{addr}/upload");
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.set_active_target_id("TID-1");
    bc.attach_active_session("SID-1");
    ctx.conn.browser_context = Some(bc);

    let (result, _) = ctx
        .conn
        .execute_devtools_command(
            crate::devtools_runtime::DevToolsCommand::AddNetworkDataCollector(
                crate::devtools_runtime::DevToolsAddNetworkDataCollectorCommand {
                    context: bidi_network_context("SID-1"),
                    collector_id: crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                        "collector-request-body",
                    ),
                    data_types: vec![crate::devtools_runtime::DevToolsNetworkDataType::Request],
                    max_encoded_data_size: 4096,
                    target_ids: Vec::new(),
                    browser_context_ids: Vec::new(),
                },
            ),
        )
        .await
        .into_parts();
    assert!(matches!(
        result,
        Ok(crate::devtools_runtime::DevToolsCommandResult::AddNetworkDataCollector(_))
    ));

    ctx.process_async(json!({
        "id": 7_293,
        "method": "Network.enable",
        "sessionId": "SID-1"
    }))
    .await;
    ctx.expect_result(7_293, json!({}), Some("SID-1"));

    ctx.process_async(json!({
        "id": 7_294,
        "method": "Page.navigate",
        "sessionId": "SID-1",
        "params": { "url": page_url }
    }))
    .await;

    flush_until_subresource_finished(&mut ctx, "Fetch", 1, "multipart upload fetch completion")
        .await;

    let messages = ctx.take_all();
    let upload_request = messages
        .iter()
        .find(|message| {
            message["method"] == json!("Network.requestWillBeSent")
                && message["params"]["type"] == json!("Fetch")
                && message["params"]["request"]["url"] == json!(upload_url)
        })
        .expect("multipart upload request event");
    let request_id = upload_request["params"]["requestId"]
        .as_str()
        .expect("multipart upload request id")
        .to_owned();

    let (result, _) = ctx
        .conn
        .execute_devtools_command(crate::devtools_runtime::DevToolsCommand::GetNetworkData(
            crate::devtools_runtime::DevToolsGetNetworkDataCommand {
                context: bidi_network_context("SID-1"),
                request_id: crate::devtools_runtime::DevToolsRequestId::from(request_id),
                data_type: crate::devtools_runtime::DevToolsNetworkDataType::Request,
                collector: Some(
                    crate::devtools_runtime::DevToolsNetworkDataCollectorId::from(
                        "collector-request-body",
                    ),
                ),
                disown: false,
            },
        ))
        .await
        .into_parts();

    let result = result.expect("multipart request body should be collected");
    let crate::devtools_runtime::DevToolsCommandResult::NetworkData(data) = result else {
        panic!("expected network data result");
    };
    assert_eq!(
        data.bytes_type,
        crate::devtools_runtime::DevToolsNetworkDataBytesType::Base64
    );
    let decoded = BASE64_STANDARD
        .decode(data.value)
        .expect("multipart request body should be base64");
    assert!(
        decoded
            .windows(5)
            .any(|window| window == [0x89, 0x50, 0x4e, 0x47, 0xff]),
        "multipart request body should preserve raw image bytes: {decoded:?}"
    );

    server.abort();
}
