use anyhow::Result;

use super::{FetchDisablePendingState, navigation::NavigationTermination};
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
    subresources: Vec<PreparedSubresourceFailure>,
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
        pending: FetchDisablePendingState,
        policy: Option<Result<PendingPageCommand>>,
        reason: FetchCleanupReason,
        preserve_document: bool,
    ) -> Self {
        let (requests, auths, responses, subrequests, subauths, subresponses) = pending;
        let subresource_error = match reason {
            FetchCleanupReason::NavigationStopped => {
                moli_fetch::NET_ERR_ABORTED_ERROR_TEXT.to_owned()
            }
            _ => reason.to_string(),
        };
        // Admit every native failure before releasing a Page. Dropping this
        // command cannot leave a pause stranded in a removed Fetch handler.
        let subresources = subrequests
            .into_iter()
            .map(|(_, pending)| CleanupSubresource::Request(pending))
            .chain(
                subauths
                    .into_iter()
                    .map(|(_, pending)| CleanupSubresource::Auth(pending)),
            )
            .chain(
                subresponses
                    .into_iter()
                    .map(|(_, pending)| CleanupSubresource::Response(pending)),
            )
            .filter_map(|resource| {
                admit_subresource_failure(conn, owner, resource, &subresource_error)
            })
            .collect();
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
    Auth(PendingSubresourceFetchAuthRequest),
    Response(PendingSubresourceFetchResponseRequest),
}

struct PreparedSubresourceFailure {
    pending: PendingPageCommand,
    owner: CommandOwnerScope,
    binding: Option<TargetRootDocumentProtocolAttachmentIdentity>,
    resource: CleanupSubresource,
}

fn admit_subresource_failure(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    resource: CleanupSubresource,
    reason: &str,
) -> Option<PreparedSubresourceFailure> {
    use moli_core::RendererPageCommand;
    let (target, command) = match &resource {
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
        CleanupSubresource::Response(request) => (
            request.command_target.as_ref(),
            RendererPageCommand::FailPendingSubresourceResponse {
                internal_id: request.internal_id,
                error_text: reason.to_owned(),
            },
        ),
    };
    let target = target?;
    // Cleanup owns this exact request even after session Closing or Page
    // retirement. The native Document guard prevents touching a replacement.
    let pending = target.start(command).ok()?;
    let binding = conn.target_root_document_output_identity_for_owner(owner, target.document());
    Some(PreparedSubresourceFailure {
        pending,
        owner: owner.clone(),
        binding,
        resource,
    })
}

impl PreparedSubresourceFailure {
    async fn wait(self, driver: &mut ProtocolWorkflowDriver) -> Option<CommandOutputPlan> {
        let completed = self.pending.wait().await.ok()?;
        let completed = CompletedSubresourceFailure {
            binding: self.binding,
            owner: self.owner,
            resource: self.resource,
            completed,
        };
        driver
            .with_state(move |state| state.finish_fetch_cleanup_subresource(completed))
            .ok()
            .flatten()
    }
}

pub(crate) struct CompletedSubresourceFailure {
    binding: Option<TargetRootDocumentProtocolAttachmentIdentity>,
    owner: CommandOwnerScope,
    resource: CleanupSubresource,
    completed: moli_core::page::CompletedPageCommand,
}

impl CompletedSubresourceFailure {
    pub(crate) fn finish(self, conn: &mut CdpConnection) -> Option<CommandOutputPlan> {
        let Self {
            binding,
            owner,
            resource,
            completed,
        } = self;
        let predecessor = completed.renderer_output_predecessor();
        completed.into_unit_reply().ok()?;
        let mut plan = CommandOutputPlan::default();
        if let Some(predecessor) = predecessor {
            plan.set_renderer_output_predecessor(predecessor);
        }
        // A unit ACK belongs to the admitted native turn, never to a replacement
        // Page's mutable snapshot or network correlation table.
        if binding
            .as_ref()
            .is_none_or(|binding| !conn.target_root_document_output_identity_is_current(binding))
        {
            return Some(plan);
        }
        let mut events = Vec::new();
        match resource {
            CleanupSubresource::Request(pending) => {
                activity::flush_post_subresource_fetch_request_activity_for_owner_background_events(
                    conn,
                    &mut events,
                    &owner,
                    &pending,
                )
            }
            CleanupSubresource::Auth(pending) => {
                activity::flush_post_subresource_auth_activity_for_owner_background_events(
                    conn,
                    &mut events,
                    &owner,
                    &pending,
                )
            }
            CleanupSubresource::Response(pending) => {
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
