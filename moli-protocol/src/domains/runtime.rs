mod activity;
mod bidi_nodes;
mod bindings;
mod command_classification;
mod dispatcher;
mod evaluate;
#[cfg(test)]
mod test_support;

use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, SessionDisposalPlan, SessionDisposalTarget,
};

/// Settles protocol work that still belongs to a session before another
/// domain starts tearing down renderer-owned resources.
pub(in crate::domains) fn fail_pending_session_calls(
    conn: &mut CdpConnection,
    background_events: &mut Vec<BackgroundProtocolEvent>,
    protocol_events: &mut Vec<BackgroundProtocolEvent>,
    session_id: &str,
) {
    conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
        background_events,
        protocol_events,
        Some(session_id),
        "Target detached",
    );
}

/// Disables Runtime service state owned by one DevTools session. Page
/// Inspector resources retire separately; worker handlers release remote
/// objects before their worker Inspector is detached.
pub(in crate::domains) async fn dispose_session_handler_async(
    conn: &mut CdpConnection,
    background_events: &mut Vec<BackgroundProtocolEvent>,
    protocol_events: &mut Vec<BackgroundProtocolEvent>,
    plan: &SessionDisposalPlan,
) -> anyhow::Result<()> {
    let session_id = plan.session_id();
    match plan.target() {
        SessionDisposalTarget::PageTarget { .. }
        | SessionDisposalTarget::SharedWorkerTarget { .. }
        | SessionDisposalTarget::DedicatedWorkerTarget { .. }
        | SessionDisposalTarget::ServiceWorkerTarget { .. } => {
            if !matches!(plan.target(), SessionDisposalTarget::PageTarget { .. }) {
                conn.release_worker_runtime_remote_objects_for_session_best_effort_async(
                    session_id,
                )
                .await;
            }
            fail_pending_session_calls(conn, background_events, protocol_events, session_id);
        }
        SessionDisposalTarget::Browser | SessionDisposalTarget::TabTarget { .. } => {}
    }
    Ok(())
}

/// Resolve from the disposal plan, including an uncommitted attach that has no
/// session route yet. The caller keeps this exact endpoint across async cleanup.
pub(in crate::domains) fn worker_inspection_endpoint_for_disposal(
    conn: &CdpConnection,
    plan: &SessionDisposalPlan,
) -> Option<moli_core::runtime::RendererWorkerInspectionEndpoint> {
    use moli_core::runtime::RendererWorkerIdentity;
    let context = conn.browser_context_by_id(plan.browser_context_id()?)?;
    let worker = match plan.target() {
        SessionDisposalTarget::SharedWorkerTarget { target_id, .. } => {
            RendererWorkerIdentity::Shared(
                context
                    .shared_worker_target(target_id)?
                    .renderer_instance_id,
            )
        }
        SessionDisposalTarget::DedicatedWorkerTarget { target_id, .. } => {
            RendererWorkerIdentity::Dedicated(
                context
                    .dedicated_worker_target(target_id)?
                    .renderer_instance_id,
            )
        }
        SessionDisposalTarget::ServiceWorkerTarget { target_id, .. } => context
            .service_worker_target(target_id)?
            .inspection_target()?,
        _ => return None,
    };
    context.worker_inspection_endpoint(worker)
}

pub(in crate::domains) use activity::{
    RuntimePreparedOutputSlot, RuntimePreparedOutputs, project_runtime_binding_calls_async,
    project_runtime_inspector_messages_async,
    project_runtime_inspector_post_response_messages_async,
    push_routed_renderer_runtime_inspector_message_batch_background_events,
};
pub(in crate::domains) use dispatcher::replay_shared_worker_runtime_bindings_for_session_async;
pub(crate) use dispatcher::{
    BidiPreloadFunctionDeclaration, CompletedRuntimeCommandDispatch, PendingRuntimeCommandDispatch,
    RuntimeCommandTaskStep, bidi_preload_function_declaration_source,
    command_waits_for_document_projection, complete_pending_runtime_command_at_response_boundary,
    debugger_command_waits_for_document_projection, devtools_deep_serialization_options_json,
    execute_devtools_runtime_command_async_with_protocol_events,
    execute_runtime_listener_command_for_owner,
    start_bidi_preload_channel_listeners_for_execution_context_background_events_async,
    start_console_inspector_command_dispatch, start_debugger_inspector_command_dispatch,
    start_heap_profiler_inspector_command_dispatch, start_moli_diagnostics_command_dispatch,
    start_profiler_inspector_command_dispatch, try_start_runtime_command_dispatch,
};
pub use dispatcher::{
    CompletedDevToolsRuntimeCommandDispatch, DevToolsRuntimeCommandTaskStep,
    PendingDevToolsRuntimeCommandDispatch,
};

#[cfg(test)]
mod tests;
