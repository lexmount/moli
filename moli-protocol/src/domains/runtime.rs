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

/// Detaches the Page's renderer Inspector endpoint after Page-owned cleanup
/// commands have completed.
pub(in crate::domains) async fn detach_page_session_inspector_async(
    conn: &mut CdpConnection,
    session_id: &str,
) {
    if let Err(error) = conn
        .detach_runtime_inspector_session_for_session_owner_async(Some(session_id))
        .await
    {
        tracing::debug!(
            session_id,
            %error,
            "renderer Inspector session was already unavailable during disposal"
        );
    }
}

/// Disables Runtime state owned by one DevTools session. Renderer Inspector
/// detachment is a separate lifecycle phase and runs after every browser-side
/// domain handler has disabled itself.
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

/// Detaches the renderer-side Inspector endpoint after every browser-side
/// domain handler has disabled itself.
pub(in crate::domains) async fn detach_session_inspector_async(
    conn: &mut CdpConnection,
    plan: &SessionDisposalPlan,
) -> anyhow::Result<()> {
    let session_id = plan.session_id();
    match plan.target() {
        SessionDisposalTarget::PageTarget { .. } => {
            detach_page_session_inspector_async(conn, session_id).await;
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
    complete_pending_runtime_command_at_response_boundary,
    devtools_deep_serialization_options_json,
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
