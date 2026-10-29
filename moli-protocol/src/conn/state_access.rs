//! Finite synchronous protocol-state access through revocable weak handles.

use anyhow::Result;

use super::{
    CdpCommandTaskStep, CdpConnection, CommandDispatchContext, CompletedCdpCommandDispatch,
    ParsedCdpCommand,
};

mod auto_attach;
mod automation_configuration;
mod body_output;
mod browser_context_disposal;
mod command_dispatch;
pub(crate) use command_dispatch::PreparedProtocolCommand;
mod direct;
mod dom;
mod scheduler;
mod target_opening;
pub use direct::{ProtocolOwnerClosed, ProtocolStateHandle, ProtocolStateOwner};

type StateOperation = Box<dyn FnOnce(&mut CdpConnection)>;

/// Finite protocol operations. The connection and its mutable borrow remain
/// private, and operations return owned work for execution outside this scope.
pub struct ProtocolWorkflowState<'a> {
    connection: &'a mut CdpConnection,
}

/// A workflow's revocable state handle and concrete renderer output source.
pub(crate) struct ProtocolWorkflowDriver {
    access: ProtocolStateHandle,
    renderer_output_source: Option<moli_core::RendererOutputCursor>,
}

impl ProtocolWorkflowDriver {
    pub(crate) fn new(access: ProtocolStateHandle) -> Self {
        Self {
            access,
            renderer_output_source: None,
        }
    }

    pub(crate) fn replace_renderer_output_source(
        &mut self,
        source: Option<moli_core::RendererOutputCursor>,
    ) -> Option<moli_core::RendererOutputCursor> {
        std::mem::replace(&mut self.renderer_output_source, source)
    }

    pub(crate) fn state_access(&self) -> ProtocolStateHandle {
        self.access.clone()
    }
    pub(crate) fn with_state<T: 'static>(
        &mut self,
        operation: impl for<'state> FnOnce(ProtocolWorkflowState<'state>) -> T,
    ) -> Result<T> {
        let source = self.renderer_output_source;
        self.access
            .with_state(move |state| state.in_renderer_output_source(source, operation))
    }
}

impl<'a> ProtocolWorkflowState<'a> {
    pub(super) fn new(connection: &'a mut CdpConnection) -> Self {
        Self { connection }
    }
}

impl<'state> ProtocolWorkflowState<'state> {
    fn in_renderer_output_source<T>(
        self,
        source: Option<moli_core::RendererOutputCursor>,
        operation: impl for<'operation> FnOnce(ProtocolWorkflowState<'operation>) -> T,
    ) -> T {
        let Some(source) = source else {
            return operation(self);
        };
        let connection = self.connection;
        let start = connection.scheduler_state.scheduler_event_count();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            operation(ProtocolWorkflowState { connection })
        }));
        connection
            .scheduler_state
            .bind_renderer_output_source_since(start, source);
        result.unwrap_or_else(|payload| std::panic::resume_unwind(payload))
    }
}

impl ProtocolWorkflowState<'_> {
    /// Fixture setup and assertions only. Test workflows use the same driver
    /// as production; this callback cannot return a state borrow or future.
    #[cfg(test)]
    pub(crate) fn finish_fixture_command_output_plan(
        self,
        plan: crate::domains::command_output::CommandOutputPlan,
        context: &mut crate::conn::CommandDispatchContext,
        command_id: Option<u64>,
        session_id: Option<&str>,
    ) -> crate::conn::CdpCommandTaskStep {
        self.connection
            .complete_with_output_plan(context, plan, command_id, session_id)
    }

    #[cfg(test)]
    pub(crate) fn fixture<T: 'static>(
        self,
        operation: impl for<'fixture> FnOnce(&'fixture mut CdpConnection) -> T,
    ) -> T {
        operation(self.connection)
    }

    #[cfg(test)]
    pub(crate) fn finish_command_workflow(
        self,
        completed: super::CompletedCommandWorkflow,
        context: &mut CommandDispatchContext,
    ) -> crate::domains::command_output::CommandOutputPlan {
        completed.finish(self.connection, context)
    }

    pub(crate) fn publish_bidi_channel_output(
        self,
        output: super::BidiChannelOutput,
    ) -> Vec<super::BackgroundProtocolEvent> {
        output.publish(self.connection)
    }

    pub(crate) fn prepare_bidi_channel_proxy(
        self,
        owner: super::BidiChannelPageOwner,
        params: serde_json::Value,
    ) -> Result<Option<super::PreparedRuntimeHelper>> {
        if !owner.can_start_listener(self.connection) {
            return Ok(None);
        }
        // Admission and Native preparation are one finite operation, on the
        // original Page rather than a possibly changed ambient Browser route.
        self.prepare_runtime_helper(
            owner.command_owner().clone(),
            "Runtime.callFunctionOn",
            params,
        )
        .map(Some)
    }

    pub(crate) fn prepare_bidi_preload_channel_listeners(
        self,
        owner: super::CommandOwnerScope,
        execution_context_id: i64,
    ) -> Option<crate::domains::runtime::PreparedBidiPreloadChannelListeners> {
        crate::domains::runtime::PreparedBidiPreloadChannelListeners::prepare(
            self.connection,
            &owner,
            execution_context_id,
        )
    }

    pub(crate) fn allocate_bidi_channel_object_group(
        self,
        owner: super::BidiChannelPageOwner,
    ) -> Option<super::BidiChannelObjectGroupLease> {
        super::BidiChannelObjectGroupLease::allocate(self.connection, owner)
    }

    pub(crate) fn prepare_bidi_channel_listener(
        self,
        lease: super::BidiChannelObjectGroupLease,
        listener: super::PendingBidiChannelListener,
    ) -> Option<super::PreparedBidiChannelAction> {
        super::PreparedBidiChannelAction::prepare_listener(self.connection, lease, listener)
    }

    pub(crate) fn cancel_bidi_channel_dispatch(self, identity: super::BidiChannelDispatchIdentity) {
        identity.cancel(self.connection);
    }

    pub(crate) fn finish_bidi_channel_dispatch(
        self,
        identity: super::BidiChannelDispatchIdentity,
        completed: super::CompletedRuntimeProtocolMessageDispatch,
    ) -> Result<super::BidiChannelDispatchOutput> {
        identity.finish_dispatch(self.connection, completed)
    }

    pub(crate) fn finish_bidi_channel_dispatch_response(
        self,
        identity: super::BidiChannelDispatchIdentity,
        response: moli_core::RendererRuntimeInspectorAsyncCompletion,
    ) -> Result<super::BidiChannelOutput> {
        identity.finish_response(self.connection, response)
    }

    pub(crate) fn prepare_runtime_helper(
        self,
        owner: super::CommandOwnerScope,
        method: &str,
        params: serde_json::Value,
    ) -> Result<super::PreparedRuntimeHelper> {
        super::PreparedRuntimeHelper::prepare(self.connection, owner, method, params)
    }

    pub(crate) fn cancel_runtime_helper(self, identity: super::RuntimeHelperIdentity) {
        identity.cancel(self.connection);
    }

    pub(crate) fn finish_runtime_helper_dispatch(
        self,
        identity: super::RuntimeHelperIdentity,
        command_id: u64,
        object_group: Option<String>,
        completed: super::CompletedRuntimeProtocolMessageDispatch,
    ) -> Result<Option<super::BackgroundCommandResponsePayload>> {
        identity.finish_dispatch(self.connection, command_id, object_group, completed)
    }

    pub(crate) fn finish_runtime_helper_response(
        self,
        identity: super::RuntimeHelperIdentity,
        command_id: u64,
        object_group: Option<String>,
        response: moli_core::RendererRuntimeInspectorAsyncCompletion,
    ) -> Result<super::BackgroundCommandResponsePayload> {
        identity.finish_response(self.connection, command_id, object_group, response)
    }

    pub(crate) fn prepare_runtime_realm_inventory(
        self,
        owner: &super::CommandOwnerScope,
    ) -> Result<super::PreparedRuntimeRealmInventory> {
        super::PreparedRuntimeRealmInventory::prepare(self.connection, owner)
    }

    pub(crate) fn finish_runtime_realm_inventory(
        self,
        prepared: super::PreparedRuntimeRealmInventory,
        completed: moli_core::page::CompletedPageCommand,
    ) -> Result<Vec<crate::automation::RuntimeExecutionContextEvent>> {
        prepared.finish(self.connection, completed)
    }

    pub fn try_start_owned_protocol_scheduler_work(
        self,
        work: crate::domains::activity::ProtocolSchedulerWork,
    ) -> Result<
        Option<super::PendingProtocolSchedulerWork>,
        Box<crate::domains::activity::ProtocolSchedulerWork>,
    > {
        self.connection
            .try_start_owned_protocol_scheduler_work(work)
    }

    pub fn complete_owned_protocol_scheduler_work(
        self,
        completed: super::CompletedProtocolSchedulerWork,
    ) -> super::CdpTurnOutcome {
        self.connection
            .complete_owned_protocol_scheduler_work(completed)
    }

    pub fn complete_failed_command_wait_with_context(
        self,
        command_id: Option<u64>,
        session_id: Option<&str>,
        message: &str,
        context: &mut CommandDispatchContext,
    ) -> CdpCommandTaskStep {
        self.connection
            .complete_failed_command_wait_with_context(command_id, session_id, message, context)
    }

    /// Joins a ready result through finite state or starts its next owned wait.
    pub fn complete_command_dispatch_with_context(
        self,
        completed: CompletedCdpCommandDispatch,
        context: &mut CommandDispatchContext,
    ) -> CdpCommandTaskStep {
        self.connection
            .complete_command_dispatch_with_context(completed, context)
    }

    pub fn start_parsed_command_dispatch_with_context(
        self,
        command: &ParsedCdpCommand,
        context: &mut CommandDispatchContext,
    ) -> CdpCommandTaskStep {
        self.connection
            .start_parsed_command_dispatch_with_context(command, context)
    }

    pub(crate) fn finish_fetch_cleanup_subresource(
        self,
        completed: crate::domains::fetch::CompletedSubresourceFailure,
    ) -> Option<crate::domains::command_output::CommandOutputPlan> {
        completed.finish(self.connection)
    }

    pub(crate) fn start_navigation_renderer_replay(
        self,
        replay: super::state::SessionRendererCallReplay,
        attachment: moli_core::page::RendererAgentAttachmentId,
    ) -> super::runtime_eval::PendingRendererCallReplay {
        self.connection
            .start_renderer_call_replay(replay, attachment)
    }

    pub(crate) fn finish_navigation_renderer_replay(
        self,
        completed: super::runtime_eval::CompletedRendererCallReplay,
    ) -> Vec<super::BackgroundProtocolEvent> {
        self.connection.finish_renderer_call_replay(completed)
    }

    pub(crate) fn prepare_fetch_navigation_error(
        self,
        token: super::DocumentNavigationToken,
        navigation: super::NavigationDispatchState,
        error: anyhow::Error,
    ) -> Result<super::PreparedNavigationLoadError> {
        self.connection
            .prepare_navigation_load_error(Some(&token), &navigation, error)
    }

    pub(crate) fn accept_fetch_navigation_response(
        self,
        pending: super::PendingFetchNavigation,
        response: Result<crate::domains::fetch::FetchNavigationResponse>,
        handle_auth: bool,
        prior_journal: Option<moli_fetch::NetworkObservationJournal>,
    ) -> crate::domains::fetch::PreparedFetchNavigationResponse {
        crate::domains::fetch::accept_navigation_response(
            self.connection,
            pending,
            response,
            handle_auth,
            prior_journal,
        )
    }
}

impl CdpConnection {
    pub(crate) fn protocol_state_access(&self) -> ProtocolStateHandle {
        self.state_access
            .clone()
            .expect("protocol commands require their synchronous state owner")
    }
}

mod activation;
mod child_navigation;
mod input;
mod navigation_commit;
mod navigation_completion;
mod session_disposal;
pub use navigation_commit::BackgroundPreparedNavigationCommit;
pub(crate) use navigation_commit::PreparedDocumentCommitState;

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use futures_util::FutureExt as _;

    use super::*;
    use crate::conn::CdpSchedulerEvent;
    use crate::domains::activity::ProtocolSchedulerWork;

    fn publish_event(connection: &mut CdpConnection) {
        let sequence = connection
            .scheduler_state
            .allocate_protocol_work_publish_sequence();
        let work =
            ProtocolSchedulerWork::renderer_fenced_output(sequence, Vec::new(), Vec::new(), None);
        connection
            .scheduler_state
            .push_scheduler_event(CdpSchedulerEvent::ProtocolWorkPublished { work });
    }

    #[tokio::test]
    async fn synchronous_operations_wake_the_actor_before_each_external_wait_finishes() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let owner = ProtocolStateOwner::new(CdpConnection::new());
                let access = owner.handle();
                let events_ready = owner.scheduler_events_ready();
                let (next, next_wait) = tokio::sync::oneshot::channel();
                let (_finish, finish_wait) = tokio::sync::oneshot::channel::<()>();
                let task = tokio::task::spawn_local(async move {
                    access
                        .with_state(|state| publish_event(state.connection))
                        .unwrap();
                    next_wait.await.unwrap();
                    access
                        .with_state(|state| publish_event(state.connection))
                        .unwrap();
                    finish_wait.await.unwrap();
                });
                tokio::time::timeout(std::time::Duration::from_secs(5), events_ready.notified())
                    .await
                    .unwrap();
                assert!(
                    !task.is_finished(),
                    "the actor must wake during the first external wait"
                );
                let first = owner
                    .with_state(|state| state.take_scheduler_events())
                    .unwrap();
                assert_eq!(first.len(), 1);
                next.send(()).unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(5), events_ready.notified())
                    .await
                    .unwrap();
                assert!(
                    !task.is_finished(),
                    "the second operation must wake without task completion"
                );
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                let second = owner
                    .with_state(|state| state.take_scheduler_events())
                    .unwrap();
                assert_eq!(
                    second.len(),
                    1,
                    "cancellation cannot discard an accepted scheduler event"
                );
                assert!(
                    owner
                        .with_state(|state| state.take_scheduler_events())
                        .unwrap()
                        .is_empty()
                );
            })
            .await;
    }

    #[test]
    fn workflow_access_and_event_notifications_cannot_extend_the_owner_lifetime() {
        let owner = ProtocolStateOwner::new(CdpConnection::new());
        let access = owner
            .with_state(|state| state.connection.protocol_state_access())
            .unwrap();
        let _events_ready = owner.scheduler_events_ready();
        drop(owner);
        assert!(
            access
                .with_state(|_| ())
                .unwrap_err()
                .is::<ProtocolOwnerClosed>()
        );
    }

    #[test]
    fn synchronous_handles_keep_each_operation_on_its_original_connection() {
        let first = ProtocolStateOwner::new(CdpConnection::new());
        let second = ProtocolStateOwner::new(CdpConnection::new());
        let access = first.handle();
        assert!(
            !access
                .with_state(|state| state.replace_root_target_discovery_enabled(true))
                .unwrap()
        );
        assert!(
            !second
                .with_state(|state| state.replace_root_target_discovery_enabled(false))
                .unwrap()
        );
        assert!(
            access
                .with_state(|state| state.replace_root_target_discovery_enabled(false))
                .unwrap()
        );
    }

    #[test]
    fn deferred_cleanup_events_wake_the_actor_even_when_the_state_operation_unwinds() {
        let owner = ProtocolStateOwner::new(CdpConnection::new());
        let access = owner.handle();
        let events_ready = owner.scheduler_events_ready();
        let result = catch_unwind(AssertUnwindSafe(|| {
            access
                .with_state(|_| {
                    access.defer_cleanup(Box::new(publish_event));
                    panic!("controlled state operation failure");
                })
                .unwrap();
        }));
        assert!(result.is_err());
        assert!(events_ready.notified().now_or_never().is_some());
        assert_eq!(
            owner
                .with_state(|state| state.take_scheduler_events())
                .unwrap()
                .len(),
            1
        );
    }
}

mod initial_document;
mod output_projection;
mod page_command;
mod page_runtime;
mod popup;
mod runtime_query;
mod subresource;
mod worker_projection;
mod worker_runtime;

mod renderer_navigation;

#[cfg(test)]
mod network_data_tests;
