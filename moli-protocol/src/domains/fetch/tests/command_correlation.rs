use super::*;
use crate::conn::{
    CapturedBody, CdpCommandTaskStep, ClaimedSubresourceContinueRequest, FetchAuthChallenge,
    PendingSubresourceFetchAuthRequest, PendingSubresourceFetchOwnerKind,
    PendingSubresourceFetchRequest, PendingSubresourceFetchResponseRequest,
};
use moli_core::page::SubresourceResourceType;

async fn context_with_loaded_fetch_page() -> TestContext {
    let mut ctx = TestContext::new();
    let page = ctx
        .conn
        .load_page_via_runtime_async("data:text/html,<title>fetch correlation</title>")
        .await
        .expect("fetch correlation page should load");
    let mut browser_context = attached_browser_context();
    browser_context
        .active_target
        .runtime_slot
        .replace_loaded_page(Some(page));
    ctx.conn.browser_context = Some(browser_context);
    ctx
}

fn pending_request(
    page_owner: &crate::conn::TargetPageResidenceIdentity,
    internal_id: u64,
) -> PendingSubresourceFetchRequest {
    PendingSubresourceFetchRequest {
        residence: crate::conn::PendingSubresourceFetchResidence::InstalledPage(page_owner.clone()),
        owner_session_id: Some("SID-1".to_owned()),
        action_session_id: Some("SID-1".to_owned()),
        owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
        internal_id,
        network_request_id: format!("NETWORK-{internal_id}"),
        network_request_handle: None,
        frame_id: "TID-1".to_owned(),
        document_url: Url::parse("https://example.test/page").unwrap(),
        resource_type: SubresourceResourceType::Fetch,
        websocket_socket_id: None,
        request_stage_chain: None,
    }
}

fn pending_auth(
    page_owner: &crate::conn::TargetPageResidenceIdentity,
    internal_id: u64,
) -> PendingSubresourceFetchAuthRequest {
    let pending = pending_request(page_owner, internal_id);
    let page_owner = pending
        .installed_page_owner()
        .expect("test request should belong to an installed Page")
        .clone();
    PendingSubresourceFetchAuthRequest {
        page_owner,
        owner_session_id: pending.owner_session_id,
        action_session_id: pending.action_session_id,
        owner_kind: pending.owner_kind,
        internal_id,
        network_request_id: pending.network_request_id,
        network_request_handle: pending.network_request_handle,
        frame_id: pending.frame_id,
        document_url: pending.document_url,
        resource_type: pending.resource_type,
        websocket_socket_id: pending.websocket_socket_id,
        url: Url::parse("https://example.test/protected").unwrap(),
        method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        request_cookie_report: None,
        challenge: FetchAuthChallenge {
            origin: "http://example.test".to_owned(),
            source: "Server".to_owned(),
            scheme: "basic".to_owned(),
            realm: "private".to_owned(),
        },
        intercept_response: true,
        auth_stage_chain: None,
    }
}

fn pending_response(
    page_owner: &crate::conn::TargetPageResidenceIdentity,
    internal_id: u64,
) -> PendingSubresourceFetchResponseRequest {
    let pending = pending_request(page_owner, internal_id);
    let page_owner = pending
        .installed_page_owner()
        .expect("test request should belong to an installed Page")
        .clone();
    PendingSubresourceFetchResponseRequest {
        page_owner,
        owner_session_id: pending.owner_session_id,
        action_session_id: pending.action_session_id,
        owner_kind: pending.owner_kind,
        internal_id,
        network_request_id: pending.network_request_id,
        network_request_handle: pending.network_request_handle,
        frame_id: pending.frame_id,
        document_url: pending.document_url,
        resource_type: pending.resource_type,
        websocket_socket_id: pending.websocket_socket_id,
        url: Url::parse("https://example.test/response").unwrap(),
        method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        request_cookie_report: None,
        response_status: 200,
        response_headers: Vec::new(),
        response_head_overridden: false,
        response_body_taken_as_stream: false,
        response_body: CapturedBody::from_bytes(Vec::new()),
        response_stage_chain: None,
    }
}

async fn assert_correlation_lifetime(
    ctx: &mut TestContext,
    command: Value,
    command_id: u64,
    internal_id: u64,
    request_id: &str,
) {
    let raw = serde_json::to_string(&command).expect("Fetch command should serialize");
    let step = ctx.conn.start_command_dispatch(&raw);

    assert!(matches!(&step, CdpCommandTaskStep::Pending(_)));
    assert_eq!(
        ctx.conn
            .in_flight_subresource_fetch_request_id_for_session_owner(Some("SID-1"), internal_id,)
            .as_deref(),
        Some(request_id),
        "renderer-visible network work must have protocol correlation state"
    );

    let (messages, _) = ctx.complete_command_task_step_for_test(step).await;
    assert!(
        messages
            .iter()
            .any(|message| message["id"] == command_id && message["error"].is_object()),
        "the synthetic unknown renderer request should fail"
    );
    assert_eq!(
        ctx.conn
            .in_flight_subresource_fetch_request_id_for_session_owner(Some("SID-1"), internal_id,),
        None,
        "failed renderer completion must roll back prepared correlation state"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_request_registers_correlation_before_renderer_completion() {
    let mut ctx = context_with_loaded_fetch_page().await;
    let page_owner = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-1"))
        .expect("test Page residence should exist");
    assert!(
        ctx.conn
            .register_pending_subresource_fetch_request_for_session_owner(
                Some("SID-1"),
                "INT-61".to_owned(),
                pending_request(&page_owner, 61),
            )
    );

    assert_correlation_lifetime(
        &mut ctx,
        json!({
            "id": 61,
            "method": "Fetch.continueRequest",
            "sessionId": "SID-1",
            "params": { "requestId": "INT-61", "interceptResponse": true }
        }),
        61,
        61,
        "INT-61",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_fetch_command_state_is_bound_to_page_attachment() {
    let mut ctx = context_with_loaded_fetch_page().await;
    let initial_owner = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-1"))
        .expect("initial Page residence should exist");
    assert!(
        ctx.conn
            .register_pending_subresource_fetch_request_for_session_owner(
                Some("SID-1"),
                "INT-collision".to_owned(),
                pending_request(&initial_owner, 71),
            )
    );

    ctx.conn
        .runtime_session_owner_slot_mut(Some("SID-1"))
        .expect("runtime owner should remain addressable")
        .replace_page_attachment_id_for_test();
    assert!(
        ctx.conn
            .take_pending_subresource_fetch_request_for_action_session_owner(
                Some("SID-1"),
                Some("SID-1"),
                "INT-collision",
            )
            .is_none(),
        "a command must not recover request state admitted by the previous Page residence"
    );

    let replacement_owner = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-1"))
        .expect("replacement Page residence should exist");
    let mut replacement = pending_request(&replacement_owner, 71);
    replacement.network_request_id = "NETWORK-replacement".to_owned();
    assert!(
        ctx.conn
            .register_pending_subresource_fetch_request_for_session_owner(
                Some("SID-1"),
                "INT-collision".to_owned(),
                replacement,
            )
    );
    assert_eq!(
        ctx.conn
            .take_pending_subresource_fetch_request_for_action_session_owner(
                Some("SID-1"),
                Some("SID-1"),
                "INT-collision",
            )
            .map(|pending| pending.network_request_id)
            .as_deref(),
        Some("NETWORK-replacement")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn completed_continue_atomically_claims_a_pause_still_pending_publication() {
    let mut ctx = context_with_loaded_fetch_page().await;
    let page_owner = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-1"))
        .expect("test Page residence should exist");
    assert!(
        ctx.conn
            .register_pending_subresource_fetch_request_for_session_owner(
                Some("SID-1"),
                "INT-pending-completion".to_owned(),
                pending_request(&page_owner, 74),
            )
    );

    assert!(
        ctx.conn
            .claim_subresource_continue_request_for_session_owner(
                Some("SID-1"),
                &page_owner,
                74,
                false,
            )
            .is_none(),
        "response/auth continuations must not claim a request that has not become in-flight"
    );
    let claimed = ctx
        .conn
        .claim_subresource_continue_request_for_session_owner(Some("SID-1"), &page_owner, 74, true)
        .expect("terminal completion should claim the still-pending pause");
    let ClaimedSubresourceContinueRequest::PendingCompletion(pending) = claimed else {
        panic!("terminal completion must preserve the pending-publication race state");
    };
    assert_eq!(pending.network_request_id, "NETWORK-74");
    assert!(
        ctx.conn
            .claim_subresource_continue_request_for_session_owner(
                Some("SID-1"),
                &page_owner,
                74,
                true,
            )
            .is_none(),
        "the exact request state must be move-owned and claimable only once"
    );

    ctx.conn
        .runtime_session_owner_slot_mut(Some("SID-1"))
        .expect("runtime owner should remain addressable")
        .replace_page_attachment_id_for_test();
    let replacement_owner = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-1"))
        .expect("replacement Page residence should exist");
    let replacement = pending_request(&replacement_owner, 75);
    assert!(
        ctx.conn
            .register_in_flight_subresource_fetch_request_for_session_owner(
                Some("SID-1"),
                Some("INT-replacement".to_owned()),
                replacement,
            )
    );
    assert!(
        ctx.conn
            .claim_subresource_continue_request_for_session_owner(
                Some("SID-1"),
                &page_owner,
                75,
                true,
            )
            .is_none(),
        "a stale capture owner must be rejected before it can claim replacement state"
    );
    assert_eq!(
        ctx.conn
            .in_flight_subresource_fetch_request_id_for_session_owner(Some("SID-1"), 75)
            .as_deref(),
        Some("INT-replacement"),
        "a rejected stale capture must leave the replacement request resident"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn continuation_claim_preserves_state_owned_by_a_different_page_residence() {
    let mut ctx = context_with_loaded_fetch_page().await;
    let retired_owner = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-1"))
        .expect("initial Page residence should exist");
    assert!(
        ctx.conn
            .register_in_flight_subresource_fetch_request_for_session_owner(
                Some("SID-1"),
                Some("INT-retired-in-flight".to_owned()),
                pending_request(&retired_owner, 76),
            )
    );
    assert!(
        ctx.conn
            .register_pending_subresource_fetch_request_for_session_owner(
                Some("SID-1"),
                "INT-retired-pending".to_owned(),
                pending_request(&retired_owner, 77),
            )
    );

    ctx.conn
        .runtime_session_owner_slot_mut(Some("SID-1"))
        .expect("runtime owner should remain addressable")
        .replace_page_attachment_id_for_test();
    let replacement_owner = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-1"))
        .expect("replacement Page residence should exist");

    assert!(
        ctx.conn
            .claim_subresource_continue_request_for_session_owner(
                Some("SID-1"),
                &replacement_owner,
                76,
                false,
            )
            .is_none(),
        "a replacement continuation must not claim in-flight state from the retired Page"
    );
    assert!(
        ctx.conn
            .claim_subresource_continue_request_for_session_owner(
                Some("SID-1"),
                &replacement_owner,
                77,
                true,
            )
            .is_none(),
        "a replacement completion must not claim pending state from the retired Page"
    );

    ctx.conn
        .runtime_session_owner_slot_mut(Some("SID-1"))
        .expect("runtime owner should remain addressable")
        .install_page_attachment_id_for_test(retired_owner.page_attachment_id());
    let ClaimedSubresourceContinueRequest::InFlight(in_flight) = ctx
        .conn
        .claim_subresource_continue_request_for_session_owner(
            Some("SID-1"),
            &retired_owner,
            76,
            false,
        )
        .expect("owner mismatch must leave the in-flight request resident")
    else {
        panic!("the preserved request should remain in-flight");
    };
    assert_eq!(
        in_flight.request_id.as_deref(),
        Some("INT-retired-in-flight")
    );

    let ClaimedSubresourceContinueRequest::PendingCompletion(pending) = ctx
        .conn
        .claim_subresource_continue_request_for_session_owner(
            Some("SID-1"),
            &retired_owner,
            77,
            true,
        )
        .expect("owner mismatch must leave the pending request resident")
    else {
        panic!("the preserved request should remain pending");
    };
    assert_eq!(pending.network_request_id, "NETWORK-77");
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_fetch_auth_state_is_bound_to_page_attachment() {
    let mut ctx = context_with_loaded_fetch_page().await;
    let initial_owner = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-1"))
        .expect("initial Page residence should exist");
    assert!(
        ctx.conn
            .register_pending_subresource_fetch_auth_request_for_session_owner(
                Some("SID-1"),
                "AUTH-collision".to_owned(),
                pending_auth(&initial_owner, 72),
            )
    );

    ctx.conn
        .runtime_session_owner_slot_mut(Some("SID-1"))
        .expect("runtime owner should remain addressable")
        .replace_page_attachment_id_for_test();
    assert!(
        ctx.conn
            .take_pending_subresource_fetch_auth_request_for_action_session_owner(
                Some("SID-1"),
                Some("SID-1"),
                "AUTH-collision",
            )
            .is_none(),
        "an auth command must not recover challenge state admitted by the previous Page residence"
    );

    let replacement_owner = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-1"))
        .expect("replacement Page residence should exist");
    let mut replacement = pending_auth(&replacement_owner, 72);
    replacement.network_request_id = "NETWORK-auth-replacement".to_owned();
    assert!(
        ctx.conn
            .register_pending_subresource_fetch_auth_request_for_session_owner(
                Some("SID-1"),
                "AUTH-collision".to_owned(),
                replacement,
            )
    );
    assert_eq!(
        ctx.conn
            .take_pending_subresource_fetch_auth_request_for_action_session_owner(
                Some("SID-1"),
                Some("SID-1"),
                "AUTH-collision",
            )
            .map(|pending| pending.network_request_id)
            .as_deref(),
        Some("NETWORK-auth-replacement")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_fetch_response_state_is_bound_to_page_attachment() {
    let mut ctx = context_with_loaded_fetch_page().await;
    let initial_owner = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-1"))
        .expect("initial Page residence should exist");
    assert!(
        ctx.conn
            .register_pending_subresource_fetch_response_request_for_session_owner(
                Some("SID-1"),
                "RESPONSE-collision".to_owned(),
                pending_response(&initial_owner, 73),
            )
    );

    ctx.conn
        .runtime_session_owner_slot_mut(Some("SID-1"))
        .expect("runtime owner should remain addressable")
        .replace_page_attachment_id_for_test();
    assert!(
        ctx.conn
            .take_pending_subresource_fetch_response_request_for_action_session_owner(
                Some("SID-1"),
                Some("SID-1"),
                "RESPONSE-collision",
            )
            .is_none(),
        "a response command must not recover response state admitted by the previous Page residence"
    );

    let replacement_owner = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-1"))
        .expect("replacement Page residence should exist");
    let mut replacement = pending_response(&replacement_owner, 73);
    replacement.network_request_id = "NETWORK-response-replacement".to_owned();
    assert!(
        ctx.conn
            .register_pending_subresource_fetch_response_request_for_session_owner(
                Some("SID-1"),
                "RESPONSE-collision".to_owned(),
                replacement,
            )
    );
    assert_eq!(
        ctx.conn
            .take_pending_subresource_fetch_response_request_for_action_session_owner(
                Some("SID-1"),
                Some("SID-1"),
                "RESPONSE-collision",
            )
            .map(|pending| pending.network_request_id)
            .as_deref(),
        Some("NETWORK-response-replacement")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_with_auth_registers_retry_correlation_before_renderer_completion() {
    let mut ctx = context_with_loaded_fetch_page().await;
    let page_owner = ctx
        .conn
        .target_page_residence_identity_for_session(Some("SID-1"))
        .expect("test Page residence should exist");
    assert!(
        ctx.conn
            .register_pending_subresource_fetch_auth_request_for_session_owner(
                Some("SID-1"),
                "INT-62".to_owned(),
                pending_auth(&page_owner, 62),
            )
    );

    assert_correlation_lifetime(
        &mut ctx,
        json!({
            "id": 62,
            "method": "Fetch.continueWithAuth",
            "sessionId": "SID-1",
            "params": {
                "requestId": "INT-62",
                "authChallengeResponse": {
                    "response": "ProvideCredentials",
                    "username": "user",
                    "password": "pass"
                }
            }
        }),
        62,
        62,
        "INT-62",
    )
    .await;
}
