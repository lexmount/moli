use super::*;

pub(super) fn push_service_worker_version_events(
    outputs: &mut TargetPreparedOutputs,
    version: TargetServiceWorkerVersionIdentity,
    events: Vec<BackgroundProtocolEvent>,
) {
    if events.is_empty() {
        return;
    }
    outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerVersionEvents { version, events });
}

pub(super) fn push_service_worker_run_events(
    outputs: &mut TargetPreparedOutputs,
    run: TargetServiceWorkerRunIdentity,
    events: Vec<BackgroundProtocolEvent>,
) {
    if events.is_empty() {
        return;
    }
    outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerRunEvents { run, events });
}

pub(super) fn push_service_worker_attachment_events(
    outputs: &mut TargetPreparedOutputs,
    attachment: TargetServiceWorkerProtocolAttachmentIdentity,
    events: Vec<BackgroundProtocolEvent>,
) {
    if events.is_empty() {
        return;
    }
    outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerAttachmentEvents { attachment, events });
}

pub(super) fn push_service_worker_runtime_events(
    outputs: &mut TargetPreparedOutputs,
    runtime: TargetServiceWorkerRuntimeAttachmentIdentity,
    events: Vec<BackgroundProtocolEvent>,
) {
    if events.is_empty() {
        return;
    }
    outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerRuntimeEvents { runtime, events });
}

pub(super) fn shared_worker_target_lifecycle_outputs_for_events(
    conn: &mut CdpConnection,
    browser_context_id: String,
    events: Vec<RendererSharedWorkerTargetEvent>,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    for event in events {
        match event {
            RendererSharedWorkerTargetEvent::Created(info) => {
                let owner_target_id =
                    conn.browser_context_by_id(&browser_context_id)
                        .and_then(|context| {
                            context.target_id_for_renderer_owner_local_host_id(
                                info.owner_local_host_id,
                            )
                        });
                outputs.extend(register_shared_worker_target(
                    conn,
                    &browser_context_id,
                    owner_target_id,
                    info,
                ));
            }
            RendererSharedWorkerTargetEvent::Destroyed { instance_id } => {
                outputs.extend(remove_shared_worker_target(
                    conn,
                    &browser_context_id,
                    instance_id,
                ));
            }
            RendererSharedWorkerTargetEvent::Console {
                instance_id,
                message,
            } => {
                outputs.extend(record_shared_worker_target_console_message(
                    conn,
                    &browser_context_id,
                    instance_id,
                    message,
                ));
            }
            RendererSharedWorkerTargetEvent::RuntimeInspectorMessages {
                instance_id,
                inspector_session_id,
                messages,
            } => {
                outputs.extend(record_shared_worker_target_runtime_inspector_messages(
                    conn,
                    &browser_context_id,
                    instance_id,
                    inspector_session_id,
                    messages,
                ));
            }
        }
    }
    outputs
}

pub(in crate::domains) fn shared_worker_target_lifecycle_prepared_outputs_for_event(
    conn: &mut CdpConnection,
    browser_context_id: String,
    event: RendererSharedWorkerTargetEvent,
) -> TargetPreparedOutputs {
    shared_worker_target_lifecycle_outputs_for_events(conn, browser_context_id, vec![event])
}

pub(in crate::domains) fn service_worker_target_lifecycle_prepared_outputs_for_event(
    conn: &mut CdpConnection,
    browser_context_id: String,
    event: RendererServiceWorkerTargetEvent,
) -> TargetPreparedOutputs {
    service_worker_target_lifecycle_outputs_for_events(conn, browser_context_id, vec![event])
}

pub(in crate::domains) fn dedicated_worker_target_lifecycle_prepared_outputs_for_event(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    page_source: Option<(TargetPageResidenceIdentity, RendererPageResidenceIdentity)>,
    event: RendererDedicatedWorkerTargetEvent,
) -> TargetPreparedOutputs {
    let Some((owner_page, owner_renderer_page)) = page_source else {
        return TargetPreparedOutputs::default();
    };
    if let RendererDedicatedWorkerTargetEvent::Destroyed { instance_id } = event {
        let context_id = owner_page.browser_context_id();
        if conn
            .browser_context_by_id(context_id)
            .and_then(|context| context.dedicated_worker_targets.get(&instance_id))
            .is_none_or(|target| {
                target.owner_page != owner_page
                    || target.renderer_owner_local_host_id
                        != owner_renderer_page.owner_local_host_id()
            })
        {
            return TargetPreparedOutputs::default();
        }
        return prepare_dedicated_worker_target_retirement(
            conn,
            context_id,
            instance_id,
            DedicatedWorkerRetirementCause::RendererDestroyed,
        );
    }
    if !conn.target_page_output_residence_is_current(&owner_page) {
        return TargetPreparedOutputs::default();
    }
    let browser_context_id = owner_page.browser_context_id().to_owned();
    let owner_page_network_sessions = conn.network_event_session_ids_for_owner(owner);
    dedicated_worker_target_lifecycle_outputs_for_events(
        conn,
        browser_context_id,
        owner_page,
        owner_renderer_page,
        owner_page_network_sessions,
        vec![event],
    )
}

pub(super) fn dedicated_worker_target_lifecycle_outputs_for_events(
    conn: &mut CdpConnection,
    browser_context_id: String,
    owner_page: TargetPageResidenceIdentity,
    owner_renderer_page: RendererPageResidenceIdentity,
    owner_page_network_sessions: Vec<Option<String>>,
    events: Vec<RendererDedicatedWorkerTargetEvent>,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    for event in events {
        match event {
            RendererDedicatedWorkerTargetEvent::Created(info) => {
                outputs.extend(register_dedicated_worker_target(
                    conn,
                    &browser_context_id,
                    owner_page.clone(),
                    owner_renderer_page,
                    owner_page_network_sessions.clone(),
                    info,
                ));
            }
            RendererDedicatedWorkerTargetEvent::ScriptLoaded {
                instance_id,
                script_url,
                response,
            } => {
                outputs.extend(record_dedicated_worker_main_script(
                    conn,
                    &browser_context_id,
                    instance_id,
                    script_url,
                    crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(response),
                ));
            }
            RendererDedicatedWorkerTargetEvent::ScriptLoadFailed {
                instance_id,
                script_url,
                error_message,
                response,
            } => {
                outputs.extend(record_dedicated_worker_main_script(
                    conn,
                    &browser_context_id,
                    instance_id,
                    script_url,
                    crate::conn::DedicatedWorkerMainScriptOutcome::Failed {
                        error_message,
                        response,
                    },
                ));
            }
            RendererDedicatedWorkerTargetEvent::Console {
                instance_id,
                message,
            } => {
                outputs.extend(record_dedicated_worker_target_console_message(
                    conn,
                    &browser_context_id,
                    instance_id,
                    message,
                ));
            }
            RendererDedicatedWorkerTargetEvent::RuntimeInspectorMessages {
                instance_id,
                inspector_session_id,
                messages,
            } => {
                outputs.extend(record_dedicated_worker_target_runtime_inspector_messages(
                    conn,
                    &browser_context_id,
                    instance_id,
                    inspector_session_id,
                    messages,
                ));
            }
            RendererDedicatedWorkerTargetEvent::Destroyed { instance_id } => {
                outputs.extend(prepare_dedicated_worker_target_retirement(
                    conn,
                    &browser_context_id,
                    instance_id,
                    DedicatedWorkerRetirementCause::RendererDestroyed,
                ));
            }
        }
    }
    outputs
}

pub(super) fn service_worker_target_lifecycle_outputs_for_events(
    conn: &mut CdpConnection,
    browser_context_id: String,
    events: Vec<RendererServiceWorkerTargetEvent>,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    for event in events {
        match event {
            RendererServiceWorkerTargetEvent::Created { info, active_run } => {
                outputs.extend(register_service_worker_target_with_active_run(
                    conn,
                    &browser_context_id,
                    info,
                    active_run,
                ));
            }
            RendererServiceWorkerTargetEvent::Started { version_id, run } => {
                outputs.extend(record_service_worker_target_started(
                    conn,
                    &browser_context_id,
                    version_id,
                    run,
                ));
            }
            RendererServiceWorkerTargetEvent::Stopped {
                version_id,
                run,
                reason,
            } => {
                outputs.extend(record_service_worker_target_stopped(
                    conn,
                    &browser_context_id,
                    version_id,
                    run,
                    reason,
                ));
            }
            RendererServiceWorkerTargetEvent::Destroyed {
                version_id,
                active_run,
            } => {
                outputs.extend(remove_service_worker_target(
                    conn,
                    &browser_context_id,
                    version_id,
                    active_run,
                ));
            }
            RendererServiceWorkerTargetEvent::VersionUpdated { version_id, status } => {
                outputs.extend(record_service_worker_target_version_updated(
                    conn,
                    &browser_context_id,
                    version_id,
                    status,
                ));
            }
            RendererServiceWorkerTargetEvent::Console {
                version_id,
                run,
                message,
            } => {
                outputs.extend(record_service_worker_target_console_message(
                    conn,
                    &browser_context_id,
                    version_id,
                    run,
                    message,
                ));
            }
            RendererServiceWorkerTargetEvent::Exception {
                version_id,
                run,
                message,
            } => {
                outputs.extend(record_service_worker_target_exception_message(
                    conn,
                    &browser_context_id,
                    version_id,
                    run,
                    message,
                ));
            }
            RendererServiceWorkerTargetEvent::FetchDiagnostic {
                version_id,
                run,
                diagnostic,
            } => {
                outputs.extend(record_service_worker_target_fetch_diagnostic(
                    conn,
                    &browser_context_id,
                    version_id,
                    run,
                    diagnostic,
                ));
            }
            RendererServiceWorkerTargetEvent::RuntimeInspectorMessages {
                version_id,
                run,
                inspector_session_id,
                messages,
            } => {
                outputs.extend(record_service_worker_target_runtime_inspector_messages(
                    conn,
                    &browser_context_id,
                    version_id,
                    run,
                    inspector_session_id,
                    messages,
                ));
            }
        }
    }
    outputs
}

pub(super) fn append_service_worker_domain_snapshot(
    conn: &CdpConnection,
    browser_context_id: &str,
    version: TargetServiceWorkerVersionIdentity,
    outputs: &mut TargetPreparedOutputs,
) {
    let session_ids =
        service_worker::enabled_sessions_for_browser_context(conn, browser_context_id);
    if session_ids.is_empty() {
        return;
    }
    let events =
        service_worker::snapshot_events_for_browser_context(conn, browser_context_id, &session_ids);
    push_service_worker_version_events(outputs, version, events);
}
