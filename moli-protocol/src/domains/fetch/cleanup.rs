use anyhow::Result;

use super::{DrainedFetchRequests, navigation::NavigationTermination};
use crate::conn::{
    CdpConnection, CommandOwnerScope, PendingSubresourceFetchAuthRequest,
    PendingSubresourceFetchRequest, PendingSubresourceFetchResponseRequest, ProtocolStateHandle,
    ProtocolWorkflowDriver, TargetRootDocumentProtocolAttachmentIdentity,
};
use crate::domains::{
    activity,
    command_output::{CommandOutputBuffer, CommandOutputPlan},
    network, page,
};
use moli_core::page::PendingPageCommand;

#[derive(Clone, Copy, Debug, thiserror::Error)]
pub(super) enum FetchCleanupReason {
    #[error("Fetch interception disabled")]
    Disabled,
    #[error("Target detached")]
    Detached,
    #[error("Target closed")]
    TargetClosed,
    #[error("Page closed")]
    PageClosed,
    #[error("Browser context disposed")]
    BrowserContextDisposed,
    #[error("Navigation stopped")]
    NavigationStopped,
    #[error("Page crashed")]
    PageCrashed,
}

pub(crate) struct PreparedFetchCleanup {
    access: ProtocolStateHandle,
    policy: Option<Result<PendingPageCommand>>,
    navigations: Vec<NavigationCleanup>,
    resumptions: Vec<(
        crate::conn::PendingFetchNavigation,
        Option<NavigationTermination>,
    )>,
    subresources: Vec<PreparedSubresourceCleanup>,
}

struct NavigationCleanup {
    plan: CommandOutputPlan,
    pending: Option<page::PreparedNavigationFailureCleanup>,
    termination: Option<NavigationTermination>,
}

pub(super) type CompletedFetchCleanup = FetchCleanupOutput;

#[derive(Default)]
pub(in crate::domains) struct FetchCleanupOutput {
    plan: CommandOutputPlan,
    error: Option<anyhow::Error>,
    terminations: Vec<NavigationTermination>,
}

impl FetchCleanupOutput {
    pub(super) fn into_command_plan(mut self) -> CommandOutputPlan {
        let mut plan = match self.error.take() {
            Some(error) => CommandOutputPlan::error(
                -32000,
                format!("failed to clear page fetch interception: {error}"),
            ),
            None => CommandOutputPlan::success(),
        };
        for termination in &mut self.terminations {
            termination.disarm();
        }
        plan.extend(self.plan);
        plan
    }

    pub(in crate::domains) fn into_disposal_output(
        mut self,
        events: &mut Vec<crate::conn::BackgroundProtocolEvent>,
    ) -> crate::conn::SessionPageUpdateResult {
        for termination in &mut self.terminations {
            termination.disarm();
        }
        let predecessor = self.plan.take_renderer_output_predecessor();
        events.extend(self.plan.into_background_events(None, None));
        crate::conn::SessionPageUpdateResult {
            first_error: self.error.map(|error| {
                error.context("renderer failed Fetch interception disable while disposing session")
            }),
            predecessor,
        }
    }
}

impl PreparedFetchCleanup {
    pub(super) fn new(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        pending: DrainedFetchRequests,
        policy: Option<Result<PendingPageCommand>>,
        reason: FetchCleanupReason,
        preserve_document: bool,
    ) -> Self {
        let DrainedFetchRequests {
            navigations: requests,
            auth_navigations: auths,
            response_transfers: responses,
            subresource_requests: subrequests,
            subresource_auth_requests: subauths,
            subresource_responses: subresponses,
        } = pending;
        let subresource_error = match reason {
            FetchCleanupReason::NavigationStopped => {
                moli_fetch::NET_ERR_ABORTED_ERROR_TEXT.to_owned()
            }
            _ => reason.to_string(),
        };
        let resume_navigations = matches!(
            reason,
            FetchCleanupReason::Disabled | FetchCleanupReason::Detached
        );
        // Removing an interceptor leaves a valid navigation running.
        // Actual Page/navigation retirement still owns transport cancellation.
        let subresources = subrequests
            .into_iter()
            .map(|(request_id, pending)| {
                if resume_navigations
                    && pending.owner_kind.drains_on_fetch_disable()
                    && (pending.resource_type == moli_core::page::SubresourceResourceType::Document
                        || pending
                            .document_command_target()
                            .is_some_and(|target| target.has_native_network_pause()))
                {
                    CleanupSubresource::ContinueRequest {
                        request_id,
                        pending,
                    }
                } else {
                    CleanupSubresource::Request(pending)
                }
            })
            .chain(subauths.into_iter().map(|(request_id, pending)| {
                if resume_navigations
                    && pending.owner_kind.drains_on_fetch_disable()
                    && (pending.resource_type == moli_core::page::SubresourceResourceType::Document
                        || pending
                            .command_target
                            .as_ref()
                            .is_some_and(|target| target.has_native_network_pause()))
                {
                    CleanupSubresource::ContinueAuth {
                        request_id,
                        pending,
                    }
                } else {
                    CleanupSubresource::Auth(pending)
                }
            }))
            .chain(subresponses.into_iter().map(|(_, pending)| {
                if resume_navigations
                    && pending.owner_kind.drains_on_fetch_disable()
                    && (pending.resource_type == moli_core::page::SubresourceResourceType::Document
                        || pending
                            .command_target
                            .as_ref()
                            .is_some_and(|target| target.has_native_network_pause()))
                {
                    CleanupSubresource::ContinueResponse(pending)
                } else {
                    CleanupSubresource::Response(pending)
                }
            }))
            .filter_map(|resource| {
                admit_subresource_cleanup(conn, owner, resource, &subresource_error)
            })
            .collect();
        let mut resumptions = Vec::new();
        let resume_requests = resume_navigations && !policy.as_ref().is_some_and(Result::is_err);
        let requests = requests
            .into_iter()
            .filter_map(|mut pending| {
                if resume_requests && pending.document_navigation_token.is_some() {
                    if matches!(reason, FetchCleanupReason::Detached)
                        && pending.navigation.owner == *owner
                        && let Some((browser_context_id, Some(target_id))) =
                            conn.target_owner_identity_for_owner(owner)
                    {
                        // The request belongs to the target after its initiating
                        // DevTools session leaves. That session no longer needs
                        // a Page.navigate reply, but other listeners still do.
                        pending.navigation.session_id = conn
                            .page_event_session_ids_for_owner(owner)
                            .into_iter()
                            .flatten()
                            .find(|session| Some(session.as_str()) != owner.session_id());
                        pending.navigation.navigate_id = None;
                        pending.navigation.owner = CommandOwnerScope::for_route(
                            crate::conn::CdpSessionRoute::PageTarget {
                                browser_context_id,
                                target_id,
                                session_key: moli_page_types::DevToolsSessionKey::Primary,
                            },
                        );
                    }
                    let termination = NavigationTermination::new(
                        conn,
                        &pending.navigation.owner,
                        pending.document_navigation_token.as_ref(),
                        &pending.navigation,
                    );
                    resumptions.push((pending, termination));
                    None
                } else {
                    Some(pending)
                }
            })
            .collect::<Vec<_>>();
        let navigations = requests
            .into_iter()
            .map(|pending| (pending.document_navigation_token, pending.navigation))
            .chain(
                auths
                    .into_iter()
                    .map(|pending| (pending.document_navigation_token, pending.navigation)),
            )
            .chain(responses.into_iter().map(|pending| {
                let (token, state, _) = pending.fail(anyhow::Error::new(reason));
                (token, state)
            }))
            .map(|(token, state)| {
                let termination = NavigationTermination::new(conn, owner, token.as_ref(), &state);
                let command_id = state.navigate_id;
                let session = state.owner.session_id().map(str::to_owned);
                let mut output = CommandOutputBuffer::default();
                let pending = token.map(|token| {
                    let failure = if preserve_document {
                        network::materialize_navigation_failure_preserving_committed_document(
                            conn,
                            &state,
                            reason.to_string(),
                        )
                    } else {
                        network::materialize_navigation_load_result(
                            conn,
                            &state,
                            Err(anyhow::Error::new(reason)),
                        )
                    };
                    let network::MaterializedNavigationLoadOutcome::Failed(failure) = failure
                    else {
                        unreachable!("a failure cannot create a Document")
                    };
                    page::prepare_failed_navigation_cleanup(
                        conn,
                        &mut output,
                        token,
                        state,
                        failure,
                    )
                });
                if pending.is_none() {
                    output.push_error_after_messages(-32000, "Navigation aborted");
                }
                NavigationCleanup {
                    plan: output
                        .into_plan()
                        .into_background_event_plan(command_id, session.as_deref()),
                    pending,
                    termination,
                }
            })
            .collect();
        Self {
            access: conn.protocol_state_access(),
            policy,
            navigations,
            resumptions,
            subresources,
        }
    }

    pub(in crate::domains) fn take_navigation_events_for_context_disposal(
        &mut self,
    ) -> Vec<crate::conn::BackgroundProtocolEvent> {
        self.navigations
            .iter_mut()
            .flat_map(|navigation| {
                let mut plan = std::mem::take(&mut navigation.plan);
                // The Browser lifetime command does not join unrelated Page
                // streams. Their existing ingress still owns each native FIFO.
                plan.take_renderer_output_predecessor();
                plan.into_background_events(None, None)
            })
            .collect()
    }

    async fn wait_policy(&mut self) -> FetchCleanupOutput {
        let mut output = FetchCleanupOutput::default();
        if let Some(policy) = self.policy.take() {
            let completed = match policy {
                Ok(pending) => pending.wait().await,
                Err(error) => Err(error),
            };
            if let Ok(completed) = &completed
                && let Some(predecessor) = completed.renderer_output_predecessor()
            {
                output.plan.set_renderer_output_predecessor(predecessor);
            }
            output.error = completed
                .and_then(|completed| completed.into_unit_reply())
                .err()
                // Cancellation may retire the original native Page before
                // its already-admitted policy update runs. It has no policy
                // left to clear; failures on a live Page remain errors.
                .filter(|error| {
                    !matches!(
                        error.downcast_ref::<moli_core::RendererPageAccessError>(),
                        Some(
                            moli_core::RendererPageAccessError::Missing { .. }
                                | moli_core::RendererPageAccessError::Retiring { .. }
                                | moli_core::RendererPageAccessError::DocumentReplaced
                        )
                    )
                });
        }
        output
    }

    pub(in crate::domains) async fn run_for_target_close(
        mut self,
        driver: &mut ProtocolWorkflowDriver,
    ) -> FetchCleanupOutput {
        let mut output = self.wait_policy().await;
        // A policy ACK may carry a historical cursor from another Page.
        // Only the failures/replays below precede this target's retirement.
        output.plan.take_renderer_output_predecessor();
        self.finish_using(driver, output).await
    }

    pub(super) async fn wait(mut self) -> CompletedFetchCleanup {
        let output = self.wait_policy().await;
        let access = self.access.clone();
        self.finish_using(&mut ProtocolWorkflowDriver::new(access), output)
            .await
    }

    pub(in crate::domains) async fn run_using(
        mut self,
        driver: &mut ProtocolWorkflowDriver,
    ) -> FetchCleanupOutput {
        let output = self.wait_policy().await;
        self.finish_using(driver, output).await
    }

    async fn finish_using(
        self,
        driver: &mut ProtocolWorkflowDriver,
        mut output: FetchCleanupOutput,
    ) -> FetchCleanupOutput {
        for (pending, mut termination) in self.resumptions {
            if output.error.is_none() {
                match driver.with_state(move |state| state.resume_fetch_navigation_request(pending))
                {
                    Ok(Some(plan)) => {
                        output.plan.extend(plan);
                        if let Some(termination) = termination.as_mut() {
                            termination.disarm();
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        output.error.get_or_insert(error);
                    }
                }
            }
        }
        for navigation in self.navigations {
            output.plan.extend(navigation.plan);
            output.terminations.extend(navigation.termination);
            if let Some(pending) = navigation.pending {
                match pending.wait_using(driver).await {
                    Ok(events) => output.plan.extend_background_events(events),
                    Err(error) => {
                        output.error.get_or_insert(error);
                    }
                }
            }
        }
        for pending in self.subresources {
            if let Some(plan) = pending.wait(driver).await {
                output.plan.extend(plan);
            }
        }
        output
    }
}

impl CompletedFetchCleanup {}

enum CleanupSubresource {
    Request(PendingSubresourceFetchRequest),
    ContinueRequest {
        request_id: String,
        pending: PendingSubresourceFetchRequest,
    },
    Auth(PendingSubresourceFetchAuthRequest),
    ContinueAuth {
        request_id: String,
        pending: PendingSubresourceFetchAuthRequest,
    },
    Response(PendingSubresourceFetchResponseRequest),
    ContinueResponse(PendingSubresourceFetchResponseRequest),
}

enum PreparedSubresourceCleanup {
    Command(Box<PreparedSubresourceCommand>),
    ChainedPause(Box<crate::conn::BackgroundProtocolEvent>),
}

struct PreparedSubresourceCommand {
    plan: CommandOutputPlan,
    pending: moli_core::page::PendingResourceCommand,
    owner: CommandOwnerScope,
    binding: Option<TargetRootDocumentProtocolAttachmentIdentity>,
    resource: CleanupSubresource,
    correlation: Option<super::state::PreparedSubresourceCorrelation>,
}

fn admit_subresource_cleanup(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    resource: CleanupSubresource,
    reason: &str,
) -> Option<PreparedSubresourceCleanup> {
    use moli_core::RendererPageCommand;
    let resource = match resource {
        CleanupSubresource::ContinueRequest {
            pending: request, ..
        } if request
            .request_stage_pause_state()
            .is_some_and(|chain| !chain.remaining_sessions.is_empty()) =>
        {
            let event =
                super::subresource::next_chained_subresource_request_pause_event(conn, request)?;
            return Some(PreparedSubresourceCleanup::ChainedPause(Box::new(event)));
        }
        CleanupSubresource::ContinueResponse(request)
            if request
                .response_stage_pause_state()
                .is_some_and(|chain| !chain.remaining_sessions.is_empty()) =>
        {
            let event =
                super::subresource::next_chained_subresource_response_pause_event(conn, request)?;
            return Some(PreparedSubresourceCleanup::ChainedPause(Box::new(event)));
        }
        CleanupSubresource::ContinueAuth { pending, .. }
            if pending
                .auth_stage_pause_state()
                .is_some_and(|chain| !chain.remaining_sessions.is_empty()) =>
        {
            let event = super::auth::next_chained_subresource_auth_required_event(
                conn,
                owner.session_id(),
                pending,
            )?;
            return Some(PreparedSubresourceCleanup::ChainedPause(Box::new(event)));
        }
        resource => resource,
    };
    let response_candidate = match &resource {
        CleanupSubresource::ContinueRequest {
            pending: request, ..
        } => conn
            .target_fetch_subresource_interception_snapshot_for_owner(owner)
            .is_some_and(|snapshot| {
                snapshot.has_response_stage_candidate(request.resource_type.into())
            }),
        _ => false,
    };
    let handle_auth_requests = conn.target_fetch_handle_auth_requests_for_owner(owner);
    let (target, command) = match &resource {
        CleanupSubresource::ContinueRequest {
            pending: request, ..
        } => {
            let (url, method, body, headers) =
                request.accumulated_request_stage_continue_modifications();
            (
                request.document_command_target(),
                RendererPageCommand::ContinuePendingSubresourceFetch {
                    internal_id: request.internal_id,
                    url,
                    method,
                    body,
                    headers,
                    intercept_response: response_candidate,
                    handle_auth_requests,
                },
            )
        }
        CleanupSubresource::Request(request) => {
            if let Some(continuation) = request.detached_parser_script_fetch_continuation() {
                continuation.fail(reason.to_owned());
                return None;
            }
            (
                request.document_command_target(),
                RendererPageCommand::FailPendingSubresourceFetch {
                    internal_id: request.internal_id,
                    error_text: reason.to_owned(),
                },
            )
        }
        CleanupSubresource::Auth(request) => (
            request.command_target.as_ref(),
            RendererPageCommand::FailPendingSubresourceAuth {
                internal_id: request.internal_id,
                error_text: reason.to_owned(),
            },
        ),
        CleanupSubresource::ContinueAuth { pending, .. } => (
            pending.command_target.as_ref(),
            RendererPageCommand::CancelPendingSubresourceAuth {
                internal_id: pending.internal_id,
            },
        ),
        CleanupSubresource::ContinueResponse(request) => (
            request.command_target.as_ref(),
            RendererPageCommand::ContinuePendingSubresourceResponse {
                internal_id: request.internal_id,
                response_code: None,
                response_headers: None,
            },
        ),
        CleanupSubresource::Response(request) => (
            request.command_target.as_ref(),
            RendererPageCommand::FailPendingSubresourceResponse {
                internal_id: request.internal_id,
                error_text: reason.to_owned(),
            },
        ),
    };
    let target = target?;
    let correlation = match &resource {
        CleanupSubresource::ContinueRequest {
            pending: request,
            request_id,
        } if target.has_native_network_pause() => {
            super::state::PreparedSubresourceCorrelation::prepare_automatic(
                conn,
                owner,
                Some(request_id.clone()),
                request.clone(),
                if response_candidate {
                    super::subresource_continuation::SubresourceResponseInterception::Candidate
                } else {
                    super::subresource_continuation::SubresourceResponseInterception::None
                },
            )
        }
        CleanupSubresource::ContinueAuth {
            request_id,
            pending,
        } if pending.intercept_response => {
            let continued = super::auth::continued_subresource_request(pending);
            Some(
                super::state::PreparedSubresourceCorrelation::prepare_deferred_response_stage(
                    conn, owner, request_id, &continued,
                )?,
            )
        }
        _ => None,
    };
    // Cleanup owns this exact request even after session Closing or Page
    // retirement. The native Document guard prevents touching a replacement.
    let pending = match target.start_resource_command(command) {
        Ok(pending) => pending,
        Err(_) => {
            if let Some(correlation) = correlation {
                correlation.rollback(conn);
            }
            return None;
        }
    };
    let mut plan = CommandOutputPlan::default();
    if matches!(
        &pending,
        moli_core::page::PendingResourceCommand::Network(Ok(_))
    ) {
        // Retiring a Document must publish its admitted cancellation before
        // that Document's networking tasks become ineligible. The load's
        // late terminal shares this ID and uses the existing once-only claim.
        let failed = match &resource {
            CleanupSubresource::Request(request) => Some((
                &request.network_request_id,
                &request.frame_id,
                request.resource_type,
            )),
            CleanupSubresource::Auth(request) => Some((
                &request.network_request_id,
                &request.frame_id,
                request.resource_type,
            )),
            CleanupSubresource::Response(request) => Some((
                &request.network_request_id,
                &request.frame_id,
                request.resource_type,
            )),
            _ => None,
        };
        if let Some((request_id, frame_id, resource_type)) = failed {
            let sessions = conn.network_event_session_ids_for_owner(owner);
            let loader = conn
                .current_document_loader_id_for_owner(owner)
                .unwrap_or_else(|| crate::conn::DEFAULT_LOADER_ID.to_owned());
            if conn
                .runtime_session_owner_slot_mut_for_owner(owner)
                .is_some_and(|slot| slot.claim_completed_subresource_request_id(request_id))
            {
                let mut events = Vec::new();
                for session in sessions {
                    network::emit_loading_failed(
                        &mut events,
                        session.as_deref(),
                        request_id,
                        frame_id,
                        &loader,
                        crate::conn::monotonic_timestamp_seconds(),
                        reason,
                        resource_type.into(),
                    );
                }
                plan.extend_background_events(events);
            }
        }
    }
    let binding = conn.target_root_document_output_identity_for_owner(owner, target.document());
    Some(PreparedSubresourceCleanup::Command(Box::new(
        PreparedSubresourceCommand {
            plan,
            pending,
            owner: owner.clone(),
            binding,
            resource,
            correlation,
        },
    )))
}

impl PreparedSubresourceCleanup {
    async fn wait(self, driver: &mut ProtocolWorkflowDriver) -> Option<CommandOutputPlan> {
        match self {
            Self::Command(command) => command.wait(driver).await,
            Self::ChainedPause(event) => {
                let mut plan = CommandOutputPlan::default();
                plan.push_background_event(*event);
                Some(plan)
            }
        }
    }
}

impl PreparedSubresourceCommand {
    async fn wait(self, driver: &mut ProtocolWorkflowDriver) -> Option<CommandOutputPlan> {
        let completed = self.pending.wait().await;
        let completed = CompletedSubresourceCleanup {
            plan: self.plan,
            binding: self.binding,
            owner: self.owner,
            resource: self.resource,
            completed,
            correlation: self.correlation,
        };
        driver
            .with_state(move |state| state.finish_fetch_cleanup_subresource(completed))
            .ok()
            .flatten()
    }
}

pub(crate) struct CompletedSubresourceCleanup {
    plan: CommandOutputPlan,
    binding: Option<TargetRootDocumentProtocolAttachmentIdentity>,
    owner: CommandOwnerScope,
    resource: CleanupSubresource,
    completed: Result<moli_core::page::CompletedResourceCommand>,
    correlation: Option<super::state::PreparedSubresourceCorrelation>,
}

impl CompletedSubresourceCleanup {
    pub(crate) fn finish(self, conn: &mut CdpConnection) -> Option<CommandOutputPlan> {
        let Self {
            mut plan,
            binding,
            owner,
            resource,
            completed,
            correlation,
        } = self;
        let completed = match completed {
            Ok(completed) => completed,
            Err(_) => {
                if let Some(correlation) = correlation {
                    correlation.rollback(conn);
                }
                return Some(plan);
            }
        };
        let predecessor = completed.renderer_output_predecessor();
        let result = if matches!(&resource, CleanupSubresource::ContinueRequest { .. }) {
            completed.into_subresource_continue_outcome().map(|_| ())
        } else {
            completed.into_unit_reply()
        };
        if result.is_err() {
            if let Some(correlation) = correlation {
                correlation.rollback(conn);
            }
            return None;
        }
        if let Some(predecessor) = predecessor {
            plan.set_renderer_output_predecessor(predecessor);
        }
        // An ACK belongs to the admitted native turn, never to a replacement
        // Page's mutable snapshot or network correlation table.
        if binding
            .as_ref()
            .is_none_or(|binding| !conn.target_root_document_output_identity_is_current(binding))
        {
            if let Some(correlation) = correlation {
                correlation.rollback(conn);
            }
            return Some(plan);
        }
        let mut events = Vec::new();
        match resource {
            CleanupSubresource::Request(pending)
            | CleanupSubresource::ContinueRequest { pending, .. } => {
                activity::flush_post_subresource_fetch_request_activity_for_owner_background_events(
                    conn,
                    &mut events,
                    &owner,
                    &pending,
                )
            }
            CleanupSubresource::Auth(pending)
            | CleanupSubresource::ContinueAuth { pending, .. } => {
                activity::flush_post_subresource_auth_activity_for_owner_background_events(
                    conn,
                    &mut events,
                    &owner,
                    &pending,
                )
            }
            CleanupSubresource::Response(pending)
            | CleanupSubresource::ContinueResponse(pending) => {
                activity::flush_post_subresource_response_activity_for_owner_background_events(
                    conn,
                    &mut events,
                    &owner,
                    &pending,
                )
            }
        }
        plan.extend_background_events(events);
        Some(plan)
    }
}
