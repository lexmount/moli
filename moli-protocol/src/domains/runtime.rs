use anyhow::Result;

mod activity;
mod bidi_nodes;
mod bindings;
mod command_classification;
mod dispatcher;
pub(crate) use dispatcher::start_protocol_neutral_runtime_inspector_command;
pub(crate) use dispatcher::{
    PageRuntimeCommandContinuation, PageRuntimeCompletion, PreparedWorkerRuntimeCommand,
    WorkerRuntimeContinuation, prepare_page_runtime_command_completion,
};
pub(crate) use dispatcher::{
    WorkerBindingReplayCall, finish_binding_dispatch, finish_binding_response,
};
pub(crate) use dispatcher::{
    register_devtools_script_result_remote_object,
    register_devtools_script_result_remote_object_realm,
    validate_protocol_neutral_runtime_handle_realms,
};
mod evaluate;
mod native;
pub(in crate::domains) use native::{
    project_binding_terminal, project_subscription_terminal, try_start_native_command,
};
#[cfg(test)]
mod test_support;

use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, SessionDisposalPlan, SessionDisposalTarget,
};

/// Settles protocol work that still belongs to a session before another
/// domain starts tearing down renderer-owned resources.
pub(crate) fn fail_pending_session_calls(
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

/// Detaches the Page's renderer Inspector endpoint after Page-owned cleanup
/// commands have completed.
pub(in crate::domains) fn detach_page_session_inspector(
    conn: &mut CdpConnection,
    session_id: &str,
) {
    if let Err(error) = conn.detach_runtime_inspector_session_for_session_owner(Some(session_id)) {
        tracing::debug!(
            session_id,
            %error,
            "renderer Inspector session was already unavailable during disposal"
        );
    }
}

/// Detaches the renderer-side Inspector endpoint after every browser-side
/// domain handler has disabled itself.
pub(crate) fn detach_session_inspector(
    conn: &mut CdpConnection,
    plan: &SessionDisposalPlan,
) -> Result<()> {
    let session_id = plan.session_id();
    match plan.target() {
        SessionDisposalTarget::PageTarget { .. } => {
            detach_page_session_inspector(conn, session_id);
        }
        SessionDisposalTarget::SharedWorkerTarget {
            browser_context_id,
            target_id,
        } => {
            let renderer_detach =
                conn.browser_context_by_id(browser_context_id)
                    .and_then(|browser_context| {
                        browser_context
                            .shared_worker_target(target_id)
                            .map(|target| {
                                (
                                    browser_context.renderer_runtime(),
                                    target.renderer_instance_id,
                                )
                            })
                    });
            if let Some((renderer_runtime, instance_id)) = renderer_detach {
                renderer_runtime.detach_shared_worker_runtime_inspector_session(
                    instance_id,
                    Some(session_id.to_owned()),
                );
            }
        }
        SessionDisposalTarget::DedicatedWorkerTarget {
            browser_context_id,
            target_id,
        } => {
            let renderer_detach =
                conn.browser_context_by_id(browser_context_id)
                    .and_then(|browser_context| {
                        browser_context
                            .dedicated_worker_target(target_id)
                            .map(|target| {
                                (
                                    browser_context.renderer_runtime(),
                                    target.renderer_instance_id,
                                )
                            })
                    });
            if let Some((renderer_runtime, instance_id)) = renderer_detach {
                renderer_runtime.detach_dedicated_worker_runtime_inspector_session(
                    instance_id,
                    Some(session_id.to_owned()),
                );
            }
        }
        SessionDisposalTarget::ServiceWorkerTarget {
            browser_context_id,
            target_id,
        } => {
            let renderer_detach =
                conn.browser_context_by_id(browser_context_id)
                    .and_then(|browser_context| {
                        browser_context
                            .service_worker_target(target_id)
                            .map(|target| {
                                (
                                    browser_context.renderer_runtime(),
                                    target.renderer_version_id,
                                )
                            })
                    });
            if let Some((renderer_runtime, version_id)) = renderer_detach {
                renderer_runtime.detach_service_worker_runtime_inspector_session(
                    version_id,
                    Some(session_id.to_owned()),
                );
            }
        }
        SessionDisposalTarget::Browser | SessionDisposalTarget::TabTarget { .. } => {}
    }
    Ok(())
}

pub(crate) use activity::{
    PreparedRuntimeInspectorProjection,
    push_routed_renderer_runtime_inspector_message_batch_background_events,
};
pub(in crate::domains) use activity::{
    RuntimePreparedOutputSlot, RuntimePreparedOutputs, project_runtime_binding_calls,
};
pub(crate) use dispatcher::{
    BidiPreloadFunctionDeclaration, CompletedRuntimeCommandDispatch, PendingRuntimeCommandDispatch,
    PreparedBidiPreloadChannelListeners, RuntimeCommandTaskStep,
    bidi_preload_function_declaration_source,
    complete_pending_runtime_command_at_response_boundary,
    devtools_deep_serialization_options_json,
    execute_devtools_runtime_command_async_with_protocol_events,
    start_bidi_preload_channel_listeners_for_execution_context_background_events_async,
    start_console_inspector_command_dispatch, start_debugger_inspector_command_dispatch,
    start_heap_profiler_inspector_command_dispatch, start_moli_diagnostics_command_dispatch,
    start_profiler_inspector_command_dispatch, start_runtime_listener_command_for_owner,
    try_start_runtime_command_dispatch, try_start_worker_emulation_command_dispatch,
};
pub(crate) use dispatcher::{
    PreparedWorkerBindingReplay, devtools_get_realms_routes, prepare_automation_runtime_workflow,
    runtime_worker_realms,
};

#[cfg(test)]
mod tests;
