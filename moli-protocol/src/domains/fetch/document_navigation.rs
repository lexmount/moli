use anyhow::Result;

use crate::conn::{
    CdpConnection, CommandDispatchContext, CommandOwnerScope, CommittedNavigationPage,
    DocumentNavigationToken, LoadedNavigation, NavigationDispatchState, NavigationLoadOutcome,
    PreparedDocumentCommitState, ProtocolStateAccess, ProtocolWorkflowDriver,
};
use crate::domains::{command_output::CommandOutputBuffer, network, page};
use moli_core::page::RendererPageReplacementError;

use super::{
    FetchCommandOutput, FetchCommandTaskStep, PendingFetchCommandDispatch, PendingFetchCommandKind,
    PendingFetchCommandOperation, PreparedFetchNavigationResponse,
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

pub(super) fn complete_owned(
    access: ProtocolStateAccess,
    completed: CompletedDocumentNavigation,
    termination: Option<NavigationTermination>,
) -> crate::conn::PreparedCommandWorkflow {
    // Source cancellation follows resources already admitted by this tail;
    // its inline completion must not get ahead of worker cleanup needing actor progress.
    let termination = std::rc::Rc::new(std::cell::RefCell::new(termination));
    let failure_termination = termination.clone();
    let cleanup = page::NavigationProtocolCleanup::default();
    let failure_cleanup = cleanup.clone();
    let failure_access = access.clone();
    crate::conn::PreparedCommandWorkflow::new(move |context| async move {
        finish(
            &mut ProtocolWorkflowDriver::owned(access),
            context,
            completed,
            termination,
            cleanup,
        )
        .await
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

async fn finish(
    driver: &mut ProtocolWorkflowDriver<'_>,
    mut context: crate::conn::CommandWorkflowContext,
    completed: CompletedDocumentNavigation,
    termination: std::rc::Rc<std::cell::RefCell<Option<NavigationTermination>>>,
    cleanup: page::NavigationProtocolCleanup,
) -> crate::conn::CompletedCommandWorkflow {
    use crate::domains::command_output::CommandOutputPlan;
    let mut prefix = CommandOutputPlan::default();
    prefix.extend_background_events(completed.prefix);
    context.retain_output(prefix);
    let navigation = match completed.outcome {
        DocumentNavigationOutcome::Committed(navigation) => navigation,
        outcome => {
            let result: Result<()> = async {
                match outcome {
                    DocumentNavigationOutcome::Other(response) => {
                        let source_termination = termination.clone();
                        let prepared = context
                            .with_state(driver, move |state, context| {
                                let prepared = state
                                    .prepare_renderer_navigation_result_with_output(
                                        completed.token,
                                        completed.state,
                                        response,
                                        context,
                                        page::NavigationCommandOutput::Background,
                                        Vec::new(),
                                    );
                                if let Some(mut termination) =
                                    source_termination.borrow_mut().take()
                                {
                                    termination.disarm();
                                }
                                prepared
                            })
                            .await?;
                        prepared.run(driver, &mut context).await?;
                    }
                    DocumentNavigationOutcome::Transferred(plan) => {
                        context.retain_output(plan.into_background_event_plan(
                            completed.state.navigate_id,
                            completed.state.owner.session_id(),
                        ));
                    }
                    DocumentNavigationOutcome::Superseded => {
                        let source_termination = termination.clone();
                        let finish = context
                            .with_state(driver, move |state, context| {
                                let finish = state.prepare_superseded_fetch_navigation(
                                    completed.token,
                                    completed.state,
                                    context,
                                );
                                if let Some(mut termination) =
                                    source_termination.borrow_mut().take()
                                {
                                    termination.disarm();
                                }
                                finish
                            })
                            .await?;
                        let mut output = CommandOutputPlan::default();
                        output.extend_background_events(finish.wait_using(driver).await?);
                        context.retain_output(output);
                    }
                    DocumentNavigationOutcome::Committed(_) => unreachable!(),
                }
                Ok(())
            }
            .await;
            return match result {
                Ok(()) => {
                    if let Some(mut termination) = termination.borrow_mut().take() {
                        termination.disarm();
                    }
                    context.complete(CommandOutputPlan::success())
                }
                Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
            };
        }
    };
    let completion = completed
        .commit
        .complete(completed.token, completed.state, *navigation);
    let admitted_termination = termination.clone();
    let prepared = context
        .with_state(driver, move |state, context| {
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
                .run(driver, context, cleanup, move || {
                    if let Some(mut termination) = termination.borrow_mut().take() {
                        termination.disarm();
                    }
                })
                .await
        }
        Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
    }
}

enum DocumentNavigationOutcome {
    Committed(Box<Result<LoadedNavigation<CommittedNavigationPage>, RendererPageReplacementError>>),
    Other(Result<NavigationLoadOutcome>),
    Transferred(crate::domains::command_output::CommandOutputPlan),
    Superseded,
}

enum DocumentNavigationResponse {
    Prepared(PreparedFetchNavigationResponse),
    Request(Box<super::document_request::PreparedInterceptedNavigationRequest>),
    Load(Option<Box<crate::conn::BackgroundNavigationLoadJob>>),
    Failure(anyhow::Error),
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
            DocumentNavigationResponse::Failure(error) => {
                return CompletedDocumentNavigation {
                    token: self.token,
                    state: self.state,
                    commit: self.commit,
                    outcome: DocumentNavigationOutcome::Other(Err(error)),
                    prefix: self.prefix,
                };
            }
            DocumentNavigationResponse::Prepared(response) => response,
            DocumentNavigationResponse::Request(request) => request.prepare(driver).await,
            DocumentNavigationResponse::Load(job) => {
                let response = match job {
                    Some(job) => job.run(None).await.0,
                    None => Err(moli_fetch::FetchCancelled.into()),
                };
                PreparedFetchNavigationResponse::Result(response)
            }
        };
        let response = match response {
            PreparedFetchNavigationResponse::Captured(response) => match *response {
                Ok(response) => response.run().await,
                Err(error) => Err(error),
            },
            PreparedFetchNavigationResponse::Streaming(response) => match *response {
                Ok(response) => response.run(None).await,
                Err(error) => Err(error),
            },
            PreparedFetchNavigationResponse::Result(response) => response,
            PreparedFetchNavigationResponse::Transferred(plan) => {
                return CompletedDocumentNavigation {
                    token: self.token,
                    state: self.state,
                    commit: self.commit,
                    outcome: DocumentNavigationOutcome::Transferred(plan),
                    prefix: self.prefix,
                };
            }
            PreparedFetchNavigationResponse::Superseded => {
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
    mut termination: Option<NavigationTermination>,
) -> FetchCommandTaskStep {
    let commit = termination
        .as_mut()
        .map(NavigationTermination::prepare_document_commit)
        .unwrap_or_else(PreparedDocumentCommitState::new);
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
        .with_navigation_termination(termination),
    )
}

pub(super) fn start_failure(
    conn: &CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    error: anyhow::Error,
) -> FetchCommandTaskStep {
    let termination = NavigationTermination::new(conn, owner, Some(&token), &state);
    start(
        command_id,
        owner,
        conn.protocol_state_access().expect("owned navigation"),
        token,
        state,
        DocumentNavigationResponse::Failure(error),
        Vec::new(),
        termination,
    )
}

pub(super) fn can_start_request(
    conn: &CdpConnection,
    token: Option<&DocumentNavigationToken>,
) -> bool {
    conn.protocol_state_access().is_some() && token.is_some()
}

pub(super) fn start_request(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    mut pending: crate::conn::PendingFetchNavigation,
    auth: Option<moli_core::page::SubresourceAuthCredentials>,
    prior_journal: Option<moli_fetch::NetworkObservationJournal>,
    mut termination: Option<NavigationTermination>,
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
    let commit = termination
        .as_mut()
        .map(NavigationTermination::prepare_document_commit)
        .unwrap_or_else(PreparedDocumentCommitState::new);
    let scheme = pending.navigation.requested_url.scheme();
    let handle_auth = scheme != "data"
        && conn.target_fetch_matches_auth_required_for_owner(
            &pending.navigation.owner,
            &pending.navigation.requested_url,
        );
    let response = if handle_auth
        || matches!(scheme, "http" | "https")
        || (pending.intercept_response && scheme != "data")
    {
        DocumentNavigationResponse::Request(Box::new(
            super::document_request::PreparedInterceptedNavigationRequest::new(
                conn,
                pending,
                auth,
                prior_journal,
                termination
                    .as_ref()
                    .map(NavigationTermination::response_transfer)
                    .unwrap_or_default(),
            ),
        ))
    } else if auth.is_some() && !pending.auth_retry_budget.try_restart() {
        DocumentNavigationResponse::Prepared(PreparedFetchNavigationResponse::Result(Ok(
            NavigationLoadOutcome::network_failure(
                moli_fetch::AuthRetryBudget::ERROR_TEXT.to_owned(),
            ),
        )))
    } else {
        network::record_main_document_request_body(conn, &pending.navigation);
        if pending.intercept_response {
            let mut output = CommandOutputBuffer::default();
            super::navigation::pause_data_url_response_stage_navigation_into_buffer(
                conn,
                &mut output,
                pending,
            );
            let mut out = FetchCommandOutput::default();
            out.push_success();
            out.extend_plan_as_background_events(
                output.into_plan(),
                state.navigate_id,
                state.owner.session_id(),
            );
            if let Some(termination) = termination.as_mut() {
                termination.disarm();
            }
            return FetchCommandTaskStep::Complete(out.into_output_plan());
        } else {
            DocumentNavigationResponse::Load(
                conn.navigation_load_job_for_navigation(
                    &token,
                    &state,
                    network::MainDocumentBodyProgressSource::default(),
                    None,
                )
                .map(Box::new),
            )
        }
    };
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
        .with_navigation_termination(termination),
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
    let (Some(access), Some(token)) = (access, token.cloned()) else {
        return FetchCommandTaskStep::Pending(PendingFetchCommandDispatch::new_for_owner(
            command_id,
            owner.clone(),
            PendingFetchCommandKind::FulfillRequest {
                state: Box::new(state),
            },
            PendingFetchCommandOperation::Ready,
        ));
    };
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
        DocumentNavigationResponse::Prepared(PreparedFetchNavigationResponse::Captured(Box::new(
            response,
        ))),
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
    let mut termination = access.as_ref().and_then(|_| {
        NavigationTermination::new(
            conn,
            owner,
            transfer.navigation_token(),
            transfer.navigation(),
        )
    });
    let (Some(access), Some(token)) = (access, transfer.navigation_token().cloned()) else {
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
    let navigation = transfer.navigation().clone();
    let response = match transfer.prepare_response(conn, response_code, response_headers) {
        Ok(response) => response,
        Err(transfer) => {
            // The body is still leased to IO.read. Returning that transfer to
            // its original map also returns the source navigation obligation.
            conn.register_pending_fetch_response_transfer_for_owner(owner, request_id, transfer);
            if let Some(termination) = termination.as_mut() {
                termination.disarm();
            }
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
        DocumentNavigationResponse::Prepared(response),
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
    let mut termination = access.as_ref().and_then(|_| {
        NavigationTermination::new(
            conn,
            owner,
            pending.document_navigation_token.as_ref(),
            &pending.navigation,
        )
    });
    let (Some(access), Some(_)) = (access, pending.document_navigation_token.as_ref()) else {
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
            DocumentNavigationResponse::Prepared(PreparedFetchNavigationResponse::Result(Ok(
                NavigationLoadOutcome::network_failure(
                    "net::ERR_HTTP_RESPONSE_CODE_FAILURE".to_owned(),
                ),
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
        if let Some(termination) = termination.as_mut() {
            termination.disarm();
        }
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
        DocumentNavigationResponse::Prepared(PreparedFetchNavigationResponse::Captured(Box::new(
            response,
        ))),
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
    let (context, _) = crate::conn::CommandWorkflowContext::take_from(command_context, None, None);
    let completed = Box::pin(finish(
        &mut ProtocolWorkflowDriver::inline(conn),
        context,
        completed,
        Default::default(),
        page::NavigationProtocolCleanup::default(),
    ))
    .await;
    out.extend_plan_as_command_response(completed.finish(conn, command_context));
}
