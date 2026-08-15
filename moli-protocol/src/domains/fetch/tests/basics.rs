use super::*;
use crate::conn::{
    CdpCommandTaskStep, FetchInterceptionPattern, PendingSubresourceFetchAuthRequest,
    PendingSubresourceFetchOwnerKind, PendingSubresourceFetchRequest,
};
use crate::devtools_runtime::{
    AutomationEvent, DevToolsNetworkInterceptId, DevToolsNetworkResourceType,
};
use crate::domains::fetch::pending_subresource_auth_required_event;
use moli_core::page::SubresourceResourceType;

fn pending_subresource_fetch(
    internal_id: u64,
    owner_session_id: &str,
) -> PendingSubresourceFetchRequest {
    pending_subresource_fetch_with_owner_kind(
        internal_id,
        owner_session_id,
        PendingSubresourceFetchOwnerKind::Fetch,
    )
}

fn pending_subresource_fetch_with_owner_kind(
    internal_id: u64,
    owner_session_id: &str,
    owner_kind: PendingSubresourceFetchOwnerKind,
) -> PendingSubresourceFetchRequest {
    PendingSubresourceFetchRequest {
        residence: crate::conn::PendingSubresourceFetchResidence::InstalledPage(
            crate::conn::TargetPageResidenceIdentity::new_for_test(
                "BID-session-fetch".to_owned(),
                Some("TID-session-fetch".to_owned()),
                1,
            ),
        ),
        owner_session_id: Some(owner_session_id.to_owned()),
        action_session_id: Some(owner_session_id.to_owned()),
        owner_kind,
        internal_id,
        network_request_id: format!("NETWORK-{internal_id}"),
        network_request_handle: None,
        frame_id: "TID-session-fetch".to_owned(),
        document_url: Url::parse("https://example.test/page").unwrap(),
        resource_type: SubresourceResourceType::Fetch,
        websocket_socket_id: None,
        request_stage_chain: None,
    }
}

fn pending_subresource_fetch_auth(
    internal_id: u64,
    owner_session_id: &str,
) -> PendingSubresourceFetchAuthRequest {
    PendingSubresourceFetchAuthRequest {
        page_owner: crate::conn::TargetPageResidenceIdentity::new_for_test(
            "BID-session-fetch".to_owned(),
            Some("TID-session-fetch".to_owned()),
            1,
        ),
        owner_session_id: Some(owner_session_id.to_owned()),
        action_session_id: Some(owner_session_id.to_owned()),
        owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
        internal_id,
        network_request_id: format!("NETWORK-{internal_id}"),
        network_request_handle: None,
        frame_id: "TID-session-fetch".to_owned(),
        document_url: Url::parse("https://example.test/page").unwrap(),
        resource_type: SubresourceResourceType::Fetch,
        websocket_socket_id: None,
        url: Url::parse("https://example.test/protected.json").unwrap(),
        method: "GET".to_owned(),
        request_headers: vec![("accept".to_owned(), "application/json".to_owned())],
        request_body: None,
        request_cookie_report: None,
        challenge: FetchAuthChallenge {
            origin: "http://example.test".to_owned(),
            source: "Server".to_owned(),
            scheme: "basic".to_owned(),
            realm: "private".to_owned(),
        },
        intercept_response: false,
        auth_stage_chain: None,
    }
}

#[test]
fn pending_subresource_auth_required_event_carries_typed_sidecar() {
    let pending = pending_subresource_fetch_auth(7, "SID-primary");
    let event = pending_subresource_auth_required_event(
        Some("SID-aux"),
        "FETCH-7",
        &pending,
        &[DevToolsNetworkInterceptId::from("intercept-auth")],
    );

    let (message, sidecar) = event.into_parts();

    assert_eq!(message["method"], "Fetch.authRequired");
    assert_eq!(message["sessionId"], "SID-aux");
    assert_eq!(message["params"]["requestId"], "FETCH-7");
    assert!(message["params"].get("networkId").is_none());
    assert_eq!(message["params"]["authChallenge"]["realm"], "private");
    let Some(AutomationEvent::NetworkAuthRequired(sidecar)) = sidecar else {
        panic!("authRequired should carry typed network sidecar");
    };
    assert_eq!(sidecar.request_id.as_str(), "FETCH-7");
    assert_eq!(
        sidecar.network_id.as_ref().map(|id| id.as_str()),
        Some("NETWORK-7")
    );
    assert_eq!(
        sidecar.resource_type,
        Some(DevToolsNetworkResourceType::Xhr)
    );
    assert_eq!(
        sidecar.blocked_intercepts,
        vec![DevToolsNetworkInterceptId::from("intercept-auth")]
    );
}

#[tokio::test]
async fn enable_without_browser_context_errors() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 1, "method": "Fetch.enable"}))
        .await;
    ctx.expect_error(1, -31998, "BrowserContextNotLoaded");
}

#[tokio::test]
async fn enable_sets_fetch_flags_for_supported_patterns() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 2,
        "method": "Fetch.enable",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request" }],
            "handleAuthRequests": true
        }
    }))
    .await;
    ctx.expect_result(2, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.fetch_owner.is_enabled());
    assert!(bc.active_target.fetch_owner.handle_auth_requests());
}

#[tokio::test]
async fn enable_and_disable_are_session_local_for_same_target() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-session-fetch".into());
    bc.set_active_target_id("TID-session-fetch".to_owned());
    bc.attach_active_session("SID-primary".to_owned());
    assert!(bc.assign_auxiliary_session_to_target("TID-session-fetch", "SID-aux".to_owned()));
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 210,
        "method": "Fetch.enable",
        "sessionId": "SID-primary",
        "params": {
            "patterns": [{ "urlPattern": "*primary*", "requestStage": "Request", "resourceType": "Fetch" }],
            "handleAuthRequests": true
        }
    }))
    .await;
    ctx.expect_result(210, json!({}), Some("SID-primary"));

    ctx.process_async(json!({
        "id": 211,
        "method": "Fetch.enable",
        "sessionId": "SID-aux",
        "params": {
            "patterns": [{ "urlPattern": "*aux*", "requestStage": "Response", "resourceType": "XHR" }]
        }
    }))
    .await;
    ctx.expect_result(211, json!({}), Some("SID-aux"));

    {
        let bc = ctx.conn.browser_context.as_ref().expect("browser context");
        let aggregate = bc.active_target.fetch_owner.config_snapshot();
        assert!(aggregate.is_enabled());
        assert!(aggregate.handle_auth_requests());
        assert_eq!(aggregate.patterns().len(), 2);

        let primary = bc
            .active_target
            .fetch_owner
            .config_snapshot_for_session(Some("SID-primary"));
        assert!(primary.is_enabled());
        assert!(primary.handle_auth_requests());
        assert_eq!(primary.patterns().len(), 1);
        assert_eq!(primary.patterns()[0].url_pattern, "*primary*");

        let aux = bc
            .active_target
            .fetch_owner
            .config_snapshot_for_session(Some("SID-aux"));
        assert!(aux.is_enabled());
        assert!(!aux.handle_auth_requests());
        assert_eq!(aux.patterns().len(), 1);
        assert_eq!(aux.patterns()[0].url_pattern, "*aux*");
    }

    ctx.process_async(json!({
        "id": 212,
        "method": "Fetch.disable",
        "sessionId": "SID-primary"
    }))
    .await;
    ctx.expect_result(212, json!({}), Some("SID-primary"));

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    let aggregate = bc.active_target.fetch_owner.config_snapshot();
    assert!(aggregate.is_enabled());
    assert!(!aggregate.handle_auth_requests());
    assert_eq!(aggregate.patterns().len(), 1);
    assert_eq!(aggregate.patterns()[0].url_pattern, "*aux*");
    assert!(
        !bc.active_target
            .fetch_owner
            .config_snapshot_for_session(Some("SID-primary"))
            .is_enabled()
    );
    assert!(
        bc.active_target
            .fetch_owner
            .config_snapshot_for_session(Some("SID-aux"))
            .is_enabled()
    );
    assert_eq!(
        bc.active_target
            .fetch_owner
            .subresource_interception_config(),
        (true, Some(moli_core::page::SubresourceResourceType::Xhr))
    );
}

#[tokio::test]
async fn disable_drains_only_current_session_pending_subresource_fetches() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-session-fetch-pending".into());
    bc.set_active_target_id("TID-session-fetch".to_owned());
    bc.attach_active_session("SID-primary".to_owned());
    assert!(bc.assign_auxiliary_session_to_target("TID-session-fetch", "SID-aux".to_owned()));
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 220,
        "method": "Fetch.enable",
        "sessionId": "SID-primary",
        "params": {
            "patterns": [{ "urlPattern": "*primary*", "requestStage": "Request", "resourceType": "Fetch" }]
        }
    }))
    .await;
    ctx.expect_result(220, json!({}), Some("SID-primary"));

    ctx.process_async(json!({
        "id": 221,
        "method": "Fetch.enable",
        "sessionId": "SID-aux",
        "params": {
            "patterns": [{ "urlPattern": "*aux*", "requestStage": "Request", "resourceType": "Fetch" }]
        }
    }))
    .await;
    ctx.expect_result(221, json!({}), Some("SID-aux"));

    {
        let fetch_owner = &mut ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .active_target
            .fetch_owner;
        fetch_owner.register_pending_subresource_fetch_request(
            "FETCH-primary".to_owned(),
            pending_subresource_fetch(2200, "SID-primary"),
        );
        fetch_owner.register_pending_subresource_fetch_request(
            "FETCH-aux".to_owned(),
            pending_subresource_fetch(2201, "SID-aux"),
        );
    }

    ctx.process_async(json!({
        "id": 222,
        "method": "Fetch.disable",
        "sessionId": "SID-primary"
    }))
    .await;
    ctx.expect_result(222, json!({}), Some("SID-primary"));

    let fetch_owner = &mut ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .fetch_owner;
    assert!(!fetch_owner.has_pending_fetch_request_id_for_test("FETCH-primary"));
    assert!(fetch_owner.has_pending_fetch_request_id_for_test("FETCH-aux"));
    assert!(
        fetch_owner
            .take_pending_subresource_fetch_request("FETCH-aux", Some("SID-aux"))
            .is_some(),
        "Fetch.disable for primary must not clear auxiliary session pending requests"
    );
}

#[tokio::test]
async fn disable_drains_fetch_owned_pending_when_same_session_network_intercept_remains() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-session-mixed-fetch-pending".into());
    bc.set_active_target_id("TID-session-fetch".to_owned());
    bc.attach_active_session("SID-primary".to_owned());
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 230,
        "method": "Fetch.enable",
        "sessionId": "SID-primary",
        "params": {
            "patterns": [{ "urlPattern": "*fetch*", "requestStage": "Request", "resourceType": "Fetch" }]
        }
    }))
    .await;
    ctx.expect_result(230, json!({}), Some("SID-primary"));

    {
        let fetch_owner = &mut ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context")
            .active_target
            .fetch_owner;
        fetch_owner.add_network_intercept(
            "NETWORK-INTERCEPT-1".to_owned(),
            Some("SID-primary".to_owned()),
            false,
            Vec::new(),
            vec![FetchInterceptionPattern {
                url_pattern: "*network*".to_owned(),
                resource_type_filter: Some(FetchResourceTypeFilter::Fetch),
                request_stage: FetchRequestStage::Request,
            }],
        );
        fetch_owner.register_pending_subresource_fetch_request(
            "FETCH-owned".to_owned(),
            pending_subresource_fetch(2300, "SID-primary"),
        );
        fetch_owner.register_pending_subresource_fetch_request(
            "NETWORK-owned".to_owned(),
            pending_subresource_fetch_with_owner_kind(
                2301,
                "SID-primary",
                PendingSubresourceFetchOwnerKind::NetworkOrBidi,
            ),
        );
    }

    ctx.process_async(json!({
        "id": 231,
        "method": "Fetch.disable",
        "sessionId": "SID-primary"
    }))
    .await;
    ctx.expect_result(231, json!({}), Some("SID-primary"));

    let fetch_owner = &mut ctx
        .conn
        .browser_context
        .as_mut()
        .expect("browser context")
        .active_target
        .fetch_owner;
    assert!(!fetch_owner.has_pending_fetch_request_id_for_test("FETCH-owned"));
    assert!(fetch_owner.has_pending_fetch_request_id_for_test("NETWORK-owned"));
    assert_eq!(
        fetch_owner.subresource_interception_config(),
        (true, Some(moli_core::page::SubresourceResourceType::Fetch)),
        "Fetch.disable should leave same-session Network/BiDi intercept config active"
    );
    assert!(
        fetch_owner
            .take_pending_subresource_fetch_request("NETWORK-owned", Some("SID-primary"))
            .is_some(),
        "Fetch.disable must not clear same-session Network/BiDi-owned pending requests"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn enable_targets_loaded_background_owner_without_promotion() {
    let mut ctx = TestContext::new();
    let background = BackgroundTarget::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut bc = BrowserContext::new("BID-fetch-bg".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.background_targets.push(background);
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>fetch background</title>",
        Some("SID-background"),
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1201,
        "sessionId": "SID-background",
        "method": "Fetch.enable",
        "params": {
            "patterns": [
                { "urlPattern": "*", "resourceType": "XHR", "requestStage": "Request" }
            ]
        }
    }))
    .await;
    ctx.expect_result(1201, json!({}), Some("SID-background"));

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(bc.active_target_id(), Some("TID-active"));
    assert!(!bc.active_target.fetch_owner.is_enabled());
    let staged = bc
        .parked_page_session_state("TID-background")
        .expect("background owner fetch config should be staged");
    assert!(staged.fetch_config.is_enabled());
    assert_eq!(staged.fetch_config.session_id(), Some("SID-background"));
    assert_eq!(staged.fetch_config.patterns().len(), 1);
    assert_eq!(
        staged.fetch_config.patterns()[0].resource_type_filter,
        Some(FetchResourceTypeFilter::Xhr)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_fetch_enable_keeps_background_owner_route_across_completion() {
    let mut ctx = TestContext::new();
    let background = BackgroundTarget::with_url(
        "TID-fetch-background".to_owned(),
        None,
        "about:blank".to_owned(),
    );

    let mut bc = BrowserContext::new("BID-fetch-owner-route".to_owned());
    bc.set_active_target_id("TID-fetch-active".to_owned());
    bc.background_targets.push(background);
    ctx.conn.browser_context = Some(bc);

    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>active fetch</title>",
        None,
    )
    .await;

    let background_route = ctx
        .conn
        .target_session_route_for_target_id("TID-fetch-background")
        .expect("background target route");
    let previous_route = ctx
        .conn
        .replace_none_session_owner_route_override(Some(background_route.clone()));
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>background fetch</title>",
        None,
    )
    .await;
    ctx.conn
        .replace_none_session_owner_route_override(previous_route);
    ctx.sent.clear();
    let raw = serde_json::to_string(&json!({
        "id": 1204,
        "method": "Fetch.enable",
        "params": {
            "patterns": [
                { "urlPattern": "*", "resourceType": "XHR", "requestStage": "Request" }
            ]
        }
    }))
    .expect("Fetch.enable command should serialize");
    let pending = {
        let previous_route = ctx
            .conn
            .replace_none_session_owner_route_override(Some(background_route));
        let step = ctx.conn.start_command_dispatch(&raw);
        ctx.conn
            .replace_none_session_owner_route_override(previous_route);
        match step {
            CdpCommandTaskStep::Pending(pending) => pending,
            CdpCommandTaskStep::Complete(outcome) => {
                panic!(
                    "background Fetch.enable should update the live background page: {:?}",
                    outcome.into_parts().0
                )
            }
        }
    };

    let active_route = ctx
        .conn
        .target_session_route_for_target_id("TID-fetch-active")
        .expect("active target route");
    let previous_route = ctx
        .conn
        .replace_none_session_owner_route_override(Some(active_route));
    let (messages, scheduler_events) = ctx
        .complete_command_task_step_for_test(CdpCommandTaskStep::Pending(pending))
        .await;
    ctx.conn
        .replace_none_session_owner_route_override(previous_route);

    assert!(scheduler_events.is_empty());
    assert_eq!(messages, vec![json!({ "id": 1204, "result": {} })]);

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(bc.active_target_id(), Some("TID-fetch-active"));
    assert!(!bc.active_target.fetch_owner.is_enabled());
    let staged = bc
        .parked_page_session_state("TID-fetch-background")
        .expect("background fetch config should stay parked");
    assert!(staged.fetch_config.is_enabled());
    assert_eq!(staged.fetch_config.patterns().len(), 1);
    assert_eq!(
        staged.fetch_config.patterns()[0].resource_type_filter,
        Some(FetchResourceTypeFilter::Xhr)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn enable_targets_inactive_owner_without_activation() {
    let mut ctx = TestContext::new();
    let mut active = BrowserContext::new("BID-active".to_owned());
    active.set_active_target_id("TID-active".to_owned());
    active.attach_active_session("SID-active".to_owned());
    ctx.conn.browser_context = Some(active);

    let mut inactive = BrowserContext::new("BID-inactive".to_owned());
    inactive.set_active_target_id("TID-inactive".to_owned());
    inactive.attach_active_session("SID-inactive".to_owned());
    ctx.conn.inactive_browser_contexts.push(inactive);

    ctx.process_async(json!({
        "id": 1202,
        "sessionId": "SID-inactive",
        "method": "Fetch.enable",
        "params": {
            "patterns": [
                { "urlPattern": "*inactive*", "requestStage": "Request" }
            ],
            "handleAuthRequests": true
        }
    }))
    .await;
    ctx.expect_result(1202, json!({}), Some("SID-inactive"));

    assert_eq!(
        ctx.conn.browser_context.as_ref().map(|bc| bc.id.as_str()),
        Some("BID-active")
    );
    assert!(
        !ctx.conn
            .browser_context
            .as_ref()
            .expect("active context")
            .active_target
            .fetch_owner
            .is_enabled()
    );
    let inactive = ctx
        .conn
        .inactive_browser_contexts
        .iter()
        .find(|bc| bc.id == "BID-inactive")
        .expect("inactive context");
    assert!(inactive.active_target.fetch_owner.is_enabled());
    assert!(inactive.active_target.fetch_owner.handle_auth_requests());
    assert_eq!(
        inactive
            .active_target
            .fetch_owner
            .config_snapshot()
            .url_pattern(),
        "*inactive*"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn disable_targets_loaded_background_owner_without_promotion() {
    let mut ctx = TestContext::new();
    let background = BackgroundTarget::with_url(
        "TID-background".to_owned(),
        Some("SID-background".to_owned()),
        "about:blank".to_owned(),
    );

    let mut bc = BrowserContext::new("BID-fetch-disable-bg".to_owned());
    bc.set_active_target_id("TID-active".to_owned());
    bc.attach_active_session("SID-active".to_owned());
    bc.background_targets.push(background);
    bc.mutate_parked_page_session_state("TID-background", |state| {
        state.fetch_config.configure(
            Some("SID-background".to_owned()),
            false,
            vec![crate::conn::FetchInterceptionPattern {
                url_pattern: "*".to_owned(),
                resource_type_filter: Some(FetchResourceTypeFilter::Xhr),
                request_stage: FetchRequestStage::Request,
            }],
        );
    });
    ctx.conn.browser_context = Some(bc);
    ctx.install_navigation_fixture_for_session_owner(
        "data:text/html,<title>fetch disable background</title>",
        Some("SID-background"),
    )
    .await;
    ctx.sent.clear();

    ctx.process_async(json!({
        "id": 1203,
        "sessionId": "SID-background",
        "method": "Fetch.disable"
    }))
    .await;
    ctx.expect_result(1203, json!({}), Some("SID-background"));

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert_eq!(bc.active_target_id(), Some("TID-active"));
    assert!(!bc.active_target.fetch_owner.is_enabled());
    assert!(
        bc.parked_page_session_state("TID-background").is_none(),
        "disabled background fetch config should collapse to default"
    );
}

#[tokio::test]
async fn enable_without_params_enables_default_fetch_interception() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 12,
        "method": "Fetch.enable"
    }))
    .await;
    ctx.expect_result(12, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.fetch_owner.is_enabled());
    assert!(!bc.active_target.fetch_owner.handle_auth_requests());
}

#[tokio::test]
async fn enable_with_invalid_request_stage_errors_and_keeps_fetch_disabled() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 3,
        "method": "Fetch.enable",
        "params": {
            "patterns": [
                { "urlPattern": "https://example.com/*", "requestStage": "Bogus" }
            ]
        }
    }))
    .await;
    ctx.expect_error(3, -32602, "InvalidParams");

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.active_target.fetch_owner.is_enabled());
    assert!(!bc.active_target.fetch_owner.handle_auth_requests());
}

#[tokio::test]
async fn enable_with_specific_url_pattern_sets_pattern() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 18,
        "method": "Fetch.enable",
        "params": {
            "patterns": [{ "urlPattern": "https://example.com/api/*", "requestStage": "Request" }]
        }
    }))
    .await;
    ctx.expect_result(18, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.fetch_owner.is_enabled());
    let fetch_config = bc.active_target.fetch_owner.config_snapshot();
    assert_eq!(fetch_config.url_pattern(), "https://example.com/api/*");
}

#[test]
fn url_pattern_matches_supports_wildcards_and_escapes() {
    assert!(super::url_pattern_matches(
        "https://example.com/api/*",
        "https://example.com/api/test"
    ));
    assert!(super::url_pattern_matches(
        "https://example.com/api/it?m",
        "https://example.com/api/item"
    ));
    assert!(super::url_pattern_matches(
        r"https://example.com/api/\*literal\?",
        "https://example.com/api/*literal?"
    ));
    assert!(!super::url_pattern_matches(
        "https://example.com/api/*",
        "https://example.com/other/test"
    ));
}

#[tokio::test]
async fn enable_with_document_resource_type_filter_sets_document_filter() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
            "id": 14,
            "method": "Fetch.enable",
            "params": {
                "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "Document" }]
            }
        }))
    .await;
    ctx.expect_result(14, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.fetch_owner.is_enabled());
    assert_eq!(
        bc.active_target
            .fetch_owner
            .config_snapshot()
            .resource_type_filter(),
        Some(FetchResourceTypeFilter::Document)
    );
}

#[tokio::test]
async fn enable_with_script_resource_type_filter_sets_script_filter() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 15,
        "method": "Fetch.enable",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "Script" }]
        }
    }))
    .await;
    ctx.expect_result(15, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.fetch_owner.is_enabled());
    assert_eq!(
        bc.active_target
            .fetch_owner
            .config_snapshot()
            .resource_type_filter(),
        Some(FetchResourceTypeFilter::Script)
    );
}

#[tokio::test]
async fn enable_with_fetch_resource_type_filter_sets_fetch_filter() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 15,
        "method": "Fetch.enable",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "Fetch" }]
        }
    }))
    .await;
    ctx.expect_result(15, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.fetch_owner.is_enabled());
    assert_eq!(
        bc.active_target
            .fetch_owner
            .config_snapshot()
            .resource_type_filter(),
        Some(FetchResourceTypeFilter::Fetch)
    );
}

#[tokio::test]
async fn enable_with_xhr_resource_type_filter_sets_xhr_filter() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 16,
        "method": "Fetch.enable",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "XHR" }]
        }
    }))
    .await;
    ctx.expect_result(16, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.fetch_owner.is_enabled());
    assert_eq!(
        bc.active_target
            .fetch_owner
            .config_snapshot()
            .resource_type_filter(),
        Some(FetchResourceTypeFilter::Xhr)
    );
}

#[tokio::test]
async fn enable_with_ping_resource_type_filter_sets_ping_filter() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 18,
        "method": "Fetch.enable",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "Ping" }]
        }
    }))
    .await;
    ctx.expect_result(18, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.fetch_owner.is_enabled());
    assert_eq!(
        bc.active_target
            .fetch_owner
            .config_snapshot()
            .resource_type_filter(),
        Some(FetchResourceTypeFilter::Ping)
    );
}

#[tokio::test]
async fn enable_with_csp_violation_report_resource_type_filter_sets_csp_report_filter() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 19,
        "method": "Fetch.enable",
        "params": {
            "patterns": [{
                "urlPattern": "*",
                "requestStage": "Request",
                "resourceType": "CSPViolationReport"
            }]
        }
    }))
    .await;
    ctx.expect_result(19, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.fetch_owner.is_enabled());
    assert_eq!(
        bc.active_target
            .fetch_owner
            .config_snapshot()
            .resource_type_filter(),
        Some(FetchResourceTypeFilter::CspViolationReport)
    );
}

#[tokio::test]
async fn enable_with_websocket_resource_type_filter_sets_websocket_filter() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 20,
        "method": "Fetch.enable",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "WebSocket" }]
        }
    }))
    .await;
    ctx.expect_result(20, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.fetch_owner.is_enabled());
    assert_eq!(
        bc.active_target
            .fetch_owner
            .config_snapshot()
            .resource_type_filter(),
        Some(FetchResourceTypeFilter::WebSocket)
    );
}

#[tokio::test]
async fn enable_with_other_resource_type_filter_sets_other_filter() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 21,
        "method": "Fetch.enable",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "Other" }]
        }
    }))
    .await;
    ctx.expect_result(21, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.fetch_owner.is_enabled());
    assert_eq!(
        bc.active_target
            .fetch_owner
            .config_snapshot()
            .resource_type_filter(),
        Some(FetchResourceTypeFilter::Other)
    );
}

#[tokio::test]
async fn enable_rejects_unimplemented_parser_discovered_resource_type_filters() {
    for resource_type in ["Stylesheet", "Media", "TextTrack"] {
        let mut ctx = TestContext::new();
        ctx.conn.browser_context = Some(BrowserContext::new(format!("BID-{resource_type}")));

        ctx.process_async(json!({
            "id": 20,
            "method": "Fetch.enable",
            "params": {
                "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": resource_type }]
            }
        }))
        .await;
        ctx.expect_error(20, -32602, "InvalidParams");

        let bc = ctx.conn.browser_context.as_ref().unwrap();
        assert!(!bc.active_target.fetch_owner.is_enabled());
    }
}

#[tokio::test]
async fn enable_accepts_image_resource_type_filter() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-Image".into()));

    ctx.process_async(json!({
        "id": 22,
        "method": "Fetch.enable",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Request", "resourceType": "Image" }]
        }
    }))
    .await;
    ctx.expect_result(22, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.fetch_owner.is_enabled());
    assert_eq!(
        bc.active_target
            .fetch_owner
            .config_snapshot()
            .resource_type_filter(),
        Some(FetchResourceTypeFilter::Image)
    );
}

#[tokio::test]
async fn enable_with_response_stage_pattern_sets_response_stage() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 17,
        "method": "Fetch.enable",
        "params": {
            "patterns": [{ "urlPattern": "*", "requestStage": "Response" }]
        }
    }))
    .await;
    ctx.expect_result(17, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.fetch_owner.is_enabled());
    let fetch_config = bc.active_target.fetch_owner.config_snapshot();
    assert_eq!(fetch_config.request_stage(), FetchRequestStage::Response);
    assert_eq!(fetch_config.resource_type_filter(), None);
}

#[tokio::test]
async fn enable_with_multiple_supported_patterns_enables_fetch_interception() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 13,
        "method": "Fetch.enable",
        "params": {
            "patterns": [
                { "urlPattern": "*", "requestStage": "Request" },
                { "urlPattern": "*", "requestStage": "Request" }
            ],
            "handleAuthRequests": true
        }
    }))
    .await;
    ctx.expect_result(13, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(bc.active_target.fetch_owner.is_enabled());
    assert!(bc.active_target.fetch_owner.handle_auth_requests());
    let fetch_config = bc.active_target.fetch_owner.config_snapshot();
    assert_eq!(fetch_config.patterns().len(), 2);
    assert_eq!(
        fetch_config.patterns()[0].request_stage,
        FetchRequestStage::Request
    );
    assert_eq!(
        fetch_config.patterns()[1].request_stage,
        FetchRequestStage::Request
    );
}

#[tokio::test]
async fn disable_without_browser_context_errors() {
    let mut ctx = TestContext::new();
    ctx.process_async(json!({"id": 4, "method": "Fetch.disable"}))
        .await;
    ctx.expect_error(4, -31998, "BrowserContextNotLoaded");
}

#[tokio::test]
async fn disable_clears_fetch_state() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.active_target
        .fetch_owner
        .configure(None, true, Vec::new());
    bc.active_target
        .fetch_owner
        .register_pending_fetch_navigation_request(PendingFetchNavigation {
            fetch_request_id: "INT-1".to_owned(),
            interception_session_id: Some("SID-1".to_owned()),
            document_navigation_token: None,
            navigation: crate::conn::NavigationDispatchState {
                navigate_id: Some(1),
                navigate_session_id: None,
                result_projection: crate::conn::NavigationResultProjection::Cdp(
                    json!({"frameId": "TID-1", "loaderId": "LID-0000000001"}),
                ),
                frame_id: "TID-1".to_owned(),
                session_id: Some("SID-1".to_owned()),
                request_id: Some("REQ-1".to_owned()),
                loader_id: "LID-0000000001".to_owned(),
                request_announced: false,
                requested_url: Url::parse("http://example.test/auth").unwrap(),
                request_method: "POST".to_owned(),
                request_body: Some("payload".to_owned()),
                request_body_bytes: Some(b"payload".to_vec()),
                request_headers: vec![("x-auth".to_owned(), "1".to_owned())],
                request_load_policy: crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
                timestamp: 0.0,
                source_document_security: Default::default(),
            },
            request_cookie_report: None,
            intercept_response: false,
            response_stage_url_match_policy:
                crate::conn::ResponseStageUrlMatchPolicy::AlreadyMatched,
            auth_required_blocked_intercepts: Vec::new(),
        });
    bc.active_target
        .fetch_owner
        .register_pending_fetch_auth_navigation(
            "INT-1".to_owned(),
            PendingFetchAuthNavigation {
                owner_session_id: None,
                action_session_id: Some("SID-1".to_owned()),
                interception_session_id: Some("SID-1".to_owned()),
                owner_kind: crate::conn::PendingSubresourceFetchOwnerKind::Fetch,
                fetch_request_id: "INT-1".to_owned(),
                response_stage_request_id: "INT-1".to_owned(),
                document_navigation_token: None,
                navigation: crate::conn::NavigationDispatchState {
                    navigate_id: Some(1),
                    navigate_session_id: None,
                    result_projection: crate::conn::NavigationResultProjection::Cdp(
                        json!({"frameId": "TID-1", "loaderId": "LID-0000000001"}),
                    ),
                    frame_id: "TID-1".to_owned(),
                    session_id: Some("SID-1".to_owned()),
                    request_id: Some("REQ-1".to_owned()),
                    loader_id: "LID-0000000001".to_owned(),
                    request_announced: false,
                    requested_url: Url::parse("http://example.test/auth").unwrap(),
                    request_method: "POST".to_owned(),
                    request_body: Some("payload".to_owned()),
                    request_body_bytes: Some(b"payload".to_vec()),
                    request_headers: vec![("x-auth".to_owned(), "1".to_owned())],
                    request_load_policy:
                        crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
                    timestamp: 0.0,
                    source_document_security: Default::default(),
                },
                challenge: FetchAuthChallenge {
                    origin: "http://example.test".to_owned(),
                    source: "Server".to_owned(),
                    scheme: "basic".to_owned(),
                    realm: "test-area".to_owned(),
                },
                request_cookie_report: None,
                auth_response: PendingFetchAuthNavigation::test_auth_response(
                    Url::parse("http://example.test/auth").unwrap(),
                ),
                intercept_response: false,
                response_stage_url_match_policy:
                    crate::conn::ResponseStageUrlMatchPolicy::AlreadyMatched,
                auth_stage_chain: None,
            },
        );
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({"id": 5, "method": "Fetch.disable"}))
        .await;
    ctx.expect_result(5, json!({}), None);

    let bc = ctx.conn.browser_context.as_ref().unwrap();
    assert!(!bc.active_target.fetch_owner.is_enabled());
    assert!(!bc.active_target.fetch_owner.handle_auth_requests());
    assert!(
        !bc.active_target
            .fetch_owner
            .has_pending_fetch_state_for_test()
    );
}

#[tokio::test]
async fn disable_after_enable_resets_pending_requests() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.active_target
        .fetch_owner
        .configure(None, false, Vec::new());
    bc.active_target
        .fetch_owner
        .register_pending_fetch_request_id_for_test("INT-1".to_owned());
    bc.active_target
        .fetch_owner
        .register_pending_fetch_request_id_for_test("INT-2".to_owned());
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({"id": 14, "method": "Fetch.disable"}))
        .await;
    ctx.expect_result(14, json!({}), None);

    ctx.process_async(json!({
        "id": 15,
        "method": "Fetch.continueRequest",
        "params": { "requestId": "INT-1" }
    }))
    .await;
    ctx.expect_error(15, -32000, "RequestNotFound");
}

#[tokio::test]
async fn continue_request_validates_request_id_and_pending_state() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 6,
        "method": "Fetch.continueRequest",
        "params": { "requestId": "bad" }
    }))
    .await;
    ctx.expect_error(6, -32602, "InvalidParams");

    ctx.process_async(json!({
        "id": 7,
        "method": "Fetch.continueRequest",
        "params": { "requestId": "INT-99" }
    }))
    .await;
    ctx.expect_error(7, -32000, "RequestNotFound");
}

#[tokio::test]
async fn continue_request_with_intercept_response_still_validates_request_id() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 60,
        "method": "Fetch.continueRequest",
        "params": {
            "requestId": "bad",
            "interceptResponse": true
        }
    }))
    .await;
    ctx.expect_error(60, -32602, "InvalidParams");
}

#[tokio::test]
async fn continue_response_and_take_response_body_as_stream_validate_like_fetch_actions() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.active_target
        .fetch_owner
        .register_pending_fetch_request_id_for_test("INT-42".to_owned());
    bc.active_target
        .fetch_owner
        .register_pending_fetch_request_id_for_test("INT-43".to_owned());
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 16,
        "method": "Fetch.continueResponse",
        "params": { "requestId": "INT-42" }
    }))
    .await;
    ctx.expect_result(16, json!({}), None);

    ctx.process_async(json!({
        "id": 17,
        "method": "Fetch.takeResponseBodyAsStream",
        "params": { "requestId": "INT-43" }
    }))
    .await;
    ctx.expect_result(17, json!({}), None);

    ctx.process_async(json!({
        "id": 18,
        "method": "Fetch.takeResponseBodyAsStream",
        "params": { "requestId": "bad" }
    }))
    .await;
    ctx.expect_error(18, -32602, "InvalidParams");
}

#[tokio::test]
async fn pending_fetch_request_can_be_consumed_once() {
    let mut ctx = TestContext::new();
    let mut bc = BrowserContext::new("BID-1".into());
    bc.active_target
        .fetch_owner
        .register_pending_fetch_request_id_for_test("INT-7".to_owned());
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 8,
        "method": "Fetch.fulfillRequest",
        "params": { "requestId": "INT-7" }
    }))
    .await;
    ctx.expect_result(8, json!({}), None);

    ctx.process_async(json!({
        "id": 9,
        "method": "Fetch.failRequest",
        "params": { "requestId": "INT-7" }
    }))
    .await;
    ctx.expect_error(9, -32000, "RequestNotFound");
}

#[tokio::test]
async fn continue_with_auth_and_get_response_body_share_request_validation() {
    let mut ctx = TestContext::new();
    ctx.conn.browser_context = Some(BrowserContext::new("BID-1".into()));

    ctx.process_async(json!({
        "id": 10,
        "method": "Fetch.continueWithAuth",
        "params": {
            "requestId": "INT-5",
            "authChallengeResponse": { "response": "Default" }
        }
    }))
    .await;
    ctx.expect_error(10, -32000, "RequestNotFound");

    ctx.process_async(json!({
        "id": 11,
        "method": "Fetch.getResponseBody",
        "params": { "requestId": "INT-5" }
    }))
    .await;
    ctx.expect_error(11, -32000, "RequestNotFound");
}

#[tokio::test]
async fn continue_with_auth_rejects_invalid_response_without_consuming_pending_auth_navigation() {
    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_target
        .fetch_owner
        .register_pending_fetch_auth_navigation(
            "INT-8".to_owned(),
            PendingFetchAuthNavigation {
                owner_session_id: Some("SID-1".to_owned()),
                action_session_id: Some("SID-1".to_owned()),
                interception_session_id: Some("SID-1".to_owned()),
                owner_kind: crate::conn::PendingSubresourceFetchOwnerKind::Fetch,
                fetch_request_id: "INT-8".to_owned(),
                response_stage_request_id: "INT-8".to_owned(),
                document_navigation_token: None,
                navigation: crate::conn::NavigationDispatchState {
                    navigate_id: Some(1),
                    navigate_session_id: Some("SID-1".to_owned()),
                    result_projection: crate::conn::NavigationResultProjection::Cdp(
                        json!({"frameId": "TID-1", "loaderId": "LID-0000000001"}),
                    ),
                    frame_id: "TID-1".to_owned(),
                    session_id: Some("SID-1".to_owned()),
                    request_id: Some("REQ-8".to_owned()),
                    loader_id: "LID-0000000001".to_owned(),
                    request_announced: false,
                    requested_url: Url::parse("http://example.test/auth").unwrap(),
                    request_method: "GET".to_owned(),
                    request_body: None,
                    request_body_bytes: None,
                    request_headers: Vec::new(),
                    request_load_policy:
                        crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
                    timestamp: 0.0,
                    source_document_security: Default::default(),
                },
                challenge: FetchAuthChallenge {
                    origin: "http://example.test".to_owned(),
                    source: "Server".to_owned(),
                    scheme: "basic".to_owned(),
                    realm: "test-area".to_owned(),
                },
                request_cookie_report: None,
                auth_response: PendingFetchAuthNavigation::test_auth_response(
                    Url::parse("http://example.test/auth").unwrap(),
                ),
                intercept_response: false,
                response_stage_url_match_policy:
                    crate::conn::ResponseStageUrlMatchPolicy::AlreadyMatched,
                auth_stage_chain: None,
            },
        );
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 77,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": "INT-8",
            "authChallengeResponse": { "response": "Bogus" }
        }
    }))
    .await;
    ctx.expect_error(77, -32602, "InvalidParams");

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert!(
        bc.active_target
            .fetch_owner
            .has_pending_fetch_auth_navigation_for_test("INT-8")
    );
}

#[tokio::test]
async fn continue_with_auth_unsupported_challenge_preserves_pending_auth_navigation() {
    let mut ctx = TestContext::new();
    let mut bc = attached_browser_context();
    bc.active_target
        .fetch_owner
        .register_pending_fetch_auth_navigation(
            "INT-9".to_owned(),
            PendingFetchAuthNavigation {
                owner_session_id: Some("SID-1".to_owned()),
                action_session_id: Some("SID-1".to_owned()),
                interception_session_id: Some("SID-1".to_owned()),
                owner_kind: crate::conn::PendingSubresourceFetchOwnerKind::Fetch,
                fetch_request_id: "INT-9".to_owned(),
                response_stage_request_id: "INT-9".to_owned(),
                document_navigation_token: None,
                navigation: crate::conn::NavigationDispatchState {
                    navigate_id: Some(1),
                    navigate_session_id: Some("SID-1".to_owned()),
                    result_projection: crate::conn::NavigationResultProjection::Cdp(
                        json!({"frameId": "TID-1", "loaderId": "LID-0000000001"}),
                    ),
                    frame_id: "TID-1".to_owned(),
                    session_id: Some("SID-1".to_owned()),
                    request_id: Some("REQ-9".to_owned()),
                    loader_id: "LID-0000000001".to_owned(),
                    request_announced: false,
                    requested_url: Url::parse("http://example.test/auth").unwrap(),
                    request_method: "GET".to_owned(),
                    request_body: None,
                    request_body_bytes: None,
                    request_headers: Vec::new(),
                    request_load_policy:
                        crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
                    timestamp: 0.0,
                    source_document_security: Default::default(),
                },
                challenge: FetchAuthChallenge {
                    origin: "http://example.test".to_owned(),
                    source: "Server".to_owned(),
                    scheme: "bearer".to_owned(),
                    realm: "token-area".to_owned(),
                },
                request_cookie_report: None,
                auth_response: PendingFetchAuthNavigation::test_auth_response(
                    Url::parse("http://example.test/auth").unwrap(),
                ),
                intercept_response: false,
                response_stage_url_match_policy:
                    crate::conn::ResponseStageUrlMatchPolicy::AlreadyMatched,
                auth_stage_chain: None,
            },
        );
    ctx.conn.browser_context = Some(bc);

    ctx.process_async(json!({
        "id": 78,
        "method": "Fetch.continueWithAuth",
        "sessionId": "SID-1",
        "params": {
            "requestId": "INT-9",
            "authChallengeResponse": {
                "response": "ProvideCredentials",
                "username": "u",
                "password": "p"
            }
        }
    }))
    .await;
    ctx.expect_error(78, -32000, "NotImplemented");

    let bc = ctx.conn.browser_context.as_ref().expect("browser context");
    assert!(
        bc.active_target
            .fetch_owner
            .has_pending_fetch_auth_navigation_for_test("INT-9")
    );
}

#[test]
fn extract_auth_challenge_normalizes_scheme_and_allows_missing_realm() {
    let challenge = extract_auth_challenge(&[(
        "www-authenticate".to_owned(),
        "BASIC charset=\"UTF-8\"".to_owned(),
    )])
    .expect("auth challenge");
    assert_eq!(challenge.source, "Server");
    assert_eq!(challenge.scheme, "basic");
    assert_eq!(challenge.realm, "");
}

#[test]
fn extract_auth_challenge_returns_none_without_www_authenticate_header() {
    assert!(
        extract_auth_challenge(&[("content-type".to_owned(), "text/plain".to_owned(),)]).is_none()
    );
}

#[test]
fn extract_auth_challenge_recognizes_proxy_authenticate_header() {
    let challenge = extract_auth_challenge(&[(
        "proxy-authenticate".to_owned(),
        "Basic realm=\"proxy-area\"".to_owned(),
    )])
    .expect("proxy auth challenge");
    assert_eq!(challenge.source, "Proxy");
    assert_eq!(challenge.scheme, "basic");
    assert_eq!(challenge.realm, "proxy-area");
}

#[test]
fn extract_auth_challenge_prefers_supported_scheme_from_later_header() {
    let challenge = extract_auth_challenge(&[
        (
            "www-authenticate".to_owned(),
            "Bearer realm=\"token-area\"".to_owned(),
        ),
        (
            "www-authenticate".to_owned(),
            "Basic realm=\"basic-area\"".to_owned(),
        ),
    ])
    .expect("auth challenge");
    assert_eq!(challenge.source, "Server");
    assert_eq!(challenge.scheme, "basic");
    assert_eq!(challenge.realm, "basic-area");
}

#[test]
fn request_auth_for_challenge_accepts_negotiate_and_ntlm_credentials() {
    let negotiate = request_auth_for_challenge(
        &FetchAuthChallenge {
            origin: "http://example.test".to_owned(),
            source: "Server".to_owned(),
            scheme: "negotiate".to_owned(),
            realm: "corp".to_owned(),
        },
        "user",
        "pass",
    );
    assert!(
        negotiate.is_some(),
        "negotiate challenges should be continuable"
    );

    let ntlm = request_auth_for_challenge(
        &FetchAuthChallenge {
            origin: "http://proxy.test".to_owned(),
            source: "Proxy".to_owned(),
            scheme: "ntlm".to_owned(),
            realm: "proxy".to_owned(),
        },
        "user",
        "pass",
    );
    assert!(ntlm.is_some(), "ntlm challenges should be continuable");
}

#[test]
fn extract_auth_challenge_prefers_supported_scheme_from_combined_header_value() {
    let challenge = extract_auth_challenge(&[(
        "www-authenticate".to_owned(),
        "Bearer realm=\"token-area\", Basic realm=\"basic-area\"".to_owned(),
    )])
    .expect("auth challenge");
    assert_eq!(challenge.source, "Server");
    assert_eq!(challenge.scheme, "basic");
    assert_eq!(challenge.realm, "basic-area");
}

#[test]
fn extract_auth_challenge_preserves_quoted_commas_in_realm() {
    let challenge = extract_auth_challenge(&[(
        "www-authenticate".to_owned(),
        r#"Bearer realm="token-area", Basic realm="basic, area""#.to_owned(),
    )])
    .expect("auth challenge");
    assert_eq!(challenge.source, "Server");
    assert_eq!(challenge.scheme, "basic");
    assert_eq!(challenge.realm, "basic, area");
}

#[test]
fn emit_auth_required_preserves_request_headers_and_post_data_shape() {
    let pending = PendingFetchNavigation {
        fetch_request_id: "INT-11".to_owned(),
        interception_session_id: Some("SID-1".to_owned()),
        document_navigation_token: None,
        navigation: crate::conn::NavigationDispatchState {
            navigate_id: Some(1),
            navigate_session_id: Some("SID-1".to_owned()),
            result_projection: crate::conn::NavigationResultProjection::Cdp(
                json!({"frameId": "TID-1", "loaderId": "LID-0000000001"}),
            ),
            frame_id: "TID-1".to_owned(),
            session_id: Some("SID-1".to_owned()),
            request_id: Some("REQ-11".to_owned()),
            loader_id: "LID-0000000001".to_owned(),
            request_announced: false,
            requested_url: Url::parse("http://example.test/auth").unwrap(),
            request_method: "POST".to_owned(),
            request_body: Some("payload".to_owned()),
            request_body_bytes: Some(b"payload".to_vec()),
            request_headers: vec![("x-test".to_owned(), "yes".to_owned())],
            request_load_policy: crate::conn::NavigationRequestLoadPolicy::DocumentInitiated,
            timestamp: 0.0,
            source_document_security: Default::default(),
        },
        request_cookie_report: None,
        intercept_response: false,
        response_stage_url_match_policy: crate::conn::ResponseStageUrlMatchPolicy::AlreadyMatched,
        auth_required_blocked_intercepts: Vec::new(),
    };
    let challenge = FetchAuthChallenge {
        origin: "http://example.test".to_owned(),
        source: "Server".to_owned(),
        scheme: "basic".to_owned(),
        realm: "test-area".to_owned(),
    };
    let mut out = Vec::new();

    emit_auth_required(&mut out, Some("SID-1"), &pending, &challenge, None, &[]);

    let event = out.pop().expect("auth event");
    assert_eq!(event["method"], "Fetch.authRequired");
    assert_eq!(event["sessionId"], "SID-1");
    assert_eq!(event["params"]["requestId"], "INT-11");
    assert!(event["params"].get("networkId").is_none());
    assert_eq!(event["params"]["request"]["method"], "POST");
    assert_eq!(event["params"]["request"]["headers"]["x-test"], "yes");
    assert_eq!(event["params"]["request"]["hasPostData"], true);
    assert_eq!(event["params"]["authChallenge"]["realm"], "test-area");
}
