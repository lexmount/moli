use serde_json::{Value, json};
use url::Url;

use super::helpers::pending_subresource_response_stage_request_paused_event;
use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, CommandOwnerScope, DEFAULT_LOADER_ID,
    FetchRequestStage, PendingSubresourceFetchOwnerKind, PendingSubresourceFetchRequest,
    PendingSubresourceFetchRequestStage, PendingSubresourceFetchRequestStageChain,
    PendingSubresourceFetchResponseRequest, build_event, monotonic_timestamp_seconds,
};
use crate::devtools_runtime::{
    AutomationEvent, DevToolsFrameId, DevToolsNetworkInterceptId, DevToolsNetworkResourceType,
    DevToolsRequestId, DevToolsTargetId, NetworkRequestEvent,
};
use crate::domains::network;
use moli_core::runtime::DetachedParserScriptFetchContinuation;

use super::navigation::{
    continue_subresource_for_deferred_response_stage_async,
    continue_subresource_for_response_stage_async, continue_subresource_without_fetch_pause_async,
};

fn subresource_request_paused_payload(
    request_id: &str,
    pending: &PendingSubresourceFetchRequest,
    url: &Url,
    method: &str,
    request_headers: &[(String, String)],
    request_body: Option<&str>,
    request_cookie_report: Option<&moli_cookie_jar::StoredCookieQueryReport>,
) -> Value {
    let mut payload = json!({
        "requestId": request_id,
        "frameId": pending.frame_id,
        "request": {
            "url": url.as_str(),
            "method": method,
            "headers": network::request_headers_as_json_object(
                request_headers,
                request_cookie_report,
            ),
            "hasPostData": request_body.is_some(),
        },
        "resourceType": pending.resource_type.as_cdp_fetch_interception_type(),
        "networkId": pending.network_request_id,
    });
    if let Some(request_body) = request_body {
        payload["request"]["postData"] = json!(request_body);
    }
    payload
}

struct PendingSubresourceFetchPauseSource {
    info: moli_core::page::PendingSubresourceFetchInfo,
    detached_parser_script_fetch_continuation: Option<DetachedParserScriptFetchContinuation>,
}

pub(crate) async fn subresource_fetch_pause_prepared_outputs_for_renderer_record_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    source_document: moli_core::RendererDocumentLifecycleIdentity,
    info: moli_core::page::PendingSubresourceFetchInfo,
) -> network::NetworkPreparedOutputs {
    if conn.target_root_document_lifecycle_identity_for_owner(owner) != Some(source_document) {
        return network::NetworkPreparedOutputs::default();
    }
    network::NetworkPreparedOutputs::from_subresource_fetch_pauses(
        prepare_subresource_fetch_pause_sources_async(
            conn,
            owner,
            None,
            None,
            vec![PendingSubresourceFetchPauseSource {
                info,
                detached_parser_script_fetch_continuation: None,
            }],
        )
        .await,
    )
}

pub(crate) async fn detached_parser_script_fetch_pause_prepared_outputs_for_renderer_record_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    source_document: moli_core::RendererDocumentLifecycleIdentity,
    info: moli_core::page::PendingSubresourceFetchInfo,
    continuation: DetachedParserScriptFetchContinuation,
) -> network::NetworkPreparedOutputs {
    if conn.target_root_document_lifecycle_identity_for_owner(owner) != Some(source_document) {
        return network::NetworkPreparedOutputs::default();
    }
    network::NetworkPreparedOutputs::from_subresource_fetch_pauses(
        prepare_subresource_fetch_pause_sources_async(
            conn,
            owner,
            None,
            None,
            vec![PendingSubresourceFetchPauseSource {
                info,
                detached_parser_script_fetch_continuation: Some(continuation),
            }],
        )
        .await,
    )
}

async fn prepare_subresource_fetch_pause_sources_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    frame_id: Option<&str>,
    document_url: Option<&Url>,
    sources: Vec<PendingSubresourceFetchPauseSource>,
) -> Vec<network::TargetSubresourceFetchPauseOutput> {
    let mut outputs = Vec::new();
    let session_id = owner.session_id();
    let page_owner = conn.target_page_residence_identity_for_owner(owner);
    let Some((fetch_snapshot, frame_id)) = (|| {
        let target_frame_id = conn
            .target_session_owner_frame_tree_identity_for_owner(owner)
            .map(|(target_id, _, _, _)| target_id)?;
        let frame_id = frame_id.unwrap_or(&target_frame_id);
        Some((
            conn.target_fetch_subresource_interception_snapshot_for_owner(owner)?,
            frame_id.to_owned(),
        ))
    })() else {
        return outputs;
    };
    let loader_id = conn
        .current_document_loader_id_for_owner(owner)
        .unwrap_or_else(|| DEFAULT_LOADER_ID.to_owned());
    for source in sources {
        let PendingSubresourceFetchPauseSource {
            info,
            detached_parser_script_fetch_continuation,
        } = source;
        let Ok((request_id, network_request_id)) =
            conn.allocate_pending_subresource_fetch_request_ids_for_owner(owner)
        else {
            return outputs;
        };
        let document_url = document_url
            .cloned()
            .unwrap_or_else(|| info.document_url.clone());
        let frame_id = info.frame_id.as_deref().unwrap_or(&frame_id).to_owned();
        let resource_type = info.resource_type.into();
        let request_stage_pause_sessions = fetch_snapshot.matching_request_stage_pause_sessions(
            session_id,
            resource_type,
            &info.url,
        );
        if request_stage_pause_sessions.is_empty() {
            if let Some(continuation) = detached_parser_script_fetch_continuation {
                continuation.continue_request(None);
                continue;
            }
            let Some(page_owner) = page_owner.clone() else {
                continue;
            };
            let pending = PendingSubresourceFetchRequest {
                residence: crate::conn::PendingSubresourceFetchResidence::InstalledPage(
                    page_owner.clone(),
                ),
                owner_session_id: None,
                action_session_id: None,
                owner_kind: PendingSubresourceFetchOwnerKind::NetworkOrBidi,
                internal_id: info.internal_id,
                network_request_id: network_request_id.clone(),
                network_request_handle: info.network_request_handle,
                frame_id: frame_id.clone(),
                document_url: document_url.clone(),
                resource_type: info.resource_type,
                websocket_socket_id: info.websocket_socket_id,
                request_stage_chain: None,
            };
            if fetch_snapshot.matching_request_stage(resource_type, &info.url)
                == Some(FetchRequestStage::Response)
            {
                let handle_auth_requests = fetch_snapshot.matches_auth_required(&info.url);
                let owner_kind = fetch_snapshot
                    .response_stage_owner_kind(resource_type, &info.url)
                    .unwrap_or(PendingSubresourceFetchOwnerKind::NetworkOrBidi);
                let response_stage_blocked_intercepts = fetch_snapshot.matching_network_intercepts(
                    FetchRequestStage::Response,
                    resource_type,
                    &info.url,
                );
                let fetch_owner_session_id = fetch_snapshot
                    .event_session_id(session_id)
                    .map(str::to_owned);
                let response_stage_owner_session_id =
                    if owner_kind == PendingSubresourceFetchOwnerKind::Fetch {
                        fetch_owner_session_id.as_deref()
                    } else {
                        session_id
                    };
                let mut pending = PendingSubresourceFetchRequest {
                    owner_kind,
                    ..pending
                };
                pending.action_session_id = fetch_owner_session_id.clone();
                continue_subresource_for_response_stage_async(
                    conn,
                    owner,
                    response_stage_owner_session_id,
                    request_id,
                    pending,
                    handle_auth_requests,
                    response_stage_blocked_intercepts,
                )
                .await;
                continue;
            }
            if fetch_snapshot.has_response_stage_candidate(resource_type) {
                let handle_auth_requests = fetch_snapshot.matches_auth_required(&info.url);
                let owner_kind = fetch_snapshot
                    .response_stage_candidate_owner_kind(resource_type)
                    .unwrap_or(PendingSubresourceFetchOwnerKind::NetworkOrBidi);
                let fetch_owner_session_id = fetch_snapshot
                    .event_session_id(session_id)
                    .map(str::to_owned);
                let response_stage_owner_session_id =
                    if owner_kind == PendingSubresourceFetchOwnerKind::Fetch {
                        fetch_owner_session_id.as_deref()
                    } else {
                        session_id
                    };
                let mut pending = PendingSubresourceFetchRequest {
                    owner_kind,
                    ..pending
                };
                pending.action_session_id = fetch_owner_session_id.clone();
                continue_subresource_for_deferred_response_stage_async(
                    conn,
                    owner,
                    response_stage_owner_session_id,
                    request_id,
                    pending,
                    handle_auth_requests,
                )
                .await;
                continue;
            }
            let handle_auth_requests = fetch_snapshot.matches_auth_required(&info.url);
            let owner_kind = fetch_snapshot
                .auth_required_owner_kind(&info.url)
                .unwrap_or(PendingSubresourceFetchOwnerKind::NetworkOrBidi);
            continue_subresource_without_fetch_pause_async(
                conn,
                owner,
                handle_auth_requests.then_some(request_id),
                page_owner.clone(),
                info.internal_id,
                network_request_id,
                info.network_request_handle,
                frame_id.clone(),
                document_url,
                info.resource_type,
                handle_auth_requests,
                owner_kind,
            )
            .await;
            continue;
        }
        let first_pause_session = request_stage_pause_sessions
            .first()
            .expect("request-stage pause session should be present")
            .clone();
        let residence = match detached_parser_script_fetch_continuation {
            Some(continuation) => {
                crate::conn::PendingSubresourceFetchResidence::DetachedParserScript(continuation)
            }
            None => {
                let Some(page_owner) = page_owner.clone() else {
                    continue;
                };
                crate::conn::PendingSubresourceFetchResidence::InstalledPage(page_owner)
            }
        };
        let pending = PendingSubresourceFetchRequest {
            residence,
            owner_session_id: None,
            action_session_id: None,
            owner_kind: first_pause_session.owner_kind,
            internal_id: info.internal_id,
            network_request_id,
            network_request_handle: info.network_request_handle,
            frame_id: frame_id.clone(),
            document_url,
            resource_type: info.resource_type,
            websocket_socket_id: info.websocket_socket_id,
            request_stage_chain: None,
        };
        let blocked_intercepts = fetch_snapshot.matching_network_intercepts(
            FetchRequestStage::Request,
            resource_type,
            &info.url,
        );
        let mut remaining_sessions = Vec::new();
        for session in request_stage_pause_sessions.into_iter().skip(1) {
            let Ok((next_request_id, _)) =
                conn.allocate_pending_subresource_fetch_request_ids_for_owner(owner)
            else {
                return outputs;
            };
            remaining_sessions.push(PendingSubresourceFetchRequestStage {
                session_id: session.session_id,
                owner_kind: session.owner_kind,
                request_id: next_request_id,
                blocked_intercepts: blocked_intercepts.clone(),
            });
        }
        let mut pending = pending;
        if !remaining_sessions.is_empty() {
            pending.request_stage_chain =
                Some(Box::new(PendingSubresourceFetchRequestStageChain {
                    url: info.url.clone(),
                    method: info.method.clone(),
                    headers: info.request_headers.clone(),
                    body: info.request_body.clone(),
                    request_cookie_report: info.request_cookie_report.clone(),
                    remaining_sessions,
                }));
        }
        let network_output =
            network::TargetSubresourceFetchPauseNetworkOutput::from_pending_fetch_info(
                pending.network_request_id.clone(),
                pending.frame_id.clone(),
                loader_id.clone(),
                monotonic_timestamp_seconds(),
                pending.document_url.clone(),
                &info,
            )
            .with_blocked_intercepts(blocked_intercepts)
            .with_fetch_request_id(request_id.clone());
        let payload = subresource_request_paused_payload(
            &request_id,
            &pending,
            &info.url,
            &info.method,
            &info.request_headers,
            info.request_body.as_deref(),
            info.request_cookie_report.as_ref(),
        );
        outputs.push(network::TargetSubresourceFetchPauseOutput::new(
            network_output,
            first_pause_session.session_id,
            request_id,
            pending,
            payload,
        ));
    }
    outputs
}

pub(crate) fn emit_subresource_fetch_pause_outputs(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    network_session_ids: &[Option<String>],
    outputs: Vec<network::TargetSubresourceFetchPauseOutput>,
) {
    for output in outputs {
        let network_request_id = output.network_output().network_request_id().to_owned();
        let network_events = network_session_ids
            .iter()
            .flat_map(|session_id| {
                network::fetch_subresource_initial_request_network_events(
                    session_id.as_deref(),
                    output.network_output(),
                )
            })
            .collect::<Vec<_>>();
        let (event_session_id, request_id, mut pending, payload) = output.into_fetch_event_parts();
        let pending_owner_session_id = event_session_id
            .as_deref()
            .filter(|session_id| conn.session_route(Some(session_id)).is_some())
            .or(owner.session_id());
        let pending_owner = pending_owner_session_id
            .map(CommandOwnerScope::for_session)
            .unwrap_or_else(|| owner.clone());
        pending.action_session_id = event_session_id.clone();
        let blocked_intercepts = blocked_intercepts_from_fetch_payload(&payload);
        let fetch_event = subresource_request_paused_event(
            event_session_id.as_deref(),
            &request_id,
            &pending,
            payload,
            blocked_intercepts,
        );
        if !conn.register_pending_subresource_fetch_request_for_owner(
            &pending_owner,
            request_id,
            pending,
        ) {
            return;
        }
        if let Ok(runtime_slot) = conn.runtime_session_owner_slot_mut_for_owner(&pending_owner) {
            // Chromium publishes Network.requestWillBeSent before the Fetch pause.
            // The renderer's later transport start belongs to the same lifecycle
            // and must not publish a second initial Network event.
            runtime_slot.record_fetch_pause_announced_request_id(network_request_id);
        }
        out.extend(network_events);
        out.push(fetch_event);
    }
}

fn subresource_request_paused_event(
    session_id: Option<&str>,
    request_id: &str,
    pending: &PendingSubresourceFetchRequest,
    payload: Value,
    blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
) -> BackgroundProtocolEvent {
    let event_payload = payload;
    let network_event = NetworkRequestEvent {
        target_id: DevToolsTargetId::from(pending.frame_id.as_str()),
        frame_id: Some(DevToolsFrameId::from(pending.frame_id.as_str())),
        request_id: DevToolsRequestId::from(request_id),
        loader_id: None,
        url: event_payload
            .pointer("/request/url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        document_url: Some(pending.document_url.as_str().to_owned()),
        method: event_payload
            .pointer("/request/method")
            .and_then(Value::as_str)
            .map(str::to_owned),
        request_headers: request_headers_from_fetch_payload(&event_payload),
        request_body: event_payload
            .pointer("/request/postData")
            .and_then(Value::as_str)
            .map(str::to_owned),
        request_initiator_type: None,
        bidi_request_initiator_type: None,
        redirect_response: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        resource_type: Some(DevToolsNetworkResourceType::from_fetch_interception_type(
            pending.resource_type,
        )),
        timestamp: None,
        wall_time: None,
        status: None,
        status_text: None,
        response_headers: Vec::new(),
        response_mime_type: None,
        response_protocol: None,
        encoded_data_length: None,
        from_cache: false,
        has_extra_info: false,
        error_text: None,
        loading_failed_canceled: false,
        blocked_intercepts,
        fetch_request_id: None,
        network_id: Some(DevToolsRequestId::from(pending.network_request_id.as_str())),
        auth_challenge: None,
    };
    let payload = network::fetch_request_paused_params(&network_event);
    BackgroundProtocolEvent::immediate_automation_event(
        build_event("Fetch.requestPaused", payload, session_id),
        AutomationEvent::RequestPaused(network_event),
    )
}

fn request_headers_from_fetch_payload(payload: &Value) -> Vec<(String, String)> {
    payload
        .pointer("/request/headers")
        .and_then(Value::as_object)
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(name, value)| {
                    value.as_str().map(|value| (name.clone(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn blocked_intercepts_from_fetch_payload(payload: &Value) -> Vec<DevToolsNetworkInterceptId> {
    payload
        .get("__moliBlockedInterceptors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(DevToolsNetworkInterceptId::from)
        .collect()
}

pub(super) fn next_chained_subresource_request_pause_event(
    conn: &mut CdpConnection,
    mut pending: PendingSubresourceFetchRequest,
) -> Option<BackgroundProtocolEvent> {
    let chain = pending.request_stage_pause_state()?;
    let url = chain.url.clone();
    let method = chain.method.clone();
    let headers = chain.headers.clone();
    let body = chain.body.clone();
    let request_cookie_report = chain.request_cookie_report.clone();
    let previous_owner_session_id = pending.owner_session_id.clone();
    let next = pending.pop_next_request_stage_pause()?;
    let next_session_has_route = next
        .session_id
        .as_deref()
        .is_some_and(|session_id| conn.session_route(Some(session_id)).is_some());
    pending.owner_session_id = if next_session_has_route {
        next.session_id.clone()
    } else {
        previous_owner_session_id.clone()
    };
    pending.action_session_id = next.session_id.clone();
    pending.owner_kind = next.owner_kind;
    let payload = subresource_request_paused_payload(
        &next.request_id,
        &pending,
        &url,
        &method,
        &headers,
        body.as_deref(),
        request_cookie_report.as_ref(),
    );
    let event = subresource_request_paused_event(
        next.session_id.as_deref(),
        &next.request_id,
        &pending,
        payload,
        next.blocked_intercepts.clone(),
    );
    let pending_owner_session_id = next
        .session_id
        .as_deref()
        .filter(|_| next_session_has_route)
        .or(previous_owner_session_id.as_deref());
    if !conn.register_pending_subresource_fetch_request_for_session_owner(
        pending_owner_session_id,
        next.request_id,
        pending,
    ) {
        return None;
    }
    Some(event)
}

pub(super) fn next_chained_subresource_response_pause_event(
    conn: &mut CdpConnection,
    mut pending: PendingSubresourceFetchResponseRequest,
) -> Option<BackgroundProtocolEvent> {
    let previous_owner_session_id = pending.owner_session_id.clone();
    let next = pending.pop_next_response_stage_pause()?;
    let next_session_has_route = next
        .session_id
        .as_deref()
        .is_some_and(|session_id| conn.session_route(Some(session_id)).is_some());
    pending.owner_session_id = if next_session_has_route {
        next.session_id.clone()
    } else {
        previous_owner_session_id.clone()
    };
    pending.action_session_id = next.session_id.clone();
    pending.owner_kind = next.owner_kind;
    let event = pending_subresource_response_stage_request_paused_event(
        next.session_id.as_deref(),
        &next.request_id,
        &pending,
        &next.blocked_intercepts,
    );
    let pending_owner_session_id = next
        .session_id
        .as_deref()
        .filter(|_| next_session_has_route)
        .or(previous_owner_session_id.as_deref());
    if !conn.register_pending_subresource_fetch_response_request_for_session_owner(
        pending_owner_session_id,
        next.request_id,
        pending,
    ) {
        return None;
    }
    Some(event)
}

#[cfg(test)]
mod tests {
    use moli_core::page::{PendingSubresourceFetchInfo, SubresourceResourceType};
    use serde_json::json;
    use url::Url;

    use crate::conn::{
        BackgroundProtocolEvent, BrowserContext, CdpConnection, CommandOwnerScope, PageTargetHost,
        PendingSubresourceFetchOwnerKind, PendingSubresourceFetchRequest,
    };
    use crate::devtools_runtime::{AutomationEvent, DevToolsNetworkResourceType};
    use crate::domains::network::{
        TargetSubresourceFetchPauseNetworkOutput, TargetSubresourceFetchPauseOutput,
    };

    fn split_events(
        events: Vec<BackgroundProtocolEvent>,
    ) -> (Vec<serde_json::Value>, Vec<Option<AutomationEvent>>) {
        events
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .unzip()
    }

    fn pending_fetch_info(url: &str) -> PendingSubresourceFetchInfo {
        PendingSubresourceFetchInfo {
            internal_id: 7,
            network_request_handle: None,
            frame_id: Some("FRAME-1".to_owned()),
            document_url: Url::parse("https://example.test/page").unwrap(),
            url: Url::parse(url).unwrap(),
            websocket_socket_id: None,
            method: "GET".to_owned(),
            request_headers: Vec::new(),
            request_body: None,
            request_body_bytes: None,
            resource_type: SubresourceResourceType::Fetch,
            request_cookie_report: None,
        }
    }

    fn fetch_pause_output(
        page_owner: crate::conn::TargetPageResidenceIdentity,
        cdp_request_id: &str,
        network_request_id: &str,
        url: &str,
    ) -> TargetSubresourceFetchPauseOutput {
        fetch_pause_output_with_cookie_report(
            page_owner,
            cdp_request_id,
            network_request_id,
            url,
            None,
        )
    }

    fn fetch_pause_output_with_cookie_report(
        page_owner: crate::conn::TargetPageResidenceIdentity,
        cdp_request_id: &str,
        network_request_id: &str,
        url: &str,
        request_cookie_report: Option<moli_cookie_jar::StoredCookieQueryReport>,
    ) -> TargetSubresourceFetchPauseOutput {
        let mut info = pending_fetch_info(url);
        info.request_cookie_report = request_cookie_report;
        let pending = PendingSubresourceFetchRequest {
            residence: crate::conn::PendingSubresourceFetchResidence::InstalledPage(page_owner),
            owner_session_id: None,
            action_session_id: None,
            owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
            internal_id: info.internal_id,
            network_request_id: network_request_id.to_owned(),
            network_request_handle: info.network_request_handle,
            frame_id: "FRAME-1".to_owned(),
            document_url: info.document_url.clone(),
            resource_type: info.resource_type,
            websocket_socket_id: None,
            request_stage_chain: None,
        };
        let network_output = TargetSubresourceFetchPauseNetworkOutput::from_pending_fetch_info(
            network_request_id.to_owned(),
            "FRAME-1".to_owned(),
            "LOADER-1".to_owned(),
            12.0,
            info.document_url.clone(),
            &info,
        );
        let payload = json!({
            "requestId": cdp_request_id,
            "frameId": "FRAME-1",
            "request": {
                "url": url,
                "method": "GET",
                "headers": {},
                "hasPostData": false,
            },
            "resourceType": "XHR",
            "networkId": network_request_id,
        });
        TargetSubresourceFetchPauseOutput::new(
            network_output,
            Some("FETCH-SID".to_owned()),
            cdp_request_id.to_owned(),
            pending,
            payload,
        )
    }

    #[test]
    fn prepared_subresource_fetch_pause_pairs_emit_network_then_fetch_per_item() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new_with_page_for_test("BID-1", "TID-active");
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        conn.install_browser_context_fixture_for_test(browser_context);
        let page_owner = conn
            .target_page_residence_identity_for_session(None)
            .expect("active test target should expose a Page residence identity");
        let owner = CommandOwnerScope::capture(&conn, None);

        let mut events = Vec::new();
        super::emit_subresource_fetch_pause_outputs(
            &mut conn,
            &mut events,
            &owner,
            &[Some("NETWORK-SID".to_owned())],
            vec![
                fetch_pause_output(
                    page_owner.clone(),
                    "FETCH-1",
                    "REQ-1",
                    "https://example.test/one",
                ),
                fetch_pause_output(page_owner, "FETCH-2", "REQ-2", "https://example.test/two"),
            ],
        );
        super::emit_subresource_fetch_pause_outputs(
            &mut conn,
            &mut events,
            &owner,
            &[Some("NETWORK-SID".to_owned())],
            Vec::new(),
        );

        let (out, sidecars) = split_events(events);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0]["method"], json!("Network.requestWillBeSent"));
        assert_eq!(out[0]["sessionId"], json!("NETWORK-SID"));
        assert_eq!(out[0]["params"]["requestId"], json!("REQ-1"));
        assert_eq!(out[1]["method"], json!("Fetch.requestPaused"));
        assert_eq!(out[1]["sessionId"], json!("FETCH-SID"));
        assert_eq!(out[1]["params"]["requestId"], json!("FETCH-1"));
        assert_eq!(out[1]["params"]["networkId"], json!("REQ-1"));
        assert_eq!(out[2]["method"], json!("Network.requestWillBeSent"));
        assert_eq!(out[2]["params"]["requestId"], json!("REQ-2"));
        assert_eq!(out[3]["method"], json!("Fetch.requestPaused"));
        assert_eq!(out[3]["params"]["requestId"], json!("FETCH-2"));
        assert_eq!(out[3]["params"]["networkId"], json!("REQ-2"));
        assert!(matches!(
            sidecars[0].as_ref(),
            Some(AutomationEvent::NetworkBeforeRequestSent(event))
                if event.request_id.as_str() == "REQ-1"
                && event.resource_type == Some(DevToolsNetworkResourceType::Fetch)
        ));
        assert!(matches!(
            sidecars[1].as_ref(),
            Some(AutomationEvent::RequestPaused(event))
                if event.request_id.as_str() == "FETCH-1"
                    && event.resource_type == Some(DevToolsNetworkResourceType::Xhr)
                    && event.url == "https://example.test/one"
        ));

        let bc = conn.browser_context.as_ref().unwrap();
        assert!(
            bc.active_page_target()
                .fetch_owner
                .has_pending_subresource_fetch_for_test("FETCH-1")
        );
        assert!(
            bc.active_page_target()
                .fetch_owner
                .has_pending_subresource_fetch_for_test("FETCH-2")
        );
    }

    #[test]
    fn fetch_pause_does_not_synthesize_cookie_extra_info() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new_with_page_for_test("BID-1", "TID-active");
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        conn.install_browser_context_fixture_for_test(browser_context);
        let page_owner = conn
            .target_page_residence_identity_for_session(None)
            .expect("active test target should expose a Page residence identity");
        let owner = CommandOwnerScope::capture(&conn, None);
        let mut events = Vec::new();

        super::emit_subresource_fetch_pause_outputs(
            &mut conn,
            &mut events,
            &owner,
            &[Some("NETWORK-SID".to_owned())],
            vec![fetch_pause_output_with_cookie_report(
                page_owner,
                "FETCH-1",
                "REQ-1",
                "https://example.test/one",
                Some(moli_cookie_jar::StoredCookieQueryReport::default()),
            )],
        );

        let (out, sidecars) = split_events(events);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["method"], json!("Network.requestWillBeSent"));
        assert_eq!(out[1]["method"], json!("Fetch.requestPaused"));
        assert!(
            out.iter()
                .all(|event| event["method"] != json!("Network.requestWillBeSentExtraInfo")),
            "request ExtraInfo belongs to the network transport observation"
        );
        assert!(matches!(
            sidecars[0].as_ref(),
            Some(AutomationEvent::NetworkBeforeRequestSent(event))
                if event.request_cookie_report.is_some()
        ));
    }

    #[test]
    fn prepared_subresource_fetch_pause_does_not_emit_after_page_replacement() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new_with_page_for_test("BID-1", "TID-active");
        browser_context
            .active_page_target_mut()
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        conn.install_browser_context_fixture_for_test(browser_context);
        let page_owner = conn
            .target_page_residence_identity_for_session(None)
            .expect("active test target should expose a Page residence identity");
        conn.browser_context
            .as_mut()
            .expect("browser context should remain installed")
            .active_page_target_mut()
            .runtime_slot
            .replace_page_attachment_id_for_test();
        let owner = CommandOwnerScope::capture(&conn, None);

        let mut events = Vec::new();
        super::emit_subresource_fetch_pause_outputs(
            &mut conn,
            &mut events,
            &owner,
            &[Some("NETWORK-SID".to_owned())],
            vec![fetch_pause_output(
                page_owner,
                "FETCH-1",
                "REQ-1",
                "https://example.test/one",
            )],
        );

        assert!(
            events.is_empty(),
            "a stale prepared pause must not emit Network or Fetch events that have no command state"
        );
        assert!(
            !conn
                .browser_context
                .as_ref()
                .expect("browser context should remain installed")
                .active_page_target()
                .fetch_owner
                .has_pending_subresource_fetch_for_test("FETCH-1"),
            "a stale prepared pause must not install pending command state"
        );
    }

    #[test]
    fn prepared_subresource_fetch_pause_can_emit_for_background_owner() {
        let mut conn = CdpConnection::default();
        let mut bc = BrowserContext::new_with_page_for_test("BID-1", "TID-active");
        let target = PageTargetHost::with_url(
            "TID-background".to_owned(),
            Some("SID-background".to_owned()),
            "https://example.test/background".to_owned(),
        );
        bc.insert_page_target_host(target);
        conn.install_browser_context_fixture_for_test(bc);
        conn.runtime_session_owner_slot_mut(Some("SID-background"))
            .expect("background test target runtime slot")
            .set_page_attachment_id_for_test(1);
        let page_owner = conn
            .target_page_residence_identity_for_session(Some("SID-background"))
            .expect("background test target should expose a Page residence identity");
        let mut events = Vec::new();
        super::emit_subresource_fetch_pause_outputs(
            &mut conn,
            &mut events,
            &CommandOwnerScope::for_session("SID-background"),
            &[Some("NETWORK-SID".to_owned())],
            vec![fetch_pause_output(
                page_owner,
                "FETCH-1",
                "REQ-1",
                "https://example.test/one",
            )],
        );

        let (out, sidecars) = split_events(events);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["method"], json!("Network.requestWillBeSent"));
        assert_eq!(out[0]["sessionId"], json!("NETWORK-SID"));
        assert_eq!(out[1]["method"], json!("Fetch.requestPaused"));
        assert_eq!(out[1]["sessionId"], json!("FETCH-SID"));
        assert!(matches!(
            sidecars[1].as_ref(),
            Some(AutomationEvent::RequestPaused(event))
                if event.request_id.as_str() == "FETCH-1"
        ));

        let bc = conn.browser_context.as_ref().unwrap();
        assert!(
            bc.background_target("TID-background")
                .expect("background target must exist")
                .fetch_owner
                .pending_state()
                .has_pending_subresource_fetch_for_test("FETCH-1")
        );
        assert!(
            !bc.active_page_target()
                .fetch_owner
                .has_pending_subresource_fetch_for_test("FETCH-1"),
            "background owner emission must not register the pause on the active target"
        );
    }
}
