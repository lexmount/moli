use crate::conn::{
    CdpConnection, DocumentBodySource, DocumentNavigationToken, FetchAuthChallenge,
    NavigationDispatchState, NavigationLoadOutcome, PendingFetchNavigation,
    PendingSubresourceFetchAuthStage, PendingSubresourceFetchAuthStageChain,
    PendingSubresourceFetchOwnerKind, PendingSubresourceFetchRequest, decode_data_url_response,
};
use crate::devtools_runtime::{DevToolsNetworkInterceptId, DevToolsNetworkResourceType};
use crate::domains::{command_output::CommandOutputBuffer, network, page};
use moli_cookie_jar::StoredCookieQueryReport;
use moli_core::page::{SubresourceAuthCredentials, SubresourceAuthScheme, SubresourceResourceType};
use moli_fetch::{
    NetworkFetchResult, NetworkObservationJournal, RawResponse, ResponseHead, StreamingRawResponse,
};
use moli_web_mime::response_headers_indicate_attachment_download;
use url::Url;

use super::{
    FetchCommandOutput,
    helpers::{
        extract_auth_challenge, navigation_response_stage_request_paused_event,
        pending_fetch_auth_navigation_required_event, populate_auth_challenge_origin,
    },
};

fn prepare_navigation_response_stage(
    conn: &CdpConnection,
    pending: &mut PendingFetchNavigation,
    final_url: &Url,
) -> bool {
    if !pending.intercept_response {
        return false;
    }
    if !pending
        .response_stage_url_match_policy
        .requires_final_url_match()
    {
        return true;
    }
    let Some(response_stage) = conn
        .target_fetch_subresource_interception_snapshot_for_session_owner(
            pending.navigation.navigate_session_id.as_deref(),
        )
        .and_then(|snapshot| {
            snapshot
                .matching_response_stage_pause_sessions(
                    pending.navigation.navigate_session_id.as_deref(),
                    DevToolsNetworkResourceType::Document,
                    final_url,
                )
                .into_iter()
                .next()
        })
    else {
        return false;
    };
    pending.interception_session_id = response_stage.session_id;
    true
}

pub(crate) async fn load_or_pause_navigation_for_auth_into_buffer_async(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    mut pending: PendingFetchNavigation,
    auth: Option<SubresourceAuthCredentials>,
    prior_network_observation_journal: Option<NetworkObservationJournal>,
) {
    let navigate_session_id = pending.navigation.navigate_session_id.clone();
    network::record_main_document_request_body(conn, &pending.navigation);
    let should_handle_auth = conn.target_fetch_matches_auth_required_for_session_owner(
        navigate_session_id.as_deref(),
        &pending.navigation.requested_url,
    ) && pending.navigation.requested_url.scheme() != "data";

    if should_handle_auth {
        let method = pending.navigation.request_method.clone();
        let raw_url = pending.navigation.requested_url.to_string();
        let body = pending.navigation.clone_request_body_bytes();
        let request_headers = pending.navigation.request_headers.clone();
        let has_auth_credentials = auth.is_some();
        if pending.intercept_response && navigation_response_stage_auth_can_stream(auth.as_ref()) {
            match conn
                .fetch_navigation_streaming_raw_response_for_session_owner_async(
                    navigate_session_id.as_deref(),
                    pending.navigation.request_load_policy,
                    &method,
                    &raw_url,
                    body,
                    request_headers,
                    auth,
                )
                .await
            {
                Ok(response) => {
                    let response = append_prior_network_observations(
                        response,
                        prior_network_observation_journal,
                    );
                    handle_streaming_response_head_for_navigation_into_buffer_async(
                        conn, out, pending, response, true,
                    )
                    .await;
                }
                Err(message) => {
                    complete_pending_fetch_navigation_result_into_buffer_async(
                        conn,
                        out,
                        pending,
                        Err(message),
                    )
                    .await;
                }
            }
            return;
        }

        if pending.intercept_response && has_auth_credentials {
            let auth_scheme = auth
                .as_ref()
                .map(|auth| format!("{:?}", auth.scheme))
                .unwrap_or_else(|| "Unknown".to_owned());
            complete_pending_fetch_navigation_result_into_buffer_async(
                conn,
                out,
                pending,
                Err(format!(
                    "Fetch response-stage interception after {auth_scheme} authentication is not supported for navigation without buffering"
                )),
            )
            .await;
            return;
        }

        let response = if let Some(auth) = auth {
            // Digest auth retries are still driven by libcurl internally. The
            // streaming collector sees the intermediate 401 response head
            // before libcurl has completed the credential retry, so keep
            // credential replay on the buffered path until the fetch runtime
            // can mark intermediate auth responses separately.
            conn.fetch_navigation_auth_raw_response_for_session_owner_async(
                navigate_session_id.as_deref(),
                pending.navigation.request_load_policy,
                &method,
                &raw_url,
                body,
                request_headers,
                auth,
            )
            .await
        } else {
            match conn
                .fetch_navigation_streaming_raw_response_for_session_owner_async(
                    navigate_session_id.as_deref(),
                    pending.navigation.request_load_policy,
                    &method,
                    &raw_url,
                    body,
                    request_headers,
                    auth,
                )
                .await
            {
                Ok(response) => collect_navigation_streaming_response(conn, response).await,
                Err(message) => Err(message),
            }
        };
        match response {
            Ok(response) => {
                let response =
                    append_prior_network_observations(response, prior_network_observation_journal);
                let response_head = response.response();
                if matches!(response_head.status, 401 | 407)
                    && let Some(mut challenge) = extract_auth_challenge(&response_head.headers)
                {
                    populate_auth_challenge_origin(
                        conn,
                        pending.navigation.navigate_session_id.as_deref(),
                        &response_head.final_url,
                        &mut challenge,
                    );
                    let event = register_navigation_auth_required_event(
                        conn,
                        &pending,
                        challenge,
                        response_head.request_cookie_report.clone(),
                        response,
                    );
                    out.extend_background_events_after_messages([event]);
                    return;
                }
                if prepare_navigation_response_stage(conn, &mut pending, &response_head.final_url) {
                    pause_buffered_raw_response_stage_navigation_into_buffer(
                        conn, out, pending, response,
                    );
                } else {
                    let navigation = conn
                        .build_navigation_from_buffered_raw_response_for_navigation_async(
                            &pending.navigation,
                            response,
                        )
                        .await;
                    complete_or_pause_response_stage_into_buffer_async(
                        conn, out, pending, navigation,
                    )
                    .await;
                }
            }
            Err(message) => {
                complete_pending_fetch_navigation_result_into_buffer_async(
                    conn,
                    out,
                    pending,
                    Err(message),
                )
                .await;
            }
        }
        return;
    }

    if pending.intercept_response && pending.navigation.requested_url.scheme() != "data" {
        match conn
            .fetch_navigation_streaming_raw_response_for_session_owner_async(
                navigate_session_id.as_deref(),
                pending.navigation.request_load_policy,
                &pending.navigation.request_method,
                pending.navigation.requested_url.as_str(),
                pending.navigation.clone_request_body_bytes(),
                pending.navigation.request_headers.clone(),
                None,
            )
            .await
        {
            Ok(response) => {
                handle_streaming_response_head_for_navigation_into_buffer_async(
                    conn, out, pending, response, false,
                )
                .await;
            }
            Err(message) => {
                complete_pending_fetch_navigation_result_into_buffer_async(
                    conn,
                    out,
                    pending,
                    Err(message),
                )
                .await;
            }
        }
        return;
    }

    if pending.intercept_response && pending.navigation.requested_url.scheme() == "data" {
        pause_data_url_response_stage_navigation_into_buffer(conn, out, pending);
        return;
    }

    let navigation = conn
        .load_navigation_request_via_runtime_with_network_events_for_navigation_async(
            &pending.navigation,
            network::MainDocumentBodyProgressSource::default(),
        )
        .await;
    complete_or_pause_response_stage_into_buffer_async(conn, out, pending, navigation).await;
}

pub(super) async fn load_or_pause_navigation_for_auth_as_background_events_async(
    conn: &mut CdpConnection,
    out: &mut FetchCommandOutput,
    pending: PendingFetchNavigation,
    auth: Option<SubresourceAuthCredentials>,
    prior_network_observation_journal: Option<NetworkObservationJournal>,
) {
    let command_id = pending.navigation.navigate_id;
    let command_session_id = pending.navigation.navigate_session_id.clone();
    let mut output = CommandOutputBuffer::default();
    Box::pin(load_or_pause_navigation_for_auth_into_buffer_async(
        conn,
        &mut output,
        pending,
        auth,
        prior_network_observation_journal,
    ))
    .await;
    out.extend_plan_as_background_events(
        output.into_plan(),
        command_id,
        command_session_id.as_deref(),
    );
}

pub(super) async fn cancel_navigation_auth_as_background_events_async(
    conn: &mut CdpConnection,
    out: &mut FetchCommandOutput,
    pending_auth: crate::conn::PendingFetchAuthNavigation,
) {
    let response = match std::sync::Arc::try_unwrap(pending_auth.auth_response) {
        Ok(response) => response,
        Err(response) => response.as_ref().clone(),
    };
    let mut pending = PendingFetchNavigation {
        fetch_request_id: pending_auth.response_stage_request_id,
        interception_session_id: pending_auth.interception_session_id.clone(),
        document_navigation_token: pending_auth.document_navigation_token,
        navigation: pending_auth.navigation,
        request_cookie_report: pending_auth.request_cookie_report,
        intercept_response: pending_auth.intercept_response,
        response_stage_url_match_policy: pending_auth.response_stage_url_match_policy,
        auth_required_blocked_intercepts: Vec::new(),
    };
    let command_id = pending.navigation.navigate_id;
    let command_session_id = pending.navigation.navigate_session_id.clone();
    let mut output = CommandOutputBuffer::default();
    if response
        .observation_journal()
        .terminal_response_is_failed_proxy_connect()
    {
        let response_head = response.response();
        let progress = network::response_stage_main_document_navigation_network_progress(
            conn,
            &pending.navigation,
            pending.request_cookie_report.as_ref(),
        );
        let mut response_events = Vec::new();
        progress.emit_response_without_extra_info_into_background_events(
            &mut response_events,
            &response_head.final_url,
            response_head.status,
            &response_head.headers,
            false,
        );
        output.extend_background_events_after_messages(response_events);
        complete_pending_fetch_navigation_result_into_buffer_async(
            conn,
            &mut output,
            pending,
            Ok(NavigationLoadOutcome::network_failure(
                "net::ERR_HTTP_RESPONSE_CODE_FAILURE".to_owned(),
            )),
        )
        .await;
        out.extend_plan_as_background_events(
            output.into_plan(),
            command_id,
            command_session_id.as_deref(),
        );
        return;
    }
    if prepare_navigation_response_stage(conn, &mut pending, &response.response().final_url) {
        pause_buffered_raw_response_stage_navigation_into_buffer(
            conn,
            &mut output,
            pending,
            response,
        );
    } else {
        pending.intercept_response = false;
        let navigation = conn
            .build_navigation_from_buffered_raw_response_for_navigation_async(
                &pending.navigation,
                response,
            )
            .await;
        complete_or_pause_response_stage_into_buffer_async(conn, &mut output, pending, navigation)
            .await;
    }
    out.extend_plan_as_background_events(
        output.into_plan(),
        command_id,
        command_session_id.as_deref(),
    );
}

pub(super) async fn complete_tokened_materialized_navigation_as_background_events_async(
    conn: &mut CdpConnection,
    out: &mut FetchCommandOutput,
    token: Option<DocumentNavigationToken>,
    navigation_state: NavigationDispatchState,
    navigation: network::MaterializedNavigationLoadOutcome,
) {
    let command_id = navigation_state.navigate_id;
    let command_session_id = navigation_state.navigate_session_id.clone();
    let mut output = CommandOutputBuffer::default();
    complete_tokened_materialized_navigation_into_buffer_async(
        conn,
        &mut output,
        token,
        navigation_state,
        navigation,
    )
    .await;
    out.extend_plan_as_background_events(
        output.into_plan(),
        command_id,
        command_session_id.as_deref(),
    );
}

pub(super) async fn complete_tokened_materialized_navigation_into_buffer_async(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    token: Option<DocumentNavigationToken>,
    navigation_state: NavigationDispatchState,
    navigation: network::MaterializedNavigationLoadOutcome,
) {
    let Some(token) = token else {
        if navigation_state.navigate_id.is_some() {
            page::push_superseded_navigation_result(out, &navigation_state);
        } else {
            tracing::warn!(
                session_id = navigation_state.navigate_session_id.as_deref(),
                "direct navigation aborted after command result was handled"
            );
        }
        return;
    };
    // Keep this boxed: fetch continue/auth futures compose with main-document
    // navigation commit futures and can exceed nextest's per-test stack when
    // inlined through navigation subresource tests.
    let mut command_context = crate::conn::CommandDispatchContext::default();
    Box::pin(page::complete_materialized_navigation_into_buffer_async(
        conn,
        out,
        token,
        navigation_state,
        navigation,
        &mut command_context,
    ))
    .await;
    if let Some(predecessor) = command_context.take_renderer_output_predecessor() {
        out.set_renderer_output_predecessor(predecessor);
    }
}

async fn complete_pending_fetch_navigation_result_into_buffer_async(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    pending: PendingFetchNavigation,
    navigation: Result<NavigationLoadOutcome, String>,
) {
    let token = pending.document_navigation_token;
    let navigation_state = pending.navigation;
    let navigation =
        network::materialize_navigation_load_result(conn, &navigation_state, navigation);
    complete_tokened_materialized_navigation_into_buffer_async(
        conn,
        out,
        token,
        navigation_state,
        navigation,
    )
    .await;
}

fn navigation_response_stage_auth_can_stream(auth: Option<&SubresourceAuthCredentials>) -> bool {
    match auth {
        None => true,
        Some(auth) => matches!(auth.scheme, SubresourceAuthScheme::Basic),
    }
}

async fn handle_streaming_response_head_for_navigation_into_buffer_async(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    mut pending: PendingFetchNavigation,
    response: NetworkFetchResult<StreamingRawResponse>,
    allow_auth_challenge: bool,
) {
    let response_head = response.response();
    if allow_auth_challenge
        && matches!(response_head.status, 401 | 407)
        && let Some(mut challenge) = extract_auth_challenge(&response_head.headers)
    {
        populate_auth_challenge_origin(
            conn,
            pending.navigation.navigate_session_id.as_deref(),
            &response_head.final_url,
            &mut challenge,
        );
        let request_cookie_report = response_head.request_cookie_report.clone();
        match collect_navigation_streaming_response(conn, response).await {
            Ok(response) => {
                let event = register_navigation_auth_required_event(
                    conn,
                    &pending,
                    challenge,
                    request_cookie_report,
                    response,
                );
                out.extend_background_events_after_messages([event]);
            }
            Err(message) => {
                complete_pending_fetch_navigation_result_into_buffer_async(
                    conn,
                    out,
                    pending,
                    Err(message),
                )
                .await;
            }
        }
        return;
    }

    if response_headers_indicate_attachment_download(&response_head.headers) {
        let navigation = conn
            .build_navigation_from_streaming_raw_response_for_navigation_async(
                &pending.navigation,
                response,
                network::MainDocumentBodyProgressSource::default(),
            )
            .await;
        complete_pending_fetch_navigation_result_into_buffer_async(conn, out, pending, navigation)
            .await;
        return;
    }

    if !prepare_navigation_response_stage(conn, &mut pending, &response_head.final_url) {
        let navigation = conn
            .build_navigation_from_streaming_raw_response_for_navigation_async(
                &pending.navigation,
                response,
                network::MainDocumentBodyProgressSource::default(),
            )
            .await;
        complete_pending_fetch_navigation_result_into_buffer_async(conn, out, pending, navigation)
            .await;
        return;
    }

    let response_status = response_head.status;
    let final_url = response_head.final_url.clone();
    let response_headers = response_head.headers.clone();
    let request_cookie_report = response_head.request_cookie_report.clone();
    let network_observation_journal = response.observation_journal();
    let network_extra_info_available = !network_observation_journal.is_empty();
    let body_progress_source = network::response_stage_main_document_navigation_network_progress(
        conn,
        &pending.navigation,
        pending.request_cookie_report.as_ref(),
    );
    let mut response_extra_info_events = Vec::new();
    body_progress_source.emit_response_extra_info_before_pause(
        &mut response_extra_info_events,
        &pending.navigation.request_method,
        &pending.navigation.request_headers,
        request_cookie_report.as_ref(),
        &response_head.redirect_chain,
        response_status,
        &response_headers,
        &response_head.cookie_set_reports,
        network_observation_journal,
        network_extra_info_available,
    );
    out.extend_background_events_after_messages(response_extra_info_events);
    let prepared_document = match conn
        .prepare_paused_streaming_response_navigation_async(
            &pending.navigation,
            response.response(),
            network_observation_journal,
            body_progress_source.clone(),
        )
        .await
    {
        Ok(prepared_document) => prepared_document,
        Err(error) => {
            complete_pending_fetch_navigation_result_into_buffer_async(
                conn,
                out,
                pending,
                Err(error),
            )
            .await;
            return;
        }
    };
    let (response, network_observation_journal) = response.into_parts_with_observation_journal();
    conn.register_pending_fetch_response_navigation_for_session_owner(
        pending.navigation.navigate_session_id.as_deref(),
        pending.fetch_request_id.clone(),
        pending.document_navigation_token.clone(),
        pending.navigation.clone(),
        DocumentBodySource::StreamingRaw {
            requested_url: pending.navigation.requested_url.clone(),
            request_method: pending.navigation.request_method.clone(),
            request_headers: pending.navigation.request_headers.clone(),
            response,
            network_observation_journal,
            body_progress_source,
            prepared_document: prepared_document.map(Box::new),
        },
    );
    out.extend_background_events_after_messages([navigation_response_stage_request_paused_event(
        pending.interception_session_id.as_deref(),
        &pending.fetch_request_id,
        &pending.navigation,
        &final_url,
        request_cookie_report.as_ref(),
        response_status,
        &response_headers,
    )]);
}

async fn complete_or_pause_response_stage_into_buffer_async(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    pending: PendingFetchNavigation,
    navigation: Result<NavigationLoadOutcome, String>,
) {
    match navigation {
        Ok(NavigationLoadOutcome::Loaded(_)) if pending.intercept_response => {
            debug_assert!(
                false,
                "response-stage document pause should register a body source before building a LoadedNavigation"
            );
            complete_pending_fetch_navigation_result_into_buffer_async(
                conn,
                out,
                pending,
                Err(
                    "response-stage document pause reached unexpected loaded-navigation path"
                        .to_owned(),
                ),
            )
            .await;
        }
        navigation @ Ok(_) => {
            complete_pending_fetch_navigation_result_into_buffer_async(
                conn, out, pending, navigation,
            )
            .await;
        }
        navigation @ Err(_) => {
            complete_pending_fetch_navigation_result_into_buffer_async(
                conn, out, pending, navigation,
            )
            .await;
        }
    }
}

fn pause_data_url_response_stage_navigation_into_buffer(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    pending: PendingFetchNavigation,
) {
    let (content_type, body) =
        match decode_data_url_response(pending.navigation.requested_url.as_str())
            .and_then(Result::ok)
        {
            Some(decoded) => (decoded.content_type, decoded.body),
            None => ("text/plain;charset=US-ASCII".to_owned(), Vec::new()),
        };
    let response = RawResponse::from_head_and_body(
        ResponseHead {
            final_url: pending.navigation.requested_url.clone(),
            status: 200,
            headers: vec![("Content-Type".to_owned(), content_type)],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        },
        body,
    );
    pause_buffered_raw_response_stage_navigation_into_buffer(
        conn,
        out,
        pending,
        NetworkFetchResult::without_request_observation(response),
    );
}

fn pause_buffered_raw_response_stage_navigation_into_buffer(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    pending: PendingFetchNavigation,
    response: NetworkFetchResult<RawResponse>,
) {
    let response_status = response.response().status;
    let final_url = response.response().final_url.clone();
    let response_headers = response.response().headers.clone();
    let request_cookie_report = response.response().request_cookie_report.clone();
    let (response, network_observation_journal) = response.into_parts_with_observation_journal();
    conn.register_pending_fetch_response_navigation_for_session_owner(
        pending.navigation.navigate_session_id.as_deref(),
        pending.fetch_request_id.clone(),
        pending.document_navigation_token.clone(),
        pending.navigation.clone(),
        DocumentBodySource::BufferedRaw {
            requested_url: pending.navigation.requested_url.clone(),
            request_method: pending.navigation.request_method.clone(),
            request_headers: pending.navigation.request_headers.clone(),
            response,
            network_observation_journal,
        },
    );
    out.extend_background_events_after_messages([navigation_response_stage_request_paused_event(
        pending.interception_session_id.as_deref(),
        &pending.fetch_request_id,
        &pending.navigation,
        &final_url,
        request_cookie_report.as_ref(),
        response_status,
        &response_headers,
    )]);
}

async fn collect_navigation_streaming_response(
    conn: &mut CdpConnection,
    response: NetworkFetchResult<StreamingRawResponse>,
) -> Result<NetworkFetchResult<RawResponse>, String> {
    conn.collect_navigation_streaming_raw_response_async(response)
        .await
}

fn append_prior_network_observations<R>(
    response: NetworkFetchResult<R>,
    prior: Option<NetworkObservationJournal>,
) -> NetworkFetchResult<R> {
    let Some(mut prior) = prior else {
        return response;
    };
    let (response, current) = response.into_parts_with_observation_journal();
    prior.append(current);
    NetworkFetchResult::with_observation_journal(response, prior)
}

pub(crate) async fn continue_navigation_without_request_pause_into_buffer_async(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    pending: PendingFetchNavigation,
) {
    Box::pin(load_or_pause_navigation_for_auth_into_buffer_async(
        conn, out, pending, None, None,
    ))
    .await;
}

pub(crate) async fn continue_subresource_without_fetch_pause_async(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    request_id: Option<String>,
    page_owner: crate::conn::TargetPageResidenceIdentity,
    internal_id: u64,
    network_request_id: String,
    network_request_handle: Option<moli_core::page::SubresourceNetworkRequestHandle>,
    frame_id: String,
    document_url: Url,
    resource_type: SubresourceResourceType,
    handle_auth_requests: bool,
    owner_kind: PendingSubresourceFetchOwnerKind,
) {
    let pending = PendingSubresourceFetchRequest {
        residence: crate::conn::PendingSubresourceFetchResidence::InstalledPage(page_owner),
        owner_session_id: None,
        action_session_id: None,
        owner_kind,
        internal_id,
        network_request_id,
        network_request_handle,
        frame_id,
        document_url,
        resource_type,
        websocket_socket_id: None,
        request_stage_chain: None,
    };
    if conn
        .continue_pending_subresource_fetch_for_session_owner_async(
            session_id,
            internal_id,
            None,
            None,
            None,
            None,
            false,
            handle_auth_requests,
        )
        .await
        .is_ok()
    {
        conn.register_in_flight_subresource_fetch_request_for_session_owner(
            session_id, request_id, pending,
        );
    }
}

fn navigation_auth_required_blocked_intercepts(
    conn: &CdpConnection,
    pending: &PendingFetchNavigation,
) -> Vec<DevToolsNetworkInterceptId> {
    if !pending.auth_required_blocked_intercepts.is_empty() {
        return pending.auth_required_blocked_intercepts.clone();
    }
    let intercepts = conn.target_fetch_matching_auth_required_network_intercepts_for_session_owner(
        pending.navigation.navigate_session_id.as_deref(),
        &pending.navigation.requested_url,
    );
    if !intercepts.is_empty() {
        return intercepts;
    }
    conn.target_fetch_matching_auth_required_network_intercepts_for_target(
        pending.navigation.frame_id.as_str(),
        &pending.navigation.requested_url,
    )
}

fn stage_blocked_intercepts(
    owner_kind: PendingSubresourceFetchOwnerKind,
    stage_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
    fallback_blocked_intercepts: &[DevToolsNetworkInterceptId],
) -> Vec<DevToolsNetworkInterceptId> {
    if !stage_blocked_intercepts.is_empty() {
        return stage_blocked_intercepts;
    }
    match owner_kind {
        PendingSubresourceFetchOwnerKind::Fetch => Vec::new(),
        PendingSubresourceFetchOwnerKind::NetworkOrBidi => fallback_blocked_intercepts.to_vec(),
    }
}

fn routable_stage_owner_session_id(
    conn: &CdpConnection,
    fallback_session_id: Option<&str>,
    stage_session_id: Option<&str>,
) -> Option<String> {
    if let Some(stage_session_id) = stage_session_id
        && conn.session_route(Some(stage_session_id)).is_some()
    {
        return Some(stage_session_id.to_owned());
    }
    fallback_session_id.map(str::to_owned)
}

fn register_navigation_auth_required_event(
    conn: &mut CdpConnection,
    pending: &PendingFetchNavigation,
    challenge: FetchAuthChallenge,
    request_cookie_report: Option<StoredCookieQueryReport>,
    response: NetworkFetchResult<RawResponse>,
) -> crate::conn::BackgroundProtocolEvent {
    let blocked_intercepts = navigation_auth_required_blocked_intercepts(conn, pending);
    let mut pending_auth = crate::conn::PendingFetchAuthNavigation {
        owner_session_id: pending.navigation.navigate_session_id.clone(),
        action_session_id: pending.interception_session_id.clone(),
        interception_session_id: pending.interception_session_id.clone(),
        owner_kind: PendingSubresourceFetchOwnerKind::Fetch,
        fetch_request_id: pending.fetch_request_id.clone(),
        response_stage_request_id: pending.fetch_request_id.clone(),
        document_navigation_token: pending.document_navigation_token.clone(),
        navigation: pending.navigation.clone(),
        request_cookie_report,
        auth_response: std::sync::Arc::new(response),
        challenge,
        intercept_response: pending.intercept_response,
        response_stage_url_match_policy: pending.response_stage_url_match_policy,
        auth_stage_chain: None,
    };
    let mut auth_event_session_id = pending.interception_session_id.clone();
    let mut auth_event_blocked_intercepts = blocked_intercepts.clone();
    let auth_sessions = conn
        .target_fetch_subresource_interception_snapshot_for_target(&pending.navigation.frame_id)
        .or_else(|| {
            conn.target_fetch_subresource_interception_snapshot_for_session_owner(
                pending.navigation.navigate_session_id.as_deref(),
            )
        })
        .map(|snapshot| {
            snapshot.matching_auth_required_pause_sessions(
                pending.navigation.navigate_session_id.as_deref(),
                &pending.navigation.requested_url,
            )
        })
        .unwrap_or_default();
    if let Some(first_pause_session) = auth_sessions.first().cloned() {
        pending_auth.owner_session_id = routable_stage_owner_session_id(
            conn,
            pending.navigation.navigate_session_id.as_deref(),
            first_pause_session.session_id.as_deref(),
        );
        pending_auth.owner_kind = first_pause_session.owner_kind;
        auth_event_session_id = first_pause_session.session_id.clone();
        auth_event_blocked_intercepts = stage_blocked_intercepts(
            first_pause_session.owner_kind,
            first_pause_session.blocked_intercepts,
            &blocked_intercepts,
        );
        let mut remaining_sessions = Vec::new();
        for session in auth_sessions.into_iter().skip(1) {
            let Ok(next_request_id) = conn.allocate_fetch_navigation_request_id_for_session_owner(
                pending.navigation.navigate_session_id.as_deref(),
            ) else {
                break;
            };
            remaining_sessions.push(PendingSubresourceFetchAuthStage {
                session_id: session.session_id,
                owner_kind: session.owner_kind,
                request_id: next_request_id,
                blocked_intercepts: stage_blocked_intercepts(
                    session.owner_kind,
                    session.blocked_intercepts,
                    &blocked_intercepts,
                ),
            });
        }
        if !remaining_sessions.is_empty() {
            pending_auth.auth_stage_chain = Some(Box::new(PendingSubresourceFetchAuthStageChain {
                remaining_sessions,
            }));
        }
    }
    pending_auth.action_session_id = auth_event_session_id.clone();
    let pending_owner_session_id = pending_auth
        .owner_session_id
        .as_deref()
        .or(pending.navigation.navigate_session_id.as_deref());
    conn.register_pending_fetch_auth_navigation_for_session_owner(
        pending_owner_session_id,
        pending.fetch_request_id.clone(),
        pending_auth.clone(),
    );
    pending_fetch_auth_navigation_required_event(
        auth_event_session_id.as_deref(),
        &pending_auth,
        &auth_event_blocked_intercepts,
    )
}

pub(crate) async fn continue_subresource_for_response_stage_async(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    request_id: String,
    pending: PendingSubresourceFetchRequest,
    handle_auth_requests: bool,
    response_stage_blocked_intercepts: Vec<DevToolsNetworkInterceptId>,
) {
    if conn
        .continue_pending_subresource_fetch_for_session_owner_async(
            session_id,
            pending.internal_id,
            None,
            None,
            None,
            None,
            true,
            handle_auth_requests,
        )
        .await
        .is_ok()
    {
        conn.register_in_flight_response_stage_subresource_fetch_request_for_session_owner(
            session_id,
            Some(request_id),
            pending,
            response_stage_blocked_intercepts,
        );
    }
}

pub(crate) async fn continue_subresource_for_deferred_response_stage_async(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    request_id: String,
    pending: PendingSubresourceFetchRequest,
    handle_auth_requests: bool,
) {
    if conn
        .continue_pending_subresource_fetch_for_session_owner_async(
            session_id,
            pending.internal_id,
            None,
            None,
            None,
            None,
            true,
            handle_auth_requests,
        )
        .await
        .is_ok()
    {
        conn.register_in_flight_deferred_response_stage_subresource_fetch_request_for_session_owner(
            session_id,
            Some(request_id),
            pending,
        );
    }
}

#[cfg(test)]
mod tests {
    use moli_web_mime::response_headers_indicate_attachment_download;

    #[test]
    fn response_stage_navigation_download_detection_uses_web_mime_attachment_helper() {
        assert!(response_headers_indicate_attachment_download(&[(
            "Content-Disposition".to_owned(),
            "attachment; filename=report.html".to_owned(),
        )]));

        assert!(!response_headers_indicate_attachment_download(&[(
            "Content-Disposition".to_owned(),
            "inline; filename=attachment.html".to_owned(),
        )]));
    }
}
