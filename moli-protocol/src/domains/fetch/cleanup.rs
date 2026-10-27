use anyhow::Result;

use super::{FetchDisablePendingState, navigation::NavigationTermination};
use crate::conn::{
    CdpConnection, CommandOwnerScope, PendingSubresourceFetchAuthRequest,
    PendingSubresourceFetchRequest, PendingSubresourceFetchResponseRequest, ProtocolStateAccess,
    ProtocolWorkflowDriver, TargetRootDocumentProtocolAttachmentIdentity,
};
use crate::domains::{
    activity,
    command_output::{CommandOutputBuffer, CommandOutputPlan},
    network, page,
};
use moli_core::page::PendingPageCommand;

pub(crate) struct PreparedFetchCleanup {
    access: Option<ProtocolStateAccess>,
    policy: Option<Result<PendingPageCommand>>,
    navigations: Vec<NavigationCleanup>,
    subresources: Vec<PreparedSubresourceFailure>,
}

struct NavigationCleanup {
    plan: CommandOutputPlan,
    pending: Option<page::PreparedNavigationFailureCleanup>,
    termination: Option<NavigationTermination>,
}

pub(super) enum CompletedFetchCleanup {
    Owned(FetchCleanupOutput),
    Inline {
        pending: PreparedFetchCleanup,
        output: FetchCleanupOutput,
    },
}

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

    #[cfg(test)]
    pub(in crate::domains) fn into_disposal_result(
        self,
        events: &mut Vec<crate::conn::BackgroundProtocolEvent>,
    ) -> Result<Option<moli_core::RendererOutputFence>> {
        let output = self.into_disposal_output(events);
        match output.first_error {
            Some(error) => Err(error),
            None => Ok(output.predecessor),
        }
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
        reason: &str,
        preserve_document: bool,
    ) -> Self {
        let (requests, auths, responses, subrequests, subauths, subresponses) = pending;
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
            .filter_map(|resource| admit_subresource_failure(conn, owner, resource, reason))
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
                let (token, state, _) = pending.fail(anyhow::Error::msg(reason.to_owned()));
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
                            reason.to_owned(),
                        )
                    } else {
                        network::materialize_navigation_load_result(
                            conn,
                            &state,
                            Err(anyhow::Error::msg(reason.to_owned())),
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
                .and_then(|completed| completed.into_unit_page_command_turn().map(drop))
                .err();
        }
        output
    }

    pub(in crate::domains) async fn run_for_target_close(
        mut self,
        driver: &mut ProtocolWorkflowDriver<'_>,
    ) -> FetchCleanupOutput {
        let mut output = self.wait_policy().await;
        // A policy ACK may carry a historical cursor from another Page.
        // Only the failures/replays below precede this target's retirement.
        output.plan.take_renderer_output_predecessor();
        self.finish_using(driver, output).await
    }

    pub(super) async fn wait(mut self) -> CompletedFetchCleanup {
        let output = self.wait_policy().await;
        let Some(access) = self.access.take() else {
            return CompletedFetchCleanup::Inline {
                pending: self,
                output,
            };
        };
        CompletedFetchCleanup::Owned(
            self.finish_using(&mut ProtocolWorkflowDriver::owned(access), output)
                .await,
        )
    }

    pub(in crate::domains) async fn run_using(
        mut self,
        driver: &mut ProtocolWorkflowDriver<'_>,
    ) -> FetchCleanupOutput {
        let output = self.wait_policy().await;
        self.finish_using(driver, output).await
    }

    #[cfg(test)]
    pub(super) async fn wait_inline(self, conn: &mut CdpConnection) -> FetchCleanupOutput {
        self.run_using(&mut ProtocolWorkflowDriver::inline(conn))
            .await
    }

    async fn finish_using(
        self,
        driver: &mut ProtocolWorkflowDriver<'_>,
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

impl CompletedFetchCleanup {
    pub(super) async fn finish(self, conn: &mut CdpConnection) -> FetchCleanupOutput {
        match self {
            Self::Owned(output) => output,
            Self::Inline { pending, output } => {
                pending
                    .finish_using(&mut ProtocolWorkflowDriver::inline(conn), output)
                    .await
            }
        }
    }
}

enum CleanupSubresource {
    Request(PendingSubresourceFetchRequest),
    Auth(PendingSubresourceFetchAuthRequest),
    Response(PendingSubresourceFetchResponseRequest),
}

struct PreparedSubresourceFailure {
    pending: PendingPageCommand,
    owner: CommandOwnerScope,
    binding: TargetRootDocumentProtocolAttachmentIdentity,
    resource: CleanupSubresource,
}

fn admit_subresource_failure(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    resource: CleanupSubresource,
    reason: &str,
) -> Option<PreparedSubresourceFailure> {
    let current = match &resource {
        CleanupSubresource::Request(pending) => {
            conn.pending_subresource_fetch_request_residence_is_current(pending)
        }
        CleanupSubresource::Auth(pending) => {
            conn.target_page_residence_identity_is_current(&pending.page_owner)
        }
        CleanupSubresource::Response(pending) => {
            conn.target_page_residence_identity_is_current(&pending.page_owner)
        }
    };
    if !current {
        return None;
    }
    let root = conn.target_root_document_lifecycle_identity_for_owner(owner)?;
    let binding = conn.target_root_document_protocol_attachment_identity_for_owner(owner, root)?;
    let page = conn.loaded_page_mut_for_owner(owner)?;
    let pending = match &resource {
        CleanupSubresource::Request(pending) => {
            page.start_fail_pending_subresource_fetch(pending.internal_id, reason.to_owned())
        }
        CleanupSubresource::Auth(pending) => {
            page.start_fail_pending_subresource_auth(pending.internal_id, reason.to_owned())
        }
        CleanupSubresource::Response(pending) => {
            page.start_fail_pending_subresource_response(pending.internal_id, reason.to_owned())
        }
    }
    .ok()?;
    Some(PreparedSubresourceFailure {
        pending,
        owner: owner.clone(),
        binding,
        resource,
    })
}

impl PreparedSubresourceFailure {
    async fn wait(self, driver: &mut ProtocolWorkflowDriver<'_>) -> Option<CommandOutputPlan> {
        let completed = self.pending.wait().await.ok()?;
        let completed = CompletedSubresourceFailure {
            binding: self.binding,
            owner: self.owner,
            resource: self.resource,
            completed,
        };
        driver
            .with_state(move |state| state.finish_fetch_cleanup_subresource(completed))
            .await
            .ok()
            .flatten()
    }
}

pub(crate) struct CompletedSubresourceFailure {
    binding: TargetRootDocumentProtocolAttachmentIdentity,
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
        completed.into_unit_page_command_turn().ok()?;
        let mut plan = CommandOutputPlan::default();
        if let Some(predecessor) = predecessor {
            plan.set_renderer_output_predecessor(predecessor);
        }
        // A unit ACK belongs to the admitted native turn, never to a replacement
        // Page's mutable snapshot or network correlation table.
        if !conn.target_root_document_protocol_attachment_identity_is_current(&binding) {
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
