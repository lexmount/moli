use anyhow::Result;

use crate::conn::{
    CdpConnection, NavigationLoadOutcome, PendingFetchNavigation,
    PendingStreamingDocumentResponseNavigation, PreparedCapturedNavigationResponse,
    PreparedNavigationRequest, ProtocolWorkflowDriver,
};
use crate::domains::{
    command_output::CommandOutputBuffer, command_output::CommandOutputPlan, network,
};
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
    request: Option<Result<PreparedNavigationRequest>>,
    buffered: bool,
    handle_auth: bool,
    prior_journal: Option<NetworkObservationJournal>,
    transferred: NavigationResponseTransfer,
}

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
        let request = if auth.is_some() && !pending.auth_retry_budget.try_restart() {
            None
        } else if handle_auth && buffered && pending.intercept_response {
            // Keep the existing protocol limitation; do not expose libcurl's
            // intermediate Digest challenge as a final streaming response.
            Some(Err(anyhow::anyhow!(
                "Fetch response-stage interception after {:?} authentication is not supported for navigation without buffering",
                auth.as_ref().expect("buffered authentication").scheme,
            )))
        } else {
            network::record_main_document_request_body(conn, &pending.navigation);
            Some(
                conn.prepare_navigation_request(
                    pending
                        .document_navigation_token
                        .as_ref()
                        .expect("request token"),
                    &pending.navigation,
                    auth,
                ),
            )
        };
        Self {
            pending,
            request,
            buffered,
            handle_auth,
            prior_journal,
            transferred,
        }
    }

    pub(crate) async fn prepare(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
    ) -> PreparedFetchNavigationResponse {
        let Some(request) = self.request else {
            return PreparedFetchNavigationResponse::Result(Ok(
                NavigationLoadOutcome::network_failure(
                    moli_fetch::AuthRetryBudget::ERROR_TEXT.to_owned(),
                ),
            ));
        };
        let response = match request {
            Ok(request) if self.buffered => request
                .wait_buffered()
                .await
                .map(FetchNavigationResponse::Captured),
            Ok(request) => match request.wait().await {
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
            },
            Err(error) => Err(error),
        };
        let result = driver
            .with_state(move |state| {
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
            })
            .await;
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
                    ),
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
                        ),
                    ));
                };
                let (response, network_observation_journal) =
                    response.into_parts_with_observation_journal();
                super::commands::continue_streaming_document_response_in_background(
                    conn,
                    sender,
                    PendingStreamingDocumentResponseNavigation {
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
