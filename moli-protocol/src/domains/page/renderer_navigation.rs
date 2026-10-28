use anyhow::{Context as _, Result};
use moli_core::page::PendingPageCommand;

use crate::conn::{
    BackgroundNavigationLoadJob, CdpConnection, CommandOwnerScope, CommandWorkflowContext,
    DocumentNavigationToken, NavigationDispatchState, NavigationLoadOutcome,
    PreparedDocumentCommitState, ProtocolWorkflowDriver,
    TargetRootDocumentProtocolAttachmentIdentity,
};
use crate::domains::{
    command_output::{CommandOutputBuffer, CommandOutputPlan},
    fetch::{
        NavigationTermination, PreparedFetchNavigationResponse,
        PreparedInterceptedNavigationRequest,
    },
    network,
};

use super::{PageCommandTaskStep, PendingPageCommandKind, PreparedNavigationFailureCleanup};

#[derive(Debug, thiserror::Error)]
pub(crate) enum RendererNavigationError {
    #[error("same-document navigation returned an unexpected renderer reply")]
    UnexpectedSameDocumentReply,
    #[error("Navigation is not same-document")]
    NotSameDocument,
}

/// The complete renderer navigation leaves its producing publication before
/// waiting. A new Document's Fetch pauses must be able to cross that stream.
pub(crate) struct PreparedRendererNavigation {
    work: RendererNavigationWork,
    source: Option<RendererNavigationSource>,
    owner: CommandOwnerScope,
    prefix: CommandOutputPlan,
}

enum RendererNavigationWork {
    Complete(CommandOutputPlan),
    Load(Box<BackgroundNavigationLoadJob>),
    Request(Box<PreparedInterceptedNavigationRequest>),
    SameDocument {
        pending: PendingPageCommand,
        binding: Option<TargetRootDocumentProtocolAttachmentIdentity>,
    },
}

struct RendererNavigationSource {
    token: DocumentNavigationToken,
    state: NavigationDispatchState,
    commit: PreparedDocumentCommitState,
    termination: Option<NavigationTermination>,
}

impl RendererNavigationSource {
    fn new(
        conn: &CdpConnection,
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
    ) -> Self {
        let mut termination = NavigationTermination::new(conn, &state.owner, Some(&token), &state);
        let commit = termination
            .as_mut()
            .map(NavigationTermination::prepare_document_commit)
            .unwrap_or_else(PreparedDocumentCommitState::new);
        Self {
            token,
            state,
            commit,
            termination,
        }
    }

    fn disarm(&mut self) {
        if let Some(termination) = self.termination.as_mut() {
            termination.disarm();
        }
    }

    async fn finish_response(
        self,
        response: Result<NavigationLoadOutcome>,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        self.finish_response_with_output(
            response,
            driver,
            context,
            super::NavigationCommandOutput::Background,
            Vec::new(),
        )
        .await
    }

    async fn finish_response_with_output(
        mut self,
        response: Result<NavigationLoadOutcome>,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
        output_mode: super::NavigationCommandOutput,
        prefix: Vec<crate::conn::BackgroundProtocolEvent>,
    ) -> Result<()> {
        let response = match response {
            Err(error) => {
                let token = self.token.clone();
                let state = self.state.clone();
                match driver.with_state(move |access| {
                    access.prepare_fetch_navigation_error(token, state, error)
                })? {
                    Ok(prepared) => prepared.run().await,
                    Err(error) => Err(error),
                }
            }
            response => response,
        };
        match response {
            Ok(NavigationLoadOutcome::ResponseCommitReady(navigation)) => {
                let committed = self
                    .commit
                    .commit_using(driver, self.token.clone(), &self.state, *navigation)
                    .await;
                let completion =
                    self.commit
                        .complete(self.token.clone(), self.state.clone(), committed);
                let prepared = context.with_state(driver, move |state, context| {
                    state.prepare_navigation_protocol_completion_with_output(
                        completion,
                        context,
                        output_mode,
                        prefix,
                    )
                })?;
                prepared.run_into_context(driver, context).await?;
            }
            response => {
                let token = self.token.clone();
                let navigation = self.state.clone();
                let prepared = context.with_state(driver, move |state, context| {
                    state.prepare_renderer_navigation_result_with_output(
                        token,
                        navigation,
                        response,
                        context,
                        output_mode,
                        prefix,
                    )
                })?;
                prepared.run(driver, context).await?;
            }
        }
        self.disarm();
        Ok(())
    }
}

impl PreparedRendererNavigation {
    pub(crate) fn prepare_owner_action(
        conn: &mut CdpConnection,
        action: crate::conn::TopLevelNavigationOwnerAction,
    ) -> Option<Self> {
        let (owner, page, request) = action.into_parts();
        if !conn.target_page_residence_identity_is_current(&page) {
            return None;
        }
        let step = match request {
            crate::conn::TopLevelNavigationRequest::History(traversal) => {
                super::navigation::start_session_owner_history_traversal_from_renderer(
                    conn,
                    &owner,
                    traversal.delta,
                )
            }
            crate::conn::TopLevelNavigationRequest::Location(navigation) => {
                let start = super::navigation::start_session_owner_navigation_from_renderer(
                    conn,
                    &owner,
                    navigation.url(),
                    navigation.request_method(),
                    navigation.request_body(),
                    navigation.request_headers(),
                    navigation.browser_navigation_kind(),
                    navigation.web_mcp_invocation(),
                    Some(match navigation.browser_navigation_kind() {
                        moli_fetch::BrowserNavigationRequestKind::Reload => {
                            moli_core::page::RendererAuxiliaryNavigationKind::Reload
                        }
                        moli_fetch::BrowserNavigationRequestKind::Navigate => {
                            match navigation.history_mutation() {
                                moli_page_types::NavigationHistoryMutation::Push => {
                                    moli_core::page::RendererAuxiliaryNavigationKind::Assign
                                }
                                moli_page_types::NavigationHistoryMutation::Replace => {
                                    moli_core::page::RendererAuxiliaryNavigationKind::Replace
                                }
                            }
                        }
                    }),
                    None,
                    navigation.initial_document_environment().cloned(),
                    navigation.navigation_initiator().cloned(),
                );
                super::navigation::finish_started_navigation_command_for_parts(
                    conn,
                    None,
                    owner.clone(),
                    start,
                    &[],
                )
            }
        };
        Some(Self::prepare(conn, owner, step))
    }

    pub(super) fn prepare(
        conn: &mut CdpConnection,
        owner: CommandOwnerScope,
        step: PageCommandTaskStep,
    ) -> Self {
        let mut source = None;
        let mut prefix = CommandOutputPlan::default();
        let work = match step {
            PageCommandTaskStep::Complete(plan) => RendererNavigationWork::Complete(plan),
            PageCommandTaskStep::Pending(pending) => match *pending.kind {
                PendingPageCommandKind::Navigate(pending) => {
                    source = Some(RendererNavigationSource::new(
                        conn,
                        pending.token,
                        pending.state,
                    ));
                    prefix.extend_background_events(pending.prefix_events);
                    RendererNavigationWork::Load(Box::new(pending.job))
                }
                PendingPageCommandKind::ContinueNavigationWithoutRequestPause(pending) => {
                    prefix.extend_background_events(pending.prefix_events);
                    let navigation = RendererNavigationSource::new(
                        conn,
                        pending
                            .pending
                            .document_navigation_token
                            .as_ref()
                            .expect("renderer request owns its navigation token")
                            .clone(),
                        pending.pending.navigation.clone(),
                    );
                    let transfer = navigation
                        .termination
                        .as_ref()
                        .map(NavigationTermination::response_transfer)
                        .unwrap_or_default();
                    let request = PreparedInterceptedNavigationRequest::new(
                        conn,
                        pending.pending,
                        None,
                        None,
                        transfer,
                    );
                    source = Some(navigation);
                    RendererNavigationWork::Request(Box::new(request))
                }
                PendingPageCommandKind::SameDocumentNavigate(pending) => {
                    let binding = conn
                        .target_root_document_lifecycle_identity_for_owner(&owner)
                        .and_then(|document| {
                            conn.target_root_document_protocol_attachment_identity_for_owner(
                                &owner, document,
                            )
                        });
                    RendererNavigationWork::SameDocument {
                        pending: pending.pending,
                        binding,
                    }
                }
                _ => unreachable!(
                    "renderer top-level navigation starts only a load, request, or same-document operation"
                ),
            },
        };
        Self {
            work,
            source,
            owner,
            prefix,
        }
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        retain_background(context, self.prefix);
        match self.work {
            RendererNavigationWork::Complete(plan) => retain_background(context, plan),
            RendererNavigationWork::Load(job) => {
                let (response, _) = job.run(None).await;
                self.source
                    .expect("load retains its original navigation")
                    .finish_response(response, driver, context)
                    .await?;
            }
            RendererNavigationWork::Request(request) => {
                let mut source = self
                    .source
                    .expect("request retains its original navigation");
                let response = match request.prepare(driver).await {
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
                        retain_background(context, plan);
                        source.disarm();
                        return Ok(());
                    }
                    PreparedFetchNavigationResponse::Superseded => {
                        Err(moli_fetch::FetchCancelled.into())
                    }
                };
                source.finish_response(response, driver, context).await?;
            }
            RendererNavigationWork::SameDocument { pending, binding } => {
                let completed = pending.wait().await?;
                let navigated = completed
                    .bool_reply_value()
                    .context(RendererNavigationError::UnexpectedSameDocumentReply)?;
                context.with_state(driver, move |state, context| {
                    state.finish_renderer_same_document_navigation(
                        self.owner, binding, completed, context,
                    )
                })??;
                anyhow::ensure!(navigated, RendererNavigationError::NotSameDocument);
            }
        }
        Ok(())
    }
}

pub(crate) enum PreparedRendererNavigationResult {
    Failure(PreparedNavigationFailureCleanup),
    Download {
        download: crate::domains::activity::PreparedMainDocumentDownload,
        token: DocumentNavigationToken,
        state: Box<NavigationDispatchState>,
    },
}

impl PreparedRendererNavigationResult {
    pub(crate) fn prepare_with_output(
        conn: &mut CdpConnection,
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        response: Result<NavigationLoadOutcome>,
        context: &mut CommandWorkflowContext,
        output_mode: super::NavigationCommandOutput,
        prefix: Vec<crate::conn::BackgroundProtocolEvent>,
    ) -> Self {
        if !conn.accepts_pending_document_navigation_for_owner(&state.owner, &token) {
            return Self::superseded(conn, token, state, context, output_mode, prefix);
        }
        let navigation = network::materialize_navigation_load_result(conn, &state, response);
        Self::prepare_materialized(conn, token, state, navigation, context, output_mode, prefix)
    }

    fn superseded(
        conn: &mut CdpConnection,
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        context: &mut CommandWorkflowContext,
        output_mode: super::NavigationCommandOutput,
        prefix: Vec<crate::conn::BackgroundProtocolEvent>,
    ) -> Self {
        let mut output = CommandOutputBuffer::default();
        output.extend_background_events_after_messages(prefix);
        let output_state = state.clone();
        let pending = PreparedNavigationFailureCleanup::superseded(conn, &mut output, token, state);
        output_mode.retain(context, &output_state, output.into_plan());
        Self::Failure(pending)
    }

    pub(crate) fn prepare_materialized(
        conn: &mut CdpConnection,
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        navigation: network::MaterializedNavigationLoadOutcome,
        context: &mut CommandWorkflowContext,
        output_mode: super::NavigationCommandOutput,
        prefix: Vec<crate::conn::BackgroundProtocolEvent>,
    ) -> Self {
        if !conn.accepts_pending_document_navigation_for_owner(&state.owner, &token) {
            return Self::superseded(conn, token, state, context, output_mode, prefix);
        }
        match navigation {
            network::MaterializedNavigationLoadOutcome::Failed(failure) => {
                let mut output = CommandOutputBuffer::default();
                output.extend_background_events_after_messages(prefix);
                let output_state = state.clone();
                let pending = super::prepare_failed_navigation_cleanup(
                    conn,
                    &mut output,
                    token,
                    state,
                    failure,
                );
                output_mode.retain(context, &output_state, output.into_plan());
                Self::Failure(pending)
            }
            network::MaterializedNavigationLoadOutcome::Download(download) => {
                if let Some(invocation) = state.web_mcp_invocation.as_ref() {
                    context.context_mut().protocol_events_mut().extend(
                        crate::domains::web_mcp::navigation_failed(
                            conn,
                            &state.owner,
                            invocation.invocation_id,
                        ),
                    );
                }
                let _ = conn.clear_pending_navigation_history_update_for_owner(&state.owner);
                let download =
                    crate::domains::activity::PreparedMainDocumentDownload::prepare_with_output(
                        conn,
                        state.clone(),
                        download,
                        context,
                        output_mode,
                        prefix,
                    );
                Self::Download {
                    download,
                    token,
                    state: Box::new(state),
                }
            }
            network::MaterializedNavigationLoadOutcome::ResponseCommitReady(_) => {
                unreachable!("prepared responses use their owned native commit")
            }
        }
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<()> {
        match self {
            Self::Failure(pending) => context
                .context_mut()
                .protocol_events_mut()
                .extend(pending.wait_using(driver).await?),
            Self::Download {
                download,
                token,
                state,
            } => {
                download.run(context).await;
                let owner = state.owner.clone();
                let finish_token = token.clone();
                if let Some(finish) = driver.with_state(move |access| {
                    access.prepare_navigation_protocol_finish(owner, finish_token, None)
                })? {
                    context
                        .context_mut()
                        .protocol_events_mut()
                        .extend(finish.wait_using(driver).await?);
                }
                driver.with_state(move |access| {
                    access.finish_renderer_navigation_result(state.owner, state.loader_id)
                })?;
            }
        }
        Ok(())
    }
}

fn retain_background(context: &mut CommandWorkflowContext, plan: CommandOutputPlan) {
    let (_, events) = plan.into_command_status_and_background_events();
    context.context_mut().protocol_events_mut().extend(events);
}

pub(crate) struct PreparedPageNavigationCompletion {
    source: RendererNavigationSource,
    response: PageNavigationResponse,
    prefix: Vec<crate::conn::BackgroundProtocolEvent>,
}

enum PageNavigationResponse {
    Result(Result<NavigationLoadOutcome>),
    Request(Box<PreparedInterceptedNavigationRequest>),
}

impl PreparedPageNavigationCompletion {
    pub(super) fn from_result(
        conn: &CdpConnection,
        token: DocumentNavigationToken,
        state: NavigationDispatchState,
        response: Result<NavigationLoadOutcome>,
        prefix: Vec<crate::conn::BackgroundProtocolEvent>,
    ) -> Self {
        Self {
            source: RendererNavigationSource::new(conn, token, state),
            response: PageNavigationResponse::Result(response),
            prefix,
        }
    }

    pub(super) fn from_request(
        conn: &mut CdpConnection,
        pending: crate::conn::PendingFetchNavigation,
        prefix: Vec<crate::conn::BackgroundProtocolEvent>,
    ) -> Self {
        let source = RendererNavigationSource::new(
            conn,
            pending
                .document_navigation_token
                .as_ref()
                .expect("pending navigation owns its token")
                .clone(),
            pending.navigation.clone(),
        );
        let transfer = source
            .termination
            .as_ref()
            .map(NavigationTermination::response_transfer)
            .unwrap_or_default();
        let request =
            PreparedInterceptedNavigationRequest::new(conn, pending, None, None, transfer);
        Self {
            source,
            response: PageNavigationResponse::Request(Box::new(request)),
            prefix,
        }
    }

    pub(crate) async fn run(
        mut self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
        output_mode: super::NavigationCommandOutput,
    ) -> Result<()> {
        let response = match self.response {
            PageNavigationResponse::Result(response) => response,
            PageNavigationResponse::Request(request) => match request.prepare(driver).await {
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
                    if let Some(result) = self.source.state.result_projection.automation_result() {
                        result.transfer_source_response();
                    }
                    let mut output = CommandOutputPlan::default();
                    output.extend_background_events(self.prefix);
                    output.extend(plan);
                    context.retain_output(output);
                    self.source.disarm();
                    return Ok(());
                }
                PreparedFetchNavigationResponse::Superseded => {
                    Err(moli_fetch::FetchCancelled.into())
                }
            },
        };
        self.source
            .finish_response_with_output(response, driver, context, output_mode, self.prefix)
            .await
    }
}
