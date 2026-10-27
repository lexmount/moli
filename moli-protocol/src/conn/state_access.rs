//! Finite protocol-state access. `direct` provides synchronous access through
//! weak handles. The actor mailbox below is a migration adapter for legacy
//! entry points which still borrow the connection across awaits; remove it
//! when those entry points have migrated.

use anyhow::Result;

use anyhow::Context as _;
use tokio::sync::{mpsc, oneshot};

use super::{
    CdpCommandTaskStep, CdpConnection, CommandDispatchContext, CompletedCdpCommandDispatch,
    ParsedCdpCommand,
};

mod direct;
pub use direct::{ProtocolStateHandle, ProtocolStateOwner};

type StateOperation = Box<dyn FnOnce(&mut CdpConnection)>;

pub struct ProtocolStateRequest(StateOperation);

impl ProtocolStateRequest {
    pub fn apply(self, connection: &mut CdpConnection) {
        (self.0)(connection);
    }
}

pub type ProtocolStateRequestReceiver = mpsc::UnboundedReceiver<ProtocolStateRequest>;

/// Revocable access to finite protocol operations. It neither owns a
/// connection nor lends a connection, reference, guard or future to callers.
#[derive(Clone)]
pub(crate) enum ProtocolStateAccess {
    Direct(ProtocolStateHandle),
    Mailbox(mpsc::WeakUnboundedSender<ProtocolStateRequest>),
}

/// Finite protocol operations. The connection and its mutable borrow remain
/// private, and operations return owned work for execution outside this scope.
pub struct ProtocolWorkflowState<'a> {
    connection: &'a mut CdpConnection,
}

/// The inline driver is only for entry points which have not yet handed their
/// task to the actor. Owned workflows use their owner's revocable access:
/// synchronous for ProtocolStateOwner, or the legacy actor mailbox.
/// Remove the inline arm and mailbox at the actor's direct-sync cutover.
pub(crate) struct ProtocolWorkflowDriver<'a> {
    access: ProtocolWorkflowAccess<'a>,
}

enum ProtocolWorkflowAccess<'a> {
    Inline(&'a mut CdpConnection),
    Owned(ProtocolStateAccess),
}

impl<'a> ProtocolWorkflowDriver<'a> {
    pub(crate) fn inline(connection: &'a mut CdpConnection) -> Self {
        Self {
            access: ProtocolWorkflowAccess::Inline(connection),
        }
    }

    pub(crate) fn owned(access: ProtocolStateAccess) -> ProtocolWorkflowDriver<'static> {
        ProtocolWorkflowDriver {
            access: ProtocolWorkflowAccess::Owned(access),
        }
    }

    pub(crate) fn owned_access(&self) -> Option<ProtocolStateAccess> {
        match &self.access {
            ProtocolWorkflowAccess::Inline(_) => None,
            ProtocolWorkflowAccess::Owned(access) => Some(access.clone()),
        }
    }
    pub(crate) async fn with_state<T: 'static>(
        &mut self,
        operation: impl for<'state> FnOnce(ProtocolWorkflowState<'state>) -> T + 'static,
    ) -> Result<T> {
        match &mut self.access {
            ProtocolWorkflowAccess::Inline(connection) => {
                Ok(operation(ProtocolWorkflowState { connection }))
            }
            ProtocolWorkflowAccess::Owned(access) => access.with_state(operation).await,
        }
    }
}

impl ProtocolWorkflowState<'_> {
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

    pub(crate) fn prepare_bidi_channel_action(
        self,
        action: super::BidiChannelOwnerAction,
    ) -> Option<super::PreparedBidiChannelAction> {
        super::PreparedBidiChannelAction::prepare(self.connection, action)
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

    /// Finishes a migrated command without borrowing state across a wait.
    /// Legacy completions are returned intact for their existing driver.
    pub fn try_complete_ready_command_dispatch_with_context(
        self,
        completed: CompletedCdpCommandDispatch,
        context: &mut CommandDispatchContext,
    ) -> Result<CdpCommandTaskStep, CompletedCdpCommandDispatch> {
        self.connection
            .try_complete_ready_command_dispatch_with_context(completed, context)
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

pub(super) struct ProtocolStateMailbox {
    direct: Option<ProtocolStateHandle>,
    sender: mpsc::UnboundedSender<ProtocolStateRequest>,
    receiver: Option<ProtocolStateRequestReceiver>,
}

impl Default for ProtocolStateMailbox {
    fn default() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            direct: None,
            sender,
            receiver: Some(receiver),
        }
    }
}

impl ProtocolStateAccess {
    pub(super) fn defer_cleanup(&self, operation: StateOperation) {
        match self {
            Self::Direct(handle) => handle.defer_cleanup(operation),
            Self::Mailbox(sender) => {
                if let Some(sender) = sender.upgrade() {
                    let _ = sender.send(ProtocolStateRequest(operation));
                }
            }
        }
    }

    pub(crate) async fn with_state<T: 'static>(
        &self,
        operation: impl for<'a> FnOnce(ProtocolWorkflowState<'a>) -> T + 'static,
    ) -> Result<T> {
        let sender = match self {
            Self::Direct(handle) => return handle.with_state(operation),
            Self::Mailbox(sender) => sender.upgrade().context("protocol owner closed")?,
        };
        let (reply, receive) = oneshot::channel();
        let owner = sender.downgrade();
        sender
            .send(ProtocolStateRequest(Box::new(move |connection| {
                // An aborted workflow must not register work merely because
                // its already-queued request is drained later.
                if !reply.is_closed()
                    && owner
                        .upgrade()
                        .is_some_and(|owner| owner.same_channel(&connection.state_mailbox.sender))
                {
                    let result = operation(ProtocolWorkflowState { connection });
                    let _ = reply.send(result);
                }
            })))
            .map_err(|_| anyhow::anyhow!("protocol owner closed"))?;
        drop(sender);
        receive.await.context("protocol owner closed")
    }
}

impl CdpConnection {
    pub fn take_protocol_state_requests(&mut self) -> Option<ProtocolStateRequestReceiver> {
        self.state_mailbox.receiver.take()
    }

    pub(crate) fn protocol_state_access(&self) -> Option<ProtocolStateAccess> {
        if let Some(handle) = &self.state_mailbox.direct {
            return Some(ProtocolStateAccess::Direct(handle.clone()));
        }
        // The inline compatibility driver must retain its old behaviour until
        // its caller takes responsibility for pumping state requests.
        self.state_mailbox
            .receiver
            .is_none()
            .then(|| ProtocolStateAccess::Mailbox(self.state_mailbox.sender.downgrade()))
    }
}

mod activation;
mod child_navigation;
mod input;
mod navigation_commit;
mod session_disposal;
pub use navigation_commit::BackgroundPreparedNavigationCommit;
pub(crate) use navigation_commit::PreparedDocumentCommitState;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn queued_state_operation_is_skipped_after_its_workflow_is_cancelled() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let mut conn = CdpConnection::new();
                let mut requests = conn.take_protocol_state_requests().unwrap();
                let access = conn.protocol_state_access().unwrap();
                let ran = std::rc::Rc::new(std::cell::Cell::new(false));
                let operation_ran = ran.clone();
                let task = tokio::task::spawn_local(async move {
                    access.with_state(move |_| operation_ran.set(true)).await
                });
                let request = requests.recv().await.unwrap();
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                request.apply(&mut conn);
                assert!(!ran.get());
            })
            .await;
    }

    #[tokio::test]
    async fn workflow_access_does_not_keep_the_protocol_owner_alive() {
        let mut conn = CdpConnection::new();
        let _requests = conn.take_protocol_state_requests().unwrap();
        let access = conn.protocol_state_access().unwrap();
        drop(conn);
        assert_eq!(
            access.with_state(|_| ()).await.unwrap_err().to_string(),
            "protocol owner closed"
        );
    }

    #[tokio::test]
    async fn state_request_cannot_be_applied_to_another_connection() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let mut first = CdpConnection::new();
                let mut requests = first.take_protocol_state_requests().unwrap();
                let access = first.protocol_state_access().unwrap();
                let ran = std::rc::Rc::new(std::cell::Cell::new(false));
                let operation_ran = ran.clone();
                let task = tokio::task::spawn_local(async move {
                    access.with_state(move |_| operation_ran.set(true)).await
                });
                let request = requests.recv().await.unwrap();
                request.apply(&mut CdpConnection::new());
                assert!(task.await.unwrap().is_err());
                assert!(!ran.get());
            })
            .await;
    }

    #[tokio::test]
    async fn dropping_state_request_receiver_settles_a_waiting_workflow() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let mut conn = CdpConnection::new();
                let mut requests = conn.take_protocol_state_requests().unwrap();
                let access = conn.protocol_state_access().unwrap();
                let task = tokio::task::spawn_local(async move { access.with_state(|_| 42).await });
                let request = requests.recv().await.unwrap();
                drop(request);
                drop(requests);
                assert_eq!(
                    task.await.unwrap().unwrap_err().to_string(),
                    "protocol owner closed"
                );
            })
            .await;
    }
}
