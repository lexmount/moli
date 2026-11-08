use anyhow::{Context, Result};

use crate::conn::{
    CdpConnection, NavigationLoadOutcome, PendingFetchNavigation,
    PendingStreamingDocumentResponseNavigation, PreparedCapturedNavigationResponse,
    ProtocolWorkflowDriver,
};
use crate::domains::{
    command_output::CommandOutputBuffer, command_output::CommandOutputPlan, network,
};
use moli_core::network::NavigationInterception;
use moli_core::page::SubresourceAuthCredentials;
use moli_fetch::{
    NetworkFetchResult, NetworkObservationJournal, RawResponse, StreamingRawResponse,
};

use super::{
    helpers::{extract_auth_challenge, populate_auth_challenge_origin},
    navigation::{self, NavigationResponseTransfer},
};

/// The paused navigation's request and retry budget leave shared state
/// together. Network headers, Digest replay and challenge bodies are ordinary
/// awaits; only interpreting the accepted response needs another state turn.
pub(crate) struct PreparedInterceptedNavigationRequest {
    pending: PendingFetchNavigation,
    request: Option<Result<NavigationInterception>>,
    renderer_runtime: Option<moli_core::runtime::RendererBrowserContextRuntime>,
    buffered: bool,
    handle_auth: bool,
    prior_journal: Option<NetworkObservationJournal>,
    transferred: NavigationResponseTransfer,
}

#[derive(Debug, thiserror::Error)]
#[error(
    "Fetch response-stage interception after {0:?} authentication is not supported for navigation without buffering"
)]
struct UnsupportedNavigationAuthInterception(moli_core::page::SubresourceAuthScheme);

pub(crate) enum FetchNavigationResponse {
    Streaming(NetworkFetchResult<StreamingRawResponse>),
    Captured(NetworkFetchResult<RawResponse>),
}

pub(crate) enum PreparedFetchNavigationResponse {
    Captured(Box<Result<PreparedCapturedNavigationResponse>>),
    Streaming(Box<Result<crate::conn::PreparedStreamingNavigationResponse>>),
    Result(Result<NavigationLoadOutcome>),
    Transferred(CommandOutputPlan),
    Superseded,
}

impl PreparedInterceptedNavigationRequest {
    pub(crate) fn new(
        conn: &mut CdpConnection,
        mut pending: PendingFetchNavigation,
        auth: Option<SubresourceAuthCredentials>,
        prior_journal: Option<NetworkObservationJournal>,
        transferred: NavigationResponseTransfer,
    ) -> Self {
        let handle_auth = conn.target_fetch_matches_auth_required_for_owner(
            &pending.navigation.owner,
            &pending.navigation.requested_url,
        );
        let buffered = !navigation::navigation_response_stage_auth_can_stream(auth.as_ref());
        let mut prior_journal = prior_journal;
        let renderer_runtime = conn
            .target_owner_identity_for_owner(&pending.navigation.owner)
            .and_then(|(id, _)| conn.browser_context_by_id(&id))
            .map(|context| context.renderer_runtime());
        let request = if auth.is_some() && !pending.auth_retry_budget.try_restart() {
            None
        } else if handle_auth && buffered && pending.intercept_response {
            // Keep the existing protocol limitation; do not expose libcurl's
            // intermediate Digest challenge as a final streaming response.
            Some(Err(UnsupportedNavigationAuthInterception(
                auth.as_ref().expect("buffered authentication").scheme,
            )
            .into()))
        } else {
            network::record_main_document_request_body(conn, &pending.navigation);
            let prepared = if let Some((mut interception, journal)) = pending.interception.take() {
                prior_journal = Some(navigation::merge_network_observations(
                    prior_journal,
                    journal,
                ));
                interception.apply_request_override(
                    pending.navigation.requested_url.clone(),
                    pending.navigation.request_method.clone(),
                    pending.navigation.request_headers.clone(),
                    pending.navigation.clone_request_body_bytes(),
                    pending.navigation.redirect_headers.clone(),
                );
                if let Some(auth) = auth {
                    interception.request_mut().set_auth(Some(auth.into()));
                }
                Ok(interception)
            } else {
                conn.prepare_navigation_request(
                    pending
                        .document_navigation_token
                        .as_ref()
                        .expect("request token"),
                    &pending.navigation,
                    auth,
                )
            };
            Some(prepared.map(|mut prepared| {
                let intercept = conn
                    .target_fetch_subresource_interception_snapshot_for_owner(
                        &pending.navigation.owner,
                    )
                    .is_some_and(|snapshot| {
                        snapshot.has_request_stage_candidate(
                            crate::automation::DevToolsNetworkResourceType::Document,
                        )
                    });
                prepared.set_manual_redirects(intercept);
                prepared
            }))
        };
        Self {
            pending,
            request,
            renderer_runtime,
            buffered,
            handle_auth,
            prior_journal,
            transferred,
        }
    }

    pub(crate) async fn prepare(
        mut self,
        driver: &mut ProtocolWorkflowDriver,
    ) -> PreparedFetchNavigationResponse {
        let Some(request) = self.request.take() else {
            return PreparedFetchNavigationResponse::Result(Ok(
                NavigationLoadOutcome::network_failure(
                    moli_fetch::AuthRetryBudget::ERROR_TEXT.to_owned(),
                ),
            ));
        };
        let mut request = match request {
            Ok(request) => request,
            Err(error) => return PreparedFetchNavigationResponse::Result(Err(error)),
        };
        let response = loop {
            let service_worker_response = if let Some(runtime) = &self.renderer_runtime {
                match runtime
                    .fetch_service_worker_main_resource_for_navigation(
                        request.request(),
                        request.resource_loader(),
                    )
                    .await
                {
                    Ok(fetch) => {
                        self.pending.reserved_service_worker_client = fetch.reserved_client;
                        fetch.response
                    }
                    Err(error) => break Err(error),
                }
            } else {
                None
            };
            let response = if self.buffered && service_worker_response.is_none() {
                request
                    .fetch_buffered()
                    .await
                    .with_context(|| format!("failed to fetch page `{}`", request.request().url))
                    .map(FetchNavigationResponse::Captured)
            } else {
                let response = if let Some(response) = service_worker_response {
                    request
                        .resource_loader()
                        .note_service_worker_response_ready()
                        .and_then(|()| {
                            moli_core::runtime::streaming_raw_response_from_navigation_response(
                                response,
                            )
                        })
                        .map(NetworkFetchResult::without_request_observation)
                } else {
                    request.fetch_raw_stream().await
                };
                match response
                    .with_context(|| format!("failed to fetch page `{}`", request.request().url))
                {
                    Ok(response)
                        if moli_web_mime::response_headers_indicate_attachment_download(
                            &response.response().headers,
                        ) || (self.handle_auth
                            && matches!(response.response().status, 401 | 407)
                            && extract_auth_challenge(&response.response().headers).is_some()) =>
                    {
                        let (response, journal) = response.into_parts_with_observation_journal();
                        response
                            .into_materialized_raw_response()
                            .await
                            .map(|response| {
                                FetchNavigationResponse::Captured(
                                    NetworkFetchResult::with_observation_journal(response, journal),
                                )
                            })
                            .map_err(|error| error.context("failed to read page body from stream"))
                    }
                    response => response.map(FetchNavigationResponse::Streaming),
                }
            };
            let head = match &response {
                Ok(FetchNavigationResponse::Streaming(response)) => {
                    Some(response.response().head())
                }
                Ok(FetchNavigationResponse::Captured(response)) => Some(response.response().head()),
                Err(_) => None,
            };
            match head.as_ref().map(|head| request.follow_redirect(head)) {
                Some(Ok(Some(_))) => {}
                Some(Err(error)) => break Err(error),
                _ => break response,
            }
            let journal = match response.expect("redirect has a response") {
                FetchNavigationResponse::Streaming(response) => {
                    response.into_parts_with_observation_journal().1
                }
                FetchNavigationResponse::Captured(response) => {
                    response.into_parts_with_observation_journal().1
                }
            };
            self.prior_journal = Some(navigation::merge_network_observations(
                self.prior_journal.take(),
                journal,
            ));
            let hop = request.request();
            self.pending.navigation.requested_url = hop.url.clone();
            self.pending.navigation.request_method = hop.method.clone();
            self.pending.navigation.request_headers = hop.request_headers.clone();
            self.pending.navigation.request_body_bytes = hop.body.clone();
            self.pending.navigation.request_body = hop
                .body
                .as_ref()
                .map(|body| String::from_utf8_lossy(body).into_owned());
            self.pending.navigation.redirect_chain = hop.redirect_chain().to_vec();
            self.pending.navigation.redirect_headers = None;
            let pending = self.pending;
            let prior = self.prior_journal.take().unwrap_or_default();
            let redirected = driver.with_state(move |state| {
                state.prepare_fetch_navigation_redirect(pending, request, prior)
            });
            match redirected {
                Ok(RedirectNavigation::Continue(next)) => {
                    let (pending, executor, prior) = *next;
                    self.pending = pending;
                    request = executor;
                    self.prior_journal = Some(prior);
                }
                Ok(RedirectNavigation::Paused(plan)) => {
                    self.transferred.mark_transferred();
                    return PreparedFetchNavigationResponse::Transferred(*plan);
                }
                Ok(RedirectNavigation::Superseded) => {
                    return PreparedFetchNavigationResponse::Superseded;
                }
                Err(error) => return PreparedFetchNavigationResponse::Result(Err(error)),
            }
        };
        let result = driver.with_state(move |state| {
            let result = state.accept_fetch_navigation_response(
                self.pending,
                response,
                self.handle_auth,
                self.prior_journal,
            );
            if matches!(result, PreparedFetchNavigationResponse::Transferred(_)) {
                // The original pause map or background navigation owns the
                // source reply now, even if this command is cancelled before
                // its completion is consumed by the actor.
                self.transferred.mark_transferred();
            }
            result
        });
        result.unwrap_or_else(|error| PreparedFetchNavigationResponse::Result(Err(error)))
    }
}

pub(crate) fn accept_navigation_response(
    conn: &mut CdpConnection,
    mut pending: PendingFetchNavigation,
    response: Result<FetchNavigationResponse>,
    handle_auth: bool,
    prior_journal: Option<NetworkObservationJournal>,
) -> PreparedFetchNavigationResponse {
    let token = pending
        .document_navigation_token
        .as_ref()
        .expect("request token")
        .clone();
    if !conn.accepts_pending_document_navigation_for_owner(&pending.navigation.owner, &token) {
        return PreparedFetchNavigationResponse::Superseded;
    }
    let response = match response {
        Ok(response) => response,
        Err(error) => return PreparedFetchNavigationResponse::Result(Err(error)),
    };
    let mut output = CommandOutputBuffer::default();
    match response {
        FetchNavigationResponse::Captured(response) => {
            let response = navigation::append_prior_network_observations(response, prior_journal);
            let head = response.response();
            if handle_auth
                && matches!(head.status, 401 | 407)
                && let Some(mut challenge) = extract_auth_challenge(&head.headers)
            {
                populate_auth_challenge_origin(
                    conn,
                    pending.navigation.owner.session_id(),
                    &head.final_url,
                    &mut challenge,
                );
                let event = navigation::register_navigation_auth_required_event(
                    conn,
                    &pending,
                    challenge,
                    head.request_cookie_report.clone(),
                    response,
                );
                output.extend_background_events_after_messages([event]);
            } else if navigation::prepare_navigation_response_stage(
                conn,
                &mut pending,
                &head.final_url,
            ) {
                navigation::pause_buffered_raw_response_stage_navigation_into_buffer(
                    conn,
                    &mut output,
                    pending,
                    response,
                );
            } else {
                let (response, journal) = response.into_parts_with_observation_journal();
                let (head, body) = response.into_body();
                return PreparedFetchNavigationResponse::Captured(Box::new(
                    conn.prepare_captured_navigation_response(
                        Some(&token),
                        &pending.navigation,
                        head,
                        crate::conn::CapturedBody::from_bytes(
                            body.try_into_materialized_bytes()
                                .expect("captured request body"),
                        ),
                        journal,
                        network::MainDocumentBodyProgressSource::default(),
                    )
                    .map(|response| {
                        response.with_service_worker_client(
                            pending.reserved_service_worker_client.take(),
                        )
                    }),
                ));
            }
        }
        FetchNavigationResponse::Streaming(response) => {
            let response = navigation::append_prior_network_observations(response, prior_journal);
            if navigation::prepare_navigation_response_stage(
                conn,
                &mut pending,
                &response.response().final_url,
            ) {
                navigation::pause_streaming_response_stage_navigation_into_buffer(
                    conn,
                    &mut output,
                    pending,
                    response,
                );
            } else {
                let Some(sender) = conn
                    .background_navigation_completion_sender_for_owner(&pending.navigation.owner)
                else {
                    return PreparedFetchNavigationResponse::Streaming(Box::new(
                        conn.prepare_streaming_navigation_response(
                            Some(&token),
                            &pending.navigation,
                            response,
                            network::MainDocumentBodyProgressSource::default(),
                            None,
                            Vec::new(),
                        )
                        .map(|response| {
                            response.with_service_worker_client(
                                pending.reserved_service_worker_client.take(),
                            )
                        }),
                    ));
                };
                let (response, network_observation_journal) =
                    response.into_parts_with_observation_journal();
                super::commands::continue_streaming_document_response_in_background(
                    conn,
                    sender,
                    PendingStreamingDocumentResponseNavigation {
                        reserved_service_worker_client: pending
                            .reserved_service_worker_client
                            .take(),
                        document_navigation_token: pending
                            .document_navigation_token
                            .expect("request token"),
                        navigation: pending.navigation,
                        response,
                        network_observation_journal,
                        body_progress_source: network::MainDocumentBodyProgressSource::default(),
                    },
                    None,
                    Vec::new(),
                );
            }
        }
    }
    PreparedFetchNavigationResponse::Transferred(output.into_plan())
}

pub(crate) enum RedirectNavigation {
    Continue(
        Box<(
            PendingFetchNavigation,
            NavigationInterception,
            NetworkObservationJournal,
        )>,
    ),
    Paused(Box<crate::domains::command_output::CommandOutputPlan>),
    Superseded,
}

pub(crate) fn prepare_navigation_redirect(
    conn: &mut CdpConnection,
    mut pending: PendingFetchNavigation,
    executor: moli_core::network::NavigationInterception,
    prior: NetworkObservationJournal,
) -> RedirectNavigation {
    if !pending
        .document_navigation_token
        .as_ref()
        .is_some_and(|token| {
            conn.accepts_pending_document_navigation_for_owner(&pending.navigation.owner, token)
        })
    {
        return RedirectNavigation::Superseded;
    }
    let Some((id, _)) =
        conn.allocate_pending_subresource_fetch_request_ids_for_owner(&pending.navigation.owner)
    else {
        return RedirectNavigation::Superseded;
    };
    pending.fetch_request_id = id;
    // The same selector and per-handler hop IDs serve root and child Documents.
    let paused = super::prepare_navigation_request_pause(conn, &mut pending);
    let mut plan = crate::domains::command_output::CommandOutputPlan::default();
    // Freeze this hop's method, headers and body before another redirect can
    // change them, even when no Fetch handler pauses this request.
    append_redirect_request_events(conn, &mut plan, &mut pending);
    if !paused {
        if let Some(sender) = conn.background_event_sender() {
            // Initial navigation output uses this same channel. Preserve its
            // FIFO before publishing subsequent redirect requests and pauses.
            for event in plan.into_background_events(None, None) {
                let _ = sender.send(event);
            }
        } else {
            conn.publish_renderer_fenced_protocol_output_after(
                plan.into_background_events(None, None),
                Vec::new(),
                Some(pending.navigation.frame_id.clone()),
            );
        }
        return RedirectNavigation::Continue(Box::new((pending, executor, prior)));
    }
    pending.interception = Some((executor, prior));
    let owner = pending.navigation.owner.clone();
    let event = super::request_paused_background_event(
        pending.interception_session_id.as_deref(),
        &pending,
    );
    if conn
        .register_pending_fetch_navigation_request_for_owner(&owner, pending)
        .is_none()
    {
        return RedirectNavigation::Superseded;
    }
    plan.push_background_event(event);
    if let Some(sender) = conn.background_event_sender() {
        for event in plan.into_background_events(None, None) {
            let _ = sender.send(event);
        }
        plan = crate::domains::command_output::CommandOutputPlan::default();
    }
    RedirectNavigation::Paused(Box::new(plan))
}

fn append_redirect_request_events(
    conn: &CdpConnection,
    plan: &mut crate::domains::command_output::CommandOutputPlan,
    pending: &mut PendingFetchNavigation,
) {
    let nav = &pending.navigation;
    let Some(network_id) = &nav.request_id else {
        return;
    };
    let info = moli_core::page::PendingSubresourceFetchInfo {
        internal_id: 0,
        network_request_handle: None,
        frame_id: Some(nav.frame_id.clone()),
        document_url: nav.requested_url.clone(),
        url: nav.requested_url.clone(),
        websocket_socket_id: None,
        method: nav.request_method.clone(),
        request_headers: nav.request_headers.clone(),
        request_body: nav.request_body.clone(),
        request_body_bytes: nav.clone_request_body_bytes(),
        resource_type: moli_core::page::SubresourceResourceType::Document,
        request_cookie_report: pending.request_cookie_report.clone(),
    };
    for redirect in nav.redirect_chain.iter().skip(nav.announced_redirects) {
        let mut info = info.clone();
        info.url = redirect.to_url.clone();
        let output = network::TargetSubresourceFetchPauseNetworkOutput::from_pending_fetch_info(
            network_id.clone(),
            nav.frame_id.clone(),
            nav.loader_id.clone(),
            crate::conn::monotonic_timestamp_seconds(),
            nav.requested_url.clone(),
            &info,
        )
        .with_child_document(Some(moli_core::page::ChildDocumentFetchRequest {
            loader_id: nav.loader_id.clone(),
            redirect: Some(redirect.clone().into()),
        }));
        for session in conn.network_event_session_ids_for_owner(&nav.owner) {
            plan.extend_background_events(
                network::fetch_subresource_initial_request_network_events(
                    session.as_deref(),
                    &output,
                ),
            );
        }
    }
    pending.navigation.announced_redirects = pending.navigation.redirect_chain.len();
}
