use anyhow::{Context as _, Result};

use super::*;
use crate::conn::{ProtocolStateAccess, ProtocolWorkflowDriver, WorkerRuntimeScope};

pub(crate) struct PreparedWorkerBindingReplay {
    scope: WorkerRuntimeScope,
    bindings: Vec<RuntimeBindingDefinition>,
}

/// Owns one admitted replay and its original renderer call. The pending set
/// represents work not yet admitted: a later definition can enqueue itself
/// while this call waits, and this call's success cannot consume that work.
pub(crate) struct WorkerBindingReplayCall {
    scope: WorkerRuntimeScope,
    binding: RuntimeBindingDefinition,
    command_id: u64,
    correlation: crate::conn::RendererCommandCorrelation,
    pending: Option<PendingSharedWorkerRuntimeProtocolMessageDispatch>,
    access: Option<ProtocolStateAccess>,
}

impl PreparedWorkerBindingReplay {
    pub(crate) fn prepare(conn: &CdpConnection, scope: WorkerRuntimeScope) -> Option<Self> {
        if !scope.can_configure(conn) {
            return None;
        }
        let bindings = conn
            .shared_worker_target_for_session(Some(scope.session_id()))?
            .runtime_bindings_requiring_replay(scope.session_id());
        (!bindings.is_empty()).then_some(Self { scope, bindings })
    }

    pub(crate) async fn run(self, driver: &mut ProtocolWorkflowDriver<'_>) {
        for binding in self.bindings {
            let scope = self.scope.clone();
            let name = binding.name.clone();
            let pending = driver
                .with_state(move |state| state.prepare_worker_binding_replay_call(scope, binding))
                .await;
            match pending {
                Ok(Ok(Some(call))) => {
                    if let Err(error) = call.run(driver).await {
                        tracing::warn!(binding = %name, %error, "failed to replay Worker Runtime binding");
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    tracing::warn!(binding = %name, %error, "failed to admit Worker Runtime binding replay");
                }
                Err(error) => {
                    tracing::debug!(%error, "Worker binding replay owner closed");
                    return;
                }
            }
        }
    }
}

impl WorkerBindingReplayCall {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        scope: WorkerRuntimeScope,
        binding: RuntimeBindingDefinition,
    ) -> Result<Option<Self>> {
        if !scope.can_configure(conn)
            || !conn
                .shared_worker_target_for_session(Some(scope.session_id()))
                .is_some_and(|target| {
                    target
                        .runtime_bindings_requiring_replay(scope.session_id())
                        .contains(&binding)
                })
        {
            return Ok(None);
        }
        let command_id = conn.next_internal_runtime_command_id();
        let descriptor = RendererCommandDescriptor::for_document_helper(
            shared_worker_runtime_binding_replay_json(command_id, &binding),
        )?;
        let pending = conn
            .start_shared_worker_runtime_protocol_message_for_session_with_deferred_response(
                Some(scope.session_id()),
                descriptor,
                command_id,
            )?;
        let correlation = conn
            .shared_worker_target_for_session(Some(scope.session_id()))
            .and_then(|target| target.renderer_call_for_frontend(scope.session_id(), command_id))
            .expect("finite replay admission registered its original renderer call");
        conn.shared_worker_target_for_session_mut(Some(scope.session_id()))
            .expect("admitted replay retains its worker")
            .take_runtime_binding_replay(scope.session_id(), &binding);
        Ok(Some(Self {
            scope,
            binding,
            command_id,
            correlation,
            pending: Some(pending),
            access: conn.protocol_state_access(),
        }))
    }

    async fn run(mut self, driver: &mut ProtocolWorkflowDriver<'_>) -> Result<()> {
        let result = self.run_inner(driver).await;
        let success = matches!(&result, Ok(true));
        driver
            .with_state(move |state| state.finish_worker_binding_replay_call(self, success))
            .await?;
        match result {
            Ok(true) => Ok(()),
            Ok(false) => Err(WorkerBindingReplayRejected.into()),
            Err(error) => Err(error),
        }
    }

    async fn run_inner(&mut self, driver: &mut ProtocolWorkflowDriver<'_>) -> Result<bool> {
        let mut completed = self
            .pending
            .take()
            .expect("one replay dispatch")
            .wait()
            .await?;
        let receiver = completed.take_deferred_response_receiver();
        let scope = self.scope.clone();
        let command_id = self.command_id;
        let mut success = driver
            .with_state(move |state| {
                state.finish_worker_binding_dispatch(scope, command_id, completed)
            })
            .await??;
        if let Some(receiver) = receiver {
            let response = receiver
                .await
                .context(moli_core::page::WorkerInspectorError::ResponseCanceled)?;
            let scope = self.scope.clone();
            success |= driver
                .with_state(move |state| {
                    state.finish_worker_binding_response(scope, command_id, response)
                })
                .await?;
        }
        Ok(success)
    }

    fn cancel(&self, conn: &mut CdpConnection, retry: bool) {
        if !self.scope.binding_is_current(conn) {
            return;
        }
        conn.take_renderer_call_if_correlation_matches_for_session_owner(
            Some(self.scope.session_id()),
            self.correlation,
        );
        if retry
            && self.scope.can_configure(conn)
            && let Some(target) =
                conn.shared_worker_target_for_session_mut(Some(self.scope.session_id()))
        {
            target.retry_runtime_binding_replay(self.scope.session_id(), &self.binding);
        }
    }

    pub(crate) fn finish(mut self, conn: &mut CdpConnection, success: bool) {
        self.cancel(conn, !success);
        self.access.take();
    }
}

impl Drop for WorkerBindingReplayCall {
    fn drop(&mut self) {
        if let Some(access) = self.access.take() {
            let scope = self.scope.clone();
            let binding = self.binding.clone();
            let correlation = self.correlation;
            access.defer_worker_binding_replay_cleanup(scope, binding, correlation);
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Worker Runtime binding replay did not receive a successful Inspector response")]
struct WorkerBindingReplayRejected;

pub(crate) fn finish_binding_dispatch(
    conn: &mut CdpConnection,
    scope: WorkerRuntimeScope,
    command_id: u64,
    completed: CompletedSharedWorkerRuntimeProtocolMessageDispatch,
) -> Result<bool> {
    if !scope.binding_is_current(conn) {
        return Ok(false);
    }
    let messages = conn.complete_shared_worker_runtime_protocol_message_for_session(completed)?;
    Ok(command_response_succeeded(&messages, Some(command_id)))
}

pub(crate) fn finish_binding_response(
    conn: &mut CdpConnection,
    scope: WorkerRuntimeScope,
    command_id: u64,
    response: moli_core::RendererRuntimeInspectorAsyncCompletion,
) -> bool {
    if !scope.binding_is_current(conn) {
        return false;
    }
    conn.resolve_runtime_inspector_response_ready(RuntimeInspectorResponseReady::new(
        command_id,
        Some(scope.session_id()),
        Ok(response),
    ))
    .is_some_and(|response| {
        matches!(
            response.into_command_response_payload(),
            BackgroundCommandResponsePayload::Success { .. }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const SESSION: &str = "SID-owned-worker";

    fn add_binding(conn: &mut CdpConnection) -> RuntimeBindingDefinition {
        let target = conn
            .shared_worker_target_for_session_mut(Some(SESSION))
            .unwrap();
        target.upsert_runtime_binding_definition(SESSION, "ownedBinding".into(), None);
        target
            .runtime_bindings_requiring_replay(SESSION)
            .into_iter()
            .next()
            .unwrap()
    }

    #[test]
    fn successful_replay_cannot_consume_a_definition_queued_while_it_waited() {
        let mut conn = worker_workflow_fixture();
        let binding = add_binding(&mut conn);
        let scope = WorkerRuntimeScope::capture(&conn, Some(SESSION)).unwrap();
        let call = WorkerBindingReplayCall::prepare(&mut conn, scope, binding.clone())
            .unwrap()
            .unwrap();
        assert!(
            conn.shared_worker_target_for_session(Some(SESSION))
                .unwrap()
                .runtime_bindings_requiring_replay(SESSION)
                .is_empty()
        );
        add_binding(&mut conn);
        call.finish(&mut conn, true);
        assert_eq!(
            conn.shared_worker_target_for_session(Some(SESSION))
                .unwrap()
                .runtime_bindings_requiring_replay(SESSION),
            vec![binding]
        );
    }

    #[test]
    fn replay_cleanup_cannot_retire_a_successor_with_the_same_native_scope_and_session_id() {
        let mut conn = worker_workflow_fixture();
        let binding = add_binding(&mut conn);
        let scope = WorkerRuntimeScope::capture(&conn, Some(SESSION)).unwrap();
        let original = WorkerBindingReplayCall::prepare(&mut conn, scope.clone(), binding.clone())
            .unwrap()
            .unwrap();
        let route = conn.session_route(Some(SESSION)).unwrap();
        conn.register_session_route_for_test(SESSION, route);
        assert!(!scope.binding_is_current(&conn));
        add_binding(&mut conn);
        let successor_scope = WorkerRuntimeScope::capture(&conn, Some(SESSION)).unwrap();
        let successor = WorkerBindingReplayCall::prepare(&mut conn, successor_scope, binding)
            .unwrap()
            .unwrap();
        let correlation = successor.correlation;
        original.finish(&mut conn, false);
        assert_eq!(
            conn.shared_worker_target_for_session(Some(SESSION))
                .unwrap()
                .renderer_call_for_frontend(SESSION, successor.command_id),
            Some(correlation)
        );
        assert!(
            conn.shared_worker_target_for_session(Some(SESSION))
                .unwrap()
                .runtime_bindings_requiring_replay(SESSION)
                .is_empty()
        );
        successor.finish(&mut conn, false);
        assert_eq!(
            conn.shared_worker_target_for_session(Some(SESSION))
                .unwrap()
                .runtime_bindings_requiring_replay(SESSION)
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn failed_native_replay_requeues_the_definition_through_direct_with_state() {
        let mut conn = worker_workflow_fixture();
        let binding = add_binding(&mut conn);
        let scope = WorkerRuntimeScope::capture(&conn, Some(SESSION)).unwrap();
        let retry_scope = scope.clone();
        let retry_binding = binding.clone();
        let owner = crate::conn::ProtocolStateOwner::new(conn);
        let handle = owner.handle();
        let replay = handle
            .with_state(|state| state.prepare_worker_binding_replay_call(scope, binding))
            .unwrap()
            .unwrap()
            .unwrap();
        let error = replay
            .run(&mut ProtocolWorkflowDriver::owned(
                ProtocolStateAccess::Direct(handle.clone()),
            ))
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<moli_core::page::WorkerInspectorError>(),
            Some(moli_core::page::WorkerInspectorError::SharedUnavailable)
        ));
        let repeat = handle
            .with_state(|state| {
                state.prepare_worker_binding_replay_call(retry_scope, retry_binding)
            })
            .unwrap()
            .unwrap();
        assert!(
            repeat.is_some(),
            "failed native replay returns its original definition to the pending set"
        );
    }
}
