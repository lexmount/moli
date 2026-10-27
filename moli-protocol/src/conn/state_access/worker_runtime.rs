use anyhow::Result;

use super::{ProtocolStateAccess, ProtocolWorkflowState};
use crate::conn::{
    CompletedSharedWorkerRuntimeProtocolMessageDispatch, RuntimeBindingDefinition,
    WorkerRuntimeScope,
};
use crate::domains::runtime::{
    PreparedWorkerRuntimeCommand, WorkerBindingReplayCall, WorkerRuntimeContinuation,
};

impl ProtocolStateAccess {
    pub(crate) fn defer_worker_binding_replay_cleanup(
        &self,
        scope: WorkerRuntimeScope,
        binding: RuntimeBindingDefinition,
        correlation: crate::conn::RendererCommandCorrelation,
    ) {
        self.defer_cleanup(Box::new(move |conn| {
            if !scope.binding_is_current(conn) {
                return;
            }
            conn.take_renderer_call_if_correlation_matches_for_session_owner(
                Some(scope.session_id()),
                correlation,
            );
            if scope.can_configure(conn)
                && let Some(target) =
                    conn.shared_worker_target_for_session_mut(Some(scope.session_id()))
            {
                target.retry_runtime_binding_replay(scope.session_id(), &binding);
            }
        }));
    }

    pub(crate) fn defer_worker_runtime_claim_cleanup(
        &self,
        scope: WorkerRuntimeScope,
        claimed: crate::conn::ClaimedPendingInspectorAwait,
    ) {
        self.defer_cleanup(Box::new(move |conn| {
            if scope.binding_is_current(conn) {
                conn.cancel_claimed_pending_inspector_await_for_scheduler_deferred_reply(
                    Some(claimed),
                    "Worker command owner closed",
                );
            }
        }));
    }
}

impl ProtocolWorkflowState<'_> {
    pub(crate) fn finish_worker_runtime_dispatch(
        self,
        prepared: PreparedWorkerRuntimeCommand,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Option<WorkerRuntimeContinuation> {
        prepared.finish_dispatch(self.connection, context)
    }

    pub(crate) fn finish_worker_runtime_response(
        self,
        continuation: WorkerRuntimeContinuation,
        response: Result<moli_core::RendererRuntimeInspectorAsyncCompletion>,
        context: &mut crate::conn::CommandWorkflowContext,
    ) -> Option<WorkerRuntimeContinuation> {
        continuation.finish_response(self.connection, response, context)
    }

    pub(crate) fn finish_worker_runtime_tail(
        self,
        continuation: WorkerRuntimeContinuation,
        context: &mut crate::conn::CommandWorkflowContext,
    ) {
        continuation.finish_tail(self.connection, context);
    }
    pub(crate) fn prepare_worker_binding_replay_call(
        self,
        scope: WorkerRuntimeScope,
        binding: RuntimeBindingDefinition,
    ) -> Result<Option<WorkerBindingReplayCall>> {
        WorkerBindingReplayCall::prepare(self.connection, scope, binding)
    }

    pub(crate) fn finish_worker_binding_dispatch(
        self,
        scope: WorkerRuntimeScope,
        command_id: u64,
        completed: CompletedSharedWorkerRuntimeProtocolMessageDispatch,
    ) -> Result<bool> {
        crate::domains::runtime::finish_binding_dispatch(
            self.connection,
            scope,
            command_id,
            completed,
        )
    }

    pub(crate) fn finish_worker_binding_response(
        self,
        scope: WorkerRuntimeScope,
        command_id: u64,
        response: moli_core::RendererRuntimeInspectorAsyncCompletion,
    ) -> bool {
        crate::domains::runtime::finish_binding_response(
            self.connection,
            scope,
            command_id,
            response,
        )
    }

    pub(crate) fn finish_worker_binding_replay_call(
        self,
        call: WorkerBindingReplayCall,
        success: bool,
    ) {
        call.finish(self.connection, success);
    }
}
