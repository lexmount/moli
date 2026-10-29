use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, Cmd, CommandOwnerScope, DEFAULT_LOADER_ID,
    PendingFetchNavigation, PendingSubresourceFetchAuthRequest, PendingSubresourceFetchRequest,
    PendingSubresourceFetchResponseRequest, monotonic_timestamp_seconds,
};
use crate::domains::network;
use anyhow::Result;

use super::PageCommandTaskStep;
use crate::domains::command_output::CommandOutputPlan;

mod target;
pub(crate) use target::{
    CompletedTargetTermination, TargetTermination, TargetTerminationSelection,
};

#[derive(Clone, Debug)]
pub(crate) struct PageTargetTerminationOwnerAction {
    owner_scope: CommandOwnerScope,
    target_id: String,
    host: Option<crate::conn::PageTargetHostIdentity>,
}

impl PageTargetTerminationOwnerAction {
    pub(crate) fn new(
        conn: &CdpConnection,
        owner_scope: CommandOwnerScope,
        target_id: String,
    ) -> Self {
        let host = conn.page_target_host_identity_for_owner(&owner_scope);
        // An admitted close belongs to the target, including after its
        // initiating frontend session detaches.
        let owner_scope = conn
            .target_session_route_for_target_id(&target_id)
            .map(CommandOwnerScope::for_route)
            .unwrap_or(owner_scope);
        Self {
            owner_scope,
            target_id,
            host,
        }
    }

    pub(crate) fn owner_scope(&self) -> &CommandOwnerScope {
        &self.owner_scope
    }

    pub(crate) fn target_id(&self) -> &str {
        &self.target_id
    }

    pub(crate) fn is_current_host(&self, conn: &CdpConnection) -> bool {
        self.host.as_ref().is_some_and(|host| {
            conn.page_target_host_identity_for_owner(&self.owner_scope)
                .is_some_and(|current| host.matches(&current))
        })
    }

    fn into_parts(
        self,
    ) -> (
        CommandOwnerScope,
        String,
        Option<crate::conn::PageTargetHostIdentity>,
    ) {
        (self.owner_scope, self.target_id, self.host)
    }
}

fn take_pending_fetch_state_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
) -> (
    Vec<PendingFetchNavigation>,
    Vec<crate::conn::PendingFetchAuthNavigation>,
    Vec<crate::conn::PausedDocumentTransfer>,
    Vec<(String, PendingSubresourceFetchRequest)>,
    Vec<(String, crate::conn::PendingSubresourceFetchAuthRequest)>,
    Vec<(String, crate::conn::PendingSubresourceFetchResponseRequest)>,
) {
    conn.take_pending_fetch_state_for_owner(owner)
        .unwrap_or_else(|| {
            (
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )
        })
}

/// Completes protocol-owned subresource pauses after the renderer has crashed.
///
/// A normal Fetch failure is first applied to the Page owner and then projected
/// from its network backlog. `Page.crash` cannot use that path: the Page owner
/// may be blocked in JavaScript, and the IO termination which unblocks it also
/// retires the Page residence. The pending Fetch residences were already
/// claimed by [`take_pending_fetch_state`], so emitting their terminal network
/// state here is both race-free and independent of the renderer owner.
fn fail_crashed_subresource_fetches_background_events(
    conn: &CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    owner: &CommandOwnerScope,
    error_text: &str,
    pending_subresource_fetches: Vec<(String, PendingSubresourceFetchRequest)>,
    pending_subresource_auths: Vec<(String, PendingSubresourceFetchAuthRequest)>,
    pending_subresource_responses: Vec<(String, PendingSubresourceFetchResponseRequest)>,
) {
    let loader_id = conn
        .current_document_loader_id_for_owner(owner)
        .unwrap_or_else(|| DEFAULT_LOADER_ID.to_owned());
    let event_session_ids = conn.network_event_session_ids_for_owner(owner);
    let timestamp = monotonic_timestamp_seconds();

    let mut emit_failure =
        |network_request_id: &str,
         frame_id: &str,
         resource_type: moli_core::page::SubresourceResourceType| {
            for event_session_id in &event_session_ids {
                network::emit_loading_failed(
                    out,
                    event_session_id.as_deref(),
                    network_request_id,
                    frame_id,
                    &loader_id,
                    timestamp,
                    error_text,
                    resource_type.into(),
                );
            }
        };

    for (_, pending) in pending_subresource_fetches {
        if let Some(continuation) = pending.detached_parser_script_fetch_continuation() {
            let _ = continuation.fail(error_text.to_owned());
        }
        emit_failure(
            &pending.network_request_id,
            &pending.frame_id,
            pending.resource_type,
        );
    }
    for (_, pending) in pending_subresource_auths {
        emit_failure(
            &pending.network_request_id,
            &pending.frame_id,
            pending.resource_type,
        );
    }
    for (_, pending) in pending_subresource_responses {
        emit_failure(
            &pending.network_request_id,
            &pending.frame_id,
            pending.resource_type,
        );
    }
}

pub(super) fn try_start_stop_loading_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let owner_scope = crate::conn::CommandOwnerScope::capture(conn, cmd.session_id);
    let prepared = PreparedPageStopLoading::prepare(conn, &owner_scope);
    PageCommandTaskStep::Pending(super::PendingPageCommandDispatch {
        command_id: cmd.id,
        owner_scope,
        kind: Box::new(super::PendingPageCommandKind::StopLoading(prepared)),
    })
}

pub(crate) struct PreparedPageStopLoading {
    lifecycle: Option<(
        crate::conn::TargetPageResidenceIdentity,
        Result<moli_core::page::PendingPageCommand>,
    )>,
    cleanup: crate::domains::fetch::PreparedFetchCleanup,
}

impl PreparedPageStopLoading {
    pub(crate) fn prepare(conn: &mut CdpConnection, owner: &CommandOwnerScope) -> Self {
        let lifecycle = conn
            .target_page_residence_identity_for_owner(owner)
            .and_then(|page_owner| {
                conn.loaded_page_mut_for_owner(owner)
                    .map(|page| (page_owner, page.start_stop_document_lifecycle()))
            });
        // Capture the interrupted work now. A navigation admitted while the
        // original Native stop waits must not become part of this cleanup.
        let pending = take_pending_fetch_state_for_owner(conn, owner);
        let cleanup = crate::domains::fetch::prepare_page_stop_loading(conn, owner, pending);
        Self { lifecycle, cleanup }
    }

    pub(crate) async fn run(
        self,
        driver: &mut crate::conn::ProtocolWorkflowDriver,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Result<CommandOutputPlan> {
        if let Some((owner, pending)) = self.lifecycle {
            let completed = match pending {
                Ok(pending) => pending.wait().await,
                Err(error) => Err(error),
            };
            let result = match completed {
                Ok(completed) => context.with_state(driver, move |state, context| {
                    state.finish_page_stop_lifecycle(owner, completed, context.context_mut())
                })?,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                tracing::debug!(%error, "failed to stop renderer document lifecycle");
            }
        }
        let mut events = Vec::new();
        let result = self
            .cleanup
            .run_using(driver)
            .await
            .into_disposal_output(&mut events);
        if let Some(error) = result.first_error {
            tracing::debug!(%error, "failed to clear pending Fetch work while stopping navigation");
        }
        let mut plan = CommandOutputPlan::success();
        plan.extend_background_events(events);
        if let Some(predecessor) = result.predecessor {
            plan.set_renderer_output_predecessor(predecessor);
        }
        Ok(plan)
    }
}

pub(super) fn try_start_crash_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> PageCommandTaskStep {
    match cmd.get_params::<serde_json::Value>() {
        Ok(Some(_)) | Ok(None) => {}
        Err(_) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    }
    let owner_scope = crate::conn::CommandOwnerScope::capture(conn, cmd.session_id);
    let prepared = match PreparedPageCrash::prepare(conn, &owner_scope, command_context) {
        Ok(prepared) => prepared,
        Err(plan) => return PageCommandTaskStep::Complete(plan),
    };
    PageCommandTaskStep::Pending(super::PendingPageCommandDispatch {
        command_id: cmd.id,
        owner_scope,
        kind: Box::new(super::PendingPageCommandKind::Crash(Box::new(prepared))),
    })
}

pub(crate) struct PreparedPageCrash {
    cleanup: crate::domains::fetch::PreparedFetchCleanup,
    page: Option<moli_core::page::Page>,
    failures: Vec<BackgroundProtocolEvent>,
    subresource_failures: Vec<BackgroundProtocolEvent>,
    crashed: Vec<BackgroundProtocolEvent>,
    action: PageTargetTerminationOwnerAction,
}

impl PreparedPageCrash {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        context: &mut crate::conn::CommandDispatchContext,
    ) -> Result<Self, CommandOutputPlan> {
        let Some((_, Some(target_id))) = conn.target_owner_identity_for_owner(owner) else {
            return Err(CommandOutputPlan::error_without_session(
                -31998,
                super::missing_page_target_error_message(conn, owner.session_id()),
            ));
        };
        let action = PageTargetTerminationOwnerAction::new(conn, owner.clone(), target_id.clone());
        let mut pending = take_pending_fetch_state_for_owner(conn, owner);
        if let Some(page) = conn.loaded_page_mut_for_owner(owner) {
            page.crash_devtools_target_from_io();
        }
        let sessions = conn.page_event_session_ids_for_owner(owner);
        let mut failures = Vec::new();
        for session in &sessions {
            let inspector = session
                .as_deref()
                .map(CommandOwnerScope::for_session)
                .unwrap_or_else(|| owner.clone());
            conn.fail_pending_inspector_awaits_for_owner_background_events_into(
                &mut failures,
                context.protocol_events_mut(),
                &inspector,
                "Page crashed",
            );
            conn.with_target_devtools_session_state_for_owner_mut(&inspector, |state| {
                state
                    .runtime_session_state
                    .record_inspector_target_crashed();
            });
        }
        let mut subresource_failures = Vec::new();
        fail_crashed_subresource_fetches_background_events(
            conn,
            &mut subresource_failures,
            owner,
            "Page crashed",
            std::mem::take(&mut pending.3),
            std::mem::take(&mut pending.4),
            std::mem::take(&mut pending.5),
        );
        let cleanup = crate::domains::fetch::prepare_page_crash(conn, owner, pending);
        let page = conn.take_crashed_page_for_owner(owner);
        let mut crashed = sessions
            .into_iter()
            .map(|session| BackgroundProtocolEvent::inspector_target_crashed(session.as_deref()))
            .collect::<Vec<_>>();
        crashed
            .extend(conn.target_crashed_events_for_all_discovery_owners(&target_id, "crashed", 1));
        Ok(Self {
            cleanup,
            page,
            failures,
            subresource_failures,
            crashed,
            action,
        })
    }

    pub(crate) fn finish_events(
        conn: &CdpConnection,
        action: PageTargetTerminationOwnerAction,
        events: Vec<BackgroundProtocolEvent>,
    ) -> Vec<BackgroundProtocolEvent> {
        if action.is_current_host(conn)
            && conn
                .target_owner_state_for_owner(action.owner_scope())
                .is_some_and(|state| state.target_crash_state.is_crashed())
        {
            events
        } else {
            Vec::new()
        }
    }

    pub(crate) async fn run(
        self,
        driver: &mut crate::conn::ProtocolWorkflowDriver,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Result<CommandOutputPlan> {
        let mut events = self.failures;
        let result = self
            .cleanup
            .run_using(driver)
            .await
            .into_disposal_output(&mut events);
        events.extend(self.subresource_failures);
        if let Some(predecessor) = result.predecessor {
            context
                .context_mut()
                .set_renderer_output_predecessor(predecessor);
        }
        if let Some(error) = result.first_error {
            tracing::debug!(%error, "Page crash Fetch cleanup failed");
        }
        if let Some(page) = self.page {
            let _ = page.close_async().await;
        }
        let crashed = driver
            .with_state(move |state| state.finish_page_crash_events(self.action, self.crashed))?;
        events.extend(crashed);
        let mut plan = CommandOutputPlan::success();
        plan.extend_background_events(events);
        Ok(plan)
    }
}

pub(super) fn try_start_close_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> PageCommandTaskStep {
    let owner_scope = crate::conn::CommandOwnerScope::capture(conn, cmd.session_id);
    let prepared =
        match PreparedPageTargetTermination::try_prepare(conn, &owner_scope, command_context) {
            Ok(prepared) => prepared,
            Err(plan) => return PageCommandTaskStep::Complete(plan),
        };
    PageCommandTaskStep::Pending(super::PendingPageCommandDispatch {
        command_id: cmd.id,
        owner_scope,
        kind: Box::new(super::PendingPageCommandKind::Close(prepared)),
    })
}

pub(crate) struct PreparedPageTargetTermination {
    cleanup: crate::domains::fetch::PreparedFetchCleanup,
    action: PageTargetTerminationOwnerAction,
}

impl PreparedPageTargetTermination {
    pub(crate) fn try_prepare(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        context: &mut crate::conn::CommandDispatchContext,
    ) -> Result<Self, CommandOutputPlan> {
        let Some((_, Some(target_id))) = conn.target_owner_identity_for_owner(owner) else {
            return Err(CommandOutputPlan::error_without_session(
                -31998,
                super::missing_page_target_error_message(conn, owner.session_id()),
            ));
        };
        Ok(Self::prepare(conn, owner, target_id, context))
    }

    pub(super) fn prepare(
        conn: &mut CdpConnection,
        owner: &CommandOwnerScope,
        target_id: String,
        command_context: &mut crate::conn::CommandDispatchContext,
    ) -> Self {
        let action = PageTargetTerminationOwnerAction::new(conn, owner.clone(), target_id);
        conn.mark_page_target_closing(owner);
        let pending = take_pending_fetch_state_for_owner(conn, owner);
        let mut events = Vec::new();
        conn.fail_pending_inspector_awaits_for_owner_background_events_into(
            &mut events,
            command_context.protocol_events_mut(),
            owner,
            "Page closed",
        );
        command_context.protocol_events_mut().extend(events);
        let cleanup = crate::domains::fetch::prepare_page_close(conn, owner, pending);
        Self { cleanup, action }
    }

    pub(crate) async fn run(
        self,
        driver: &mut crate::conn::ProtocolWorkflowDriver,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Result<()> {
        let result = self
            .cleanup
            .run_using(driver)
            .await
            .into_disposal_output(context.context_mut().protocol_events_mut());
        if let Some(predecessor) = result.predecessor {
            context
                .context_mut()
                .set_renderer_output_predecessor(predecessor);
        }
        if let Some(error) = result.first_error {
            tracing::warn!(%error, "Page close cleanup failed; retaining target retirement");
        }
        driver.with_state(move |state| state.publish_page_output_close(self.action))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn::BrowserContext;

    #[tokio::test]
    async fn admitted_target_termination_survives_its_initiating_session() {
        let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
        let mut context = BrowserContext::new("BID-close-initiator".into());
        context.set_active_target_id("TID-close-initiator");
        context.attach_active_session("SID-primary");
        assert!(
            context
                .assign_attached_session_to_target("TID-close-initiator", "SID-initiator".into())
        );
        ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(context));
        let action = ctx.with_fixture(|conn| {
            PageTargetTerminationOwnerAction::new(
                conn,
                CommandOwnerScope::for_session("SID-initiator"),
                "TID-close-initiator".into(),
            )
        });
        let handle = ctx.handle();
        let detached = ctx.drive_owned_wait(handle.process_message_for_test(r#"{"id":2,"method":"Target.detachFromTarget","params":{"sessionId":"SID-initiator"}}"#)).await.unwrap();
        assert!(
            detached
                .into_parts()
                .0
                .iter()
                .any(|message| message["id"] == 2 && message.get("result").is_some())
        );
        ctx.with_fixture(|conn| assert!(!conn.has_attached_devtools_session("SID-initiator")));

        let work = ctx.with_fixture(|conn| {
            conn.publish_page_target_termination_owner_action(action);
            conn.take_scheduler_events().into_iter().find_map(|event| match event {
                crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } if work.kind() == crate::ProtocolSchedulerWorkKind::PageTargetTerminationOwnerAction => Some(work),
                _ => None,
            }).unwrap()
        });
        ctx.run_fixture_scheduler_work(work).await;
        ctx.with_fixture(|conn| {
            assert!(
                conn.browser_context
                    .as_ref()
                    .unwrap()
                    .page_target("TID-close-initiator")
                    .is_none()
            )
        });
        ctx.with_fixture(|conn| assert!(!conn.has_attached_devtools_session("SID-primary")));
    }

    #[tokio::test]
    async fn delayed_target_termination_rejects_a_recreated_host_with_the_same_ids() {
        let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
        let install = |conn: &mut CdpConnection| {
            let mut context = BrowserContext::new("BID-recreated".into());
            context.set_active_target_id("TID-recreated");
            conn.install_browser_context_fixture_for_test(context);
        };
        ctx.with_fixture(|conn| install(conn));
        let owner = ctx.with_fixture(|conn| CommandOwnerScope::capture(conn, None));
        let action = ctx.with_fixture(|conn| {
            PageTargetTerminationOwnerAction::new(conn, owner.clone(), "TID-recreated".into())
        });
        let original =
            ctx.with_fixture(|conn| conn.page_target_host_identity_for_owner(&owner).unwrap());
        ctx.with_fixture(|conn| install(conn));
        let replacement =
            ctx.with_fixture(|conn| conn.page_target_host_identity_for_owner(&owner).unwrap());
        assert!(!original.matches(&replacement));

        let work = ctx.with_fixture(|conn| {
            conn.publish_page_target_termination_owner_action(action);
            conn.take_scheduler_events().into_iter().find_map(|event| match event {
                crate::conn::CdpSchedulerEvent::ProtocolWorkPublished { work } if work.kind() == crate::ProtocolSchedulerWorkKind::PageTargetTerminationOwnerAction => Some(work),
                _ => None,
            }).unwrap()
        });
        let outcome = ctx.run_fixture_scheduler_work(work).await;
        assert!(outcome.into_parts().0.is_empty());
        ctx.with_fixture(|conn| {
            assert!(replacement.matches(&conn.page_target_host_identity_for_owner(&owner).unwrap()))
        });
        ctx.with_fixture(|conn| {
            assert!(
                conn.browser_context
                    .as_ref()
                    .unwrap()
                    .page_target("TID-recreated")
                    .is_some()
            )
        });
    }
}
