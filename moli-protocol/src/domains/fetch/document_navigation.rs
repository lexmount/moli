use anyhow::Result;

use crate::conn::{
    CdpConnection, CommandDispatchContext, CommandOwnerScope, CommittedNavigationPage,
    DocumentNavigationToken, LoadedNavigation, NavigationDispatchState, NavigationLoadOutcome,
    PreparedCapturedNavigationResponse, PreparedDocumentCommitState, ProtocolStateAccess,
    ProtocolWorkflowDriver,
};
use crate::domains::{command_output::CommandOutputBuffer, network, page};
use moli_core::page::RendererPageReplacementError;

use super::{
    FetchCommandOutput, FetchCommandTaskStep, PendingFetchCommandDispatch, PendingFetchCommandKind,
    PendingFetchCommandOperation,
    commands::{PendingContinueResponseState, PendingFulfillRequestState},
    navigation::NavigationTermination,
};

pub(super) struct PendingDocumentNavigation {
    access: ProtocolStateAccess,
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    response: DocumentNavigationResponse,
    commit: PreparedDocumentCommitState,
    prefix: Vec<crate::conn::BackgroundProtocolEvent>,
}

pub(super) struct CompletedDocumentNavigation {
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    commit: PreparedDocumentCommitState,
    outcome: DocumentNavigationOutcome,
    prefix: Vec<crate::conn::BackgroundProtocolEvent>,
}

impl CompletedDocumentNavigation {
    pub(super) fn has_prepared_commit(&self) -> bool {
        matches!(self.outcome, DocumentNavigationOutcome::Committed(_))
    }
}

pub(super) fn complete_owned(
    access: ProtocolStateAccess,
    completed: CompletedDocumentNavigation,
    termination: Option<NavigationTermination>,
) -> crate::conn::PreparedCommandWorkflow {
    use crate::domains::command_output::CommandOutputPlan;
    let DocumentNavigationOutcome::Committed(navigation) = completed.outcome else {
        unreachable!("owned protocol tail follows a prepared native commit");
    };
    let completion = completed
        .commit
        .complete(completed.token, completed.state, *navigation);
    // Source cancellation follows resources already admitted by this tail;
    // its inline completion must not get ahead of worker cleanup needing actor progress.
    let termination = std::rc::Rc::new(std::cell::RefCell::new(termination));
    let failure_termination = termination.clone();
    let cleanup = page::NavigationProtocolCleanup::default();
    let failure_cleanup = cleanup.clone();
    let failure_access = access.clone();
    crate::conn::PreparedCommandWorkflow::new(move |mut context| async move {
        let mut prefix = CommandOutputPlan::default();
        prefix.extend_background_events(completed.prefix);
        context.retain_output(prefix);
        let mut driver = ProtocolWorkflowDriver::owned(access);
        let admitted_termination = termination.clone();
        let prepared = context
            .with_state(&mut driver, move |state, context| {
                let prepared = state.prepare_navigation_protocol_completion(completion, context);
                if prepared.source_terminal_is_retained()
                    && let Some(mut termination) = admitted_termination.borrow_mut().take()
                {
                    termination.disarm();
                }
                prepared
            })
            .await;
        match prepared {
            Ok(prepared) => {
                prepared
                    .run(&mut driver, context, cleanup, move || {
                        if let Some(mut termination) = termination.borrow_mut().take() {
                            termination.disarm();
                        }
                    })
                    .await
            }
            Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
        }
    })
    .with_failure_cleanup(move |response| {
        crate::conn::PreparedCommandWorkflow::new(move |mut context| async move {
            failure_cleanup
                .recover(
                    &mut ProtocolWorkflowDriver::owned(failure_access),
                    &mut context,
                )
                .await;
            drop(failure_termination.borrow_mut().take());
            context.complete(response)
        })
    })
}

enum DocumentNavigationOutcome {
    Committed(Box<Result<LoadedNavigation<CommittedNavigationPage>, RendererPageReplacementError>>),
    Other(Result<NavigationLoadOutcome>),
    Transferred(crate::domains::command_output::CommandOutputPlan),
    Superseded,
}

enum DocumentNavigationResponse {
    Captured(Box<Result<PreparedCapturedNavigationResponse>>),
    Request(Box<super::document_request::PreparedInterceptedNavigationRequest>),
    Result(Result<NavigationLoadOutcome>),
}

impl PendingDocumentNavigation {
    // Preparation, admission, native execution, adoption and body completion
    // remain one ordinary async workflow in the original command's wait task.
    pub(super) async fn wait(self) -> CompletedDocumentNavigation {
        let mut driver = ProtocolWorkflowDriver::owned(self.access.clone());
        self.wait_using(&mut driver).await
    }

    pub(super) async fn wait_using(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
    ) -> CompletedDocumentNavigation {
        let response = match self.response {
            DocumentNavigationResponse::Captured(response) => {
                super::document_request::PreparedFetchNavigationResponse::Captured(response)
            }
            DocumentNavigationResponse::Request(request) => request.prepare(driver).await,
            DocumentNavigationResponse::Result(response) => {
                super::document_request::PreparedFetchNavigationResponse::Result(response)
            }
        };
        let response = match response {
            super::document_request::PreparedFetchNavigationResponse::Captured(response) => {
                match *response {
                    Ok(response) => response.run().await,
                    Err(error) => Err(error),
                }
            }
            super::document_request::PreparedFetchNavigationResponse::Streaming(response) => {
                match *response {
                    Ok(response) => response.run(None).await,
                    Err(error) => Err(error),
                }
            }
            super::document_request::PreparedFetchNavigationResponse::Result(response) => response,
            super::document_request::PreparedFetchNavigationResponse::Transferred(plan) => {
                return CompletedDocumentNavigation {
                    token: self.token,
                    state: self.state,
                    commit: self.commit,
                    outcome: DocumentNavigationOutcome::Transferred(plan),
                    prefix: self.prefix,
                };
            }
            super::document_request::PreparedFetchNavigationResponse::Superseded => {
                return CompletedDocumentNavigation {
                    token: self.token,
                    state: self.state,
                    commit: self.commit,
                    outcome: DocumentNavigationOutcome::Superseded,
                    prefix: self.prefix,
                };
            }
        };
        let response = match response {
            Err(error) => {
                let token = self.token.clone();
                let state = self.state.clone();
                match driver
                    .with_state(move |state_access| {
                        state_access.prepare_fetch_navigation_error(token, state, error)
                    })
                    .await
                {
                    Ok(Ok(error_document)) => error_document.run().await,
                    Ok(Err(error)) | Err(error) => Err(error),
                }
            }
            response => response,
        };
        let outcome = match response {
            Ok(NavigationLoadOutcome::ResponseCommitReady(navigation)) => {
                DocumentNavigationOutcome::Committed(Box::new(
                    self.commit
                        .commit_using(driver, self.token.clone(), &self.state, *navigation)
                        .await,
                ))
            }
            response => DocumentNavigationOutcome::Other(response),
        };
        CompletedDocumentNavigation {
            token: self.token,
            state: self.state,
            commit: self.commit,
            outcome,
            prefix: self.prefix,
        }
    }
}

fn start(
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    access: ProtocolStateAccess,
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    response: DocumentNavigationResponse,
    prefix: Vec<crate::conn::BackgroundProtocolEvent>,
    mut termination: NavigationTermination,
) -> FetchCommandTaskStep {
    let commit = termination.prepare_document_commit();
    FetchCommandTaskStep::Pending(
        PendingFetchCommandDispatch::new_for_owner(
            command_id,
            owner.clone(),
            PendingFetchCommandKind::DocumentNavigation,
            PendingFetchCommandOperation::DocumentNavigation(Box::new(PendingDocumentNavigation {
                access,
                token,
                state,
                response,
                commit,
                prefix,
            })),
        )
        .with_navigation_termination(Some(termination)),
    )
}

pub(super) fn can_start_request(
    conn: &CdpConnection,
    token: Option<&DocumentNavigationToken>,
    state: &NavigationDispatchState,
    intercept_response: bool,
) -> bool {
    conn.protocol_state_access().is_some()
        && token.is_some()
        && (matches!(state.requested_url.scheme(), "http" | "https")
            || (intercept_response && state.requested_url.scheme() != "data"))
}

pub(super) fn start_request(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    pending: crate::conn::PendingFetchNavigation,
    auth: Option<moli_core::page::SubresourceAuthCredentials>,
    prior_journal: Option<moli_fetch::NetworkObservationJournal>,
    mut termination: NavigationTermination,
) -> FetchCommandTaskStep {
    let access = conn
        .protocol_state_access()
        .expect("the scheduler pumps finite protocol state operations");
    let token = pending
        .document_navigation_token
        .as_ref()
        .expect("a prepared request has a navigation token")
        .clone();
    let state = pending.navigation.clone();
    let commit = termination.prepare_document_commit();
    let response = DocumentNavigationResponse::Request(Box::new(
        super::document_request::PreparedInterceptedNavigationRequest::new(
            conn,
            pending,
            auth,
            prior_journal,
            termination.response_transfer(),
        ),
    ));
    FetchCommandTaskStep::Pending(
        PendingFetchCommandDispatch::new_for_owner(
            command_id,
            owner.clone(),
            PendingFetchCommandKind::DocumentNavigation,
            PendingFetchCommandOperation::DocumentNavigation(Box::new(PendingDocumentNavigation {
                access,
                token,
                state,
                response,
                commit,
                prefix: Vec::new(),
            })),
        )
        .with_navigation_termination(Some(termination)),
    )
}

pub(super) fn start_fulfill(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    state: PendingFulfillRequestState,
) -> FetchCommandTaskStep {
    let (token, navigation) = match &state {
        PendingFulfillRequestState::Navigation { pending, .. } => (
            pending.document_navigation_token.as_ref(),
            &pending.navigation,
        ),
        PendingFulfillRequestState::ResponseTransfer { transfer, .. } => {
            (transfer.navigation_token(), transfer.navigation())
        }
        _ => unreachable!("only Document fulfill operations enter this workflow"),
    };
    let access = conn.protocol_state_access();
    let termination = access
        .as_ref()
        .and_then(|_| NavigationTermination::new(conn, owner, token, navigation));
    let (Some(access), Some(termination)) = (access, termination) else {
        return FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new_for_owner(
            command_id,
            owner.clone(),
            PendingFetchCommandKind::FulfillRequest {
                state: Box::new(state),
            },
            PendingFetchCommandOperation::Ready,
        ));
    };
    let token = token
        .expect("navigation termination retains its token")
        .clone();
    let navigation = navigation.clone();
    let response = match state {
        PendingFulfillRequestState::Navigation {
            pending,
            response_code,
            response_headers,
            decoded_body,
        } => conn.prepare_synthetic_navigation_response(
            Some(&token),
            &navigation,
            navigation.requested_url.clone(),
            response_code,
            response_headers,
            crate::conn::CapturedBody::from_optional_renderer_synthetic_response_body(decoded_body),
            pending.request_cookie_report,
            Default::default(),
            network::MainDocumentBodyProgressSource::default(),
        ),
        PendingFulfillRequestState::ResponseTransfer {
            transfer,
            response_code,
            response_headers,
            decoded_body,
        } => transfer.prepare_synthetic_response(
            conn,
            response_code,
            response_headers,
            crate::conn::CapturedBody::from_optional_renderer_synthetic_response_body(decoded_body),
        ),
        _ => unreachable!("Document state checked above"),
    };
    start(
        command_id,
        owner,
        access,
        token,
        navigation,
        DocumentNavigationResponse::Captured(Box::new(response)),
        Vec::new(),
        termination,
    )
}

pub(super) fn start_continue_response(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    request_id: String,
    transfer: crate::conn::PausedDocumentTransfer,
    response_code: Option<u16>,
    response_headers: Vec<(String, Vec<u8>)>,
) -> FetchCommandTaskStep {
    let access = conn.protocol_state_access();
    let termination = access.as_ref().and_then(|_| {
        NavigationTermination::new(
            conn,
            owner,
            transfer.navigation_token(),
            transfer.navigation(),
        )
    });
    let (Some(access), Some(mut termination)) = (access, termination) else {
        return FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new_for_owner(
            command_id,
            owner.clone(),
            PendingFetchCommandKind::ContinueResponse {
                state: Box::new(PendingContinueResponseState::ResponseTransfer {
                    request_id,
                    transfer: Box::new(transfer),
                    response_code,
                    response_headers,
                }),
            },
            PendingFetchCommandOperation::Ready,
        ));
    };
    let token = transfer.navigation_token().expect("captured token").clone();
    let navigation = transfer.navigation().clone();
    let response = match transfer.prepare_captured_response(conn, response_code, response_headers) {
        Ok(response) => response,
        Err(transfer) => {
            // The body is still leased to IO.read. Returning that transfer to
            // its original map also returns the source navigation obligation.
            conn.register_pending_fetch_response_transfer_for_owner(owner, request_id, transfer);
            termination.disarm();
            return FetchCommandTaskStep::Complete(
                crate::domains::command_output::CommandOutputPlan::error(
                    -32000,
                    "ResponseBodyStreamActive",
                ),
            );
        }
    };
    start(
        command_id,
        owner,
        access,
        token,
        navigation,
        DocumentNavigationResponse::Captured(Box::new(response)),
        Vec::new(),
        termination,
    )
}

pub(super) fn start_cancel_auth(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    pending: crate::conn::PendingFetchAuthNavigation,
) -> FetchCommandTaskStep {
    let access = conn.protocol_state_access();
    let termination = access.as_ref().and_then(|_| {
        NavigationTermination::new(
            conn,
            owner,
            pending.document_navigation_token.as_ref(),
            &pending.navigation,
        )
    });
    let (Some(access), Some(mut termination)) = (access, termination) else {
        return FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new_for_owner(
            command_id,
            owner.clone(),
            PendingFetchCommandKind::ContinueWithAuth {
                state: Box::new(
                    super::auth::PendingContinueWithAuthState::NavigationCancel {
                        pending: Box::new(pending),
                    },
                ),
            },
            PendingFetchCommandOperation::Ready,
        ));
    };
    let (mut pending, response) = super::auth::canceled_auth_response(pending);
    if response
        .observation_journal()
        .terminal_response_is_failed_proxy_connect()
    {
        let head = response.response();
        let progress = network::response_stage_main_document_navigation_network_progress(
            conn,
            &pending.navigation,
            pending.request_cookie_report.as_ref(),
        );
        let mut events = Vec::new();
        progress.emit_response_without_extra_info_into_background_events(
            &mut events,
            &head.final_url,
            head.status,
            &head.headers,
            false,
        );
        return start(
            command_id,
            owner,
            access,
            pending.document_navigation_token.expect("proxy auth token"),
            pending.navigation,
            DocumentNavigationResponse::Result(Ok(NavigationLoadOutcome::network_failure(
                "net::ERR_HTTP_RESPONSE_CODE_FAILURE".to_owned(),
            ))),
            events,
            termination,
        );
    }
    if super::navigation::prepare_navigation_response_stage(
        conn,
        &mut pending,
        &response.response().final_url,
    ) {
        let command_id = pending.navigation.navigate_id;
        let session_id = pending.navigation.owner.session_id().map(str::to_owned);
        let mut output = CommandOutputBuffer::default();
        super::navigation::pause_buffered_raw_response_stage_navigation_into_buffer(
            conn,
            &mut output,
            pending,
            response,
        );
        let mut out = FetchCommandOutput::default();
        out.push_success();
        out.extend_plan_as_background_events(output.into_plan(), command_id, session_id.as_deref());
        termination.disarm();
        return FetchCommandTaskStep::Complete(out.into_output_plan());
    }
    let token = pending
        .document_navigation_token
        .expect("captured auth token");
    // CancelAuth continues the original 401/407 response; it does not cancel
    // the navigation or start another HTTP request.
    let (response, journal) = response.into_parts_with_observation_journal();
    let (head, body) = response.into_body();
    let body = body
        .try_into_materialized_bytes()
        .expect("an auth challenge owns its captured body");
    let response = conn.prepare_captured_navigation_response(
        Some(&token),
        &pending.navigation,
        head,
        crate::conn::CapturedBody::from_bytes(body),
        journal,
        network::MainDocumentBodyProgressSource::default(),
    );
    start(
        command_id,
        owner,
        access,
        token,
        pending.navigation,
        DocumentNavigationResponse::Captured(Box::new(response)),
        Vec::new(),
        termination,
    )
}

pub(super) async fn complete(
    conn: &mut CdpConnection,
    completed: CompletedDocumentNavigation,
    out: &mut FetchCommandOutput,
    command_context: &mut CommandDispatchContext,
) {
    // Captured/synthetic continuation keeps its original publication boundary:
    // success enters this plan before navigation output, and the actor flushes
    // the plan only after completion. Streaming continuation uses another path.
    out.push_success();
    out.extend_background_events(completed.prefix);
    match completed.outcome {
        DocumentNavigationOutcome::Committed(navigation) => {
            let command_id = completed.state.navigate_id;
            let session_id = completed.state.owner.session_id().map(str::to_owned);
            let completion =
                completed
                    .commit
                    .complete(completed.token, completed.state, *navigation);
            let mut output = CommandOutputBuffer::default();
            let mut context = CommandDispatchContext::new(command_context.response_flush().clone());
            page::complete_prepared_navigation_into_buffer_async(
                conn,
                &mut output,
                completion,
                &mut context,
            )
            .await;
            if let Some(predecessor) = context.take_renderer_output_predecessor() {
                output.set_renderer_output_predecessor(predecessor);
            }
            out.extend_plan_as_background_events(
                output.into_plan(),
                command_id,
                session_id.as_deref(),
            );
        }
        DocumentNavigationOutcome::Other(navigation) => {
            let navigation =
                network::materialize_navigation_load_result(conn, &completed.state, navigation);
            super::navigation::complete_tokened_materialized_navigation_as_background_events_async(
                conn,
                out,
                Some(completed.token),
                completed.state,
                navigation,
            )
            .await;
        }
        DocumentNavigationOutcome::Transferred(plan) => {
            out.extend_plan_as_background_events(
                plan,
                completed.state.navigate_id,
                completed.state.owner.session_id(),
            );
        }
        DocumentNavigationOutcome::Superseded => {
            let mut output = CommandOutputBuffer::default();
            page::complete_superseded_intercepted_navigation_into_buffer_async(
                conn,
                &mut output,
                &completed.token,
                &completed.state,
            )
            .await;
            out.extend_plan_as_background_events(
                output.into_plan(),
                completed.state.navigate_id,
                completed.state.owner.session_id(),
            );
        }
    }
}
