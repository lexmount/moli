use super::*;

pub(super) fn register_service_worker_target_with_active_run(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    info: RendererServiceWorkerTargetInfo,
    active_renderer_run: Option<RendererServiceWorkerRunIdentity>,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    if conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| context.service_worker_target_id_for_renderer_version(info.version_id))
        .is_some()
    {
        return outputs;
    }
    let target_id = conn.gen_target_id();
    let should_emit_created = conn.has_any_target_discovery();
    let mut auto_attach_owners = service_worker_auto_attach_owner_sessions(conn)
        .into_iter()
        .map(|owner_session_id| {
            let waiting_for_debugger =
                conn.auto_attach_owner_waits_for_debugger_on_start(owner_session_id.as_deref());
            (owner_session_id, waiting_for_debugger)
        })
        .collect::<Vec<_>>();
    let related_auto_attach_owners = conn
        .service_worker_auto_attach_related_owner_sessions_for_target(
            browser_context_id,
            info.registration_id,
            info.version_id,
            &info.script_url,
            &info.scope_url,
        );
    let should_pause_on_start_for_related_devtools = related_auto_attach_owners
        .iter()
        .any(|owner| owner.wait_for_debugger_on_start);
    for owner in related_auto_attach_owners {
        if !auto_attach_owners
            .iter()
            .any(|(existing_owner, _)| *existing_owner == owner.owner_session_id)
        {
            auto_attach_owners.push((owner.owner_session_id, owner.wait_for_debugger_on_start));
        }
    }
    let attached_sessions = auto_attach_owners
        .into_iter()
        .map(|(owner, waiting_for_debugger)| (owner, conn.gen_session_id(), waiting_for_debugger))
        .collect::<Vec<_>>();
    let (created_snapshot, version) = {
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        if should_pause_on_start_for_related_devtools {
            context
                .renderer_runtime()
                .set_service_worker_pause_on_start_for_version_for_devtools(info.version_id, true);
        }
        context.insert_service_worker_target(ServiceWorkerTargetState::new(
            info.registration_id,
            info.version_id,
            target_id.clone(),
            info.script_url,
            info.scope_url,
            info.status,
            active_renderer_run,
        ));
        let target = context
            .service_worker_target(&target_id)
            .expect("inserted service-worker target must remain resident");
        let version = target
            .version_identity(browser_context_id)
            .expect("inserted service-worker target must own its version scope");
        let snapshot = if should_emit_created {
            let snapshot = context.devtools_target_info(&target_id);
            debug_assert!(snapshot.is_some());
            snapshot
        } else {
            None
        };
        (snapshot, version)
    };
    let mut attached_outputs = Vec::new();
    for (owner_session_id, session_id, waiting_for_debugger) in attached_sessions {
        if let Some(target_info) = conn
            .prepare_auto_attached_service_worker_session_binding_info_in_browser_context(
                browser_context_id,
                &target_id,
                session_id.clone(),
            )
        {
            let prepared_session = TargetAttachSessionCommit::auto_attached(
                session_id.clone(),
                owner_session_id,
                CdpSessionRoute::ServiceWorkerTarget {
                    browser_context_id: browser_context_id.to_owned(),
                    target_id: target_id.clone(),
                },
                waiting_for_debugger,
            );
            let prepared_attach =
                PreparedTargetAttach::new(target_id.clone(), target_info, [prepared_session]);
            let attachment = conn
                .browser_context_by_id(browser_context_id)
                .and_then(|context| context.service_worker_target(&target_id))
                .and_then(|target| {
                    target.protocol_attachment_identity(browser_context_id, &session_id)
                })
                .expect("auto-attached service-worker session must own an exact attachment");
            attached_outputs.push((attachment, prepared_attach));
        }
    }
    if let Some(target_info) = created_snapshot {
        outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerCreated {
            version: version.clone(),
            target_delta: PreparedTargetHostDelta::created(target_id.clone(), Some(target_info)),
        });
    }
    for (attachment, prepared_attach) in attached_outputs {
        outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerAttached {
            attachment,
            prepared_attach,
        });
    }
    append_service_worker_domain_snapshot(conn, browser_context_id, version, &mut outputs);
    outputs
}

#[cfg(test)]
pub(super) fn register_service_worker_target(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    info: RendererServiceWorkerTargetInfo,
) -> TargetPreparedOutputs {
    register_service_worker_target_with_active_run(conn, browser_context_id, info, None)
}

pub(super) fn record_service_worker_target_started(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    renderer_run: RendererServiceWorkerRunIdentity,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some((version, reloaded_events)) = ({
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        let Some(target_id) = context
            .service_worker_target_id_for_renderer_version(renderer_version_id)
            .map(str::to_owned)
        else {
            return outputs;
        };
        let Some(target) = context.service_worker_target_mut(&target_id) else {
            return outputs;
        };
        let Some(run) = target.mark_worker_started(browser_context_id, renderer_run) else {
            return outputs;
        };
        let reloaded_events = target
            .take_inspector_target_reloaded_after_crash_session_ids()
            .into_iter()
            .filter_map(|session_id| {
                let runtime = target.runtime_attachment_identity_for_run(
                    browser_context_id,
                    &session_id,
                    &run,
                )?;
                Some((
                    runtime,
                    BackgroundProtocolEvent::inspector_target_reloaded_after_crash(Some(
                        &session_id,
                    )),
                ))
            })
            .collect::<Vec<_>>();
        Some((run.version().clone(), reloaded_events))
    }) else {
        return outputs;
    };
    for (runtime, event) in reloaded_events {
        push_service_worker_runtime_events(&mut outputs, runtime, vec![event]);
    }
    append_service_worker_domain_snapshot(conn, browser_context_id, version, &mut outputs);
    outputs
}

pub(super) fn record_service_worker_target_stopped(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    renderer_run: RendererServiceWorkerRunIdentity,
    reason: String,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some((
        target_id,
        version,
        session_runtimes,
        inspector_crashed_session_ids,
        context_reported_session_ids,
        retirement,
    )) = ({
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        let Some(target_id) = context
            .service_worker_target_id_for_renderer_version(renderer_version_id)
            .map(str::to_owned)
        else {
            return outputs;
        };
        let Some(target) = context.service_worker_target_mut(&target_id) else {
            return outputs;
        };
        let Some(retirement) =
            target.mark_worker_stopped(browser_context_id, renderer_run, &reason)
        else {
            return outputs;
        };
        let version = retirement.identity().version().clone();
        let session_runtimes = target
            .session_ids()
            .into_iter()
            .filter_map(|session_id| {
                let attachment =
                    target.protocol_attachment_identity(browser_context_id, &session_id)?;
                Some((
                    session_id,
                    TargetServiceWorkerRuntimeAttachmentIdentity::new(
                        attachment,
                        retirement.identity().clone(),
                    ),
                ))
            })
            .collect::<Vec<_>>();
        let inspector_crashed_session_ids = target.inspector_enabled_session_ids();
        for session_id in &inspector_crashed_session_ids {
            target.record_inspector_target_crashed_for_session(session_id);
        }
        Some((
            target_id,
            version,
            session_runtimes,
            inspector_crashed_session_ids,
            target.runtime_context_reported_session_ids(),
            retirement,
        ))
    })
    else {
        return outputs;
    };

    for (session_id, runtime) in &session_runtimes {
        let mut pending_await_direct_events = Vec::new();
        let mut pending_await_claimed_events = Vec::new();
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            &mut pending_await_direct_events,
            &mut pending_await_claimed_events,
            Some(session_id),
            "Service worker stopped",
        );
        pending_await_direct_events.extend(pending_await_claimed_events);
        push_service_worker_runtime_events(
            &mut outputs,
            runtime.clone(),
            pending_await_direct_events,
        );
    }

    for session_id in inspector_crashed_session_ids {
        let Some((_, runtime)) = session_runtimes
            .iter()
            .find(|(candidate, _)| *candidate == session_id)
        else {
            continue;
        };
        push_service_worker_runtime_events(
            &mut outputs,
            runtime.clone(),
            vec![BackgroundProtocolEvent::inspector_target_crashed(Some(
                &session_id,
            ))],
        );
    }

    if !context_reported_session_ids.is_empty() {
        let mut runtime_context_cleared = Vec::new();
        if let Some(context) = conn.browser_context_by_id_mut(browser_context_id)
            && let Some(target) = context.service_worker_target_mut(&target_id)
        {
            for session_id in context_reported_session_ids {
                if target.is_session(&session_id) {
                    target.clear_runtime_remote_object_tracking(&session_id);
                    target.record_runtime_contexts_cleared_for_frontend(&session_id);
                    if let Some((_, runtime)) = session_runtimes
                        .iter()
                        .find(|(candidate, _)| *candidate == session_id)
                    {
                        runtime_context_cleared.push((session_id, runtime.clone()));
                    }
                }
            }
        }
        for (session_id, runtime) in runtime_context_cleared {
            push_service_worker_runtime_events(
                &mut outputs,
                runtime,
                vec![BackgroundProtocolEvent::runtime_execution_contexts_cleared(
                    Some(&session_id),
                    RuntimeExecutionContextsClearedEvent { target_id: None },
                )],
            );
        }
    }

    append_service_worker_domain_snapshot(conn, browser_context_id, version, &mut outputs);
    outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerRunRetired { retirement });
    outputs
}

pub(super) fn record_service_worker_target_version_updated(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    status: RendererServiceWorkerVersionStatus,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some(version) = ({
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        let Some(target_id) = context
            .service_worker_target_id_for_renderer_version(renderer_version_id)
            .map(str::to_owned)
        else {
            return outputs;
        };
        let Some(target) = context.service_worker_target_mut(&target_id) else {
            return outputs;
        };
        target.update_version_status(browser_context_id, status)
    }) else {
        return outputs;
    };
    let service_worker_domain_sessions =
        service_worker::enabled_sessions_for_browser_context(conn, browser_context_id);
    if service_worker_domain_sessions.is_empty() {
        return outputs;
    }
    let Some(context) = conn.browser_context_by_id(browser_context_id) else {
        return outputs;
    };
    let Some(target_id) = context
        .service_worker_target_id_for_renderer_version(renderer_version_id)
        .map(str::to_owned)
    else {
        return outputs;
    };
    let Some(target) = context.service_worker_target(&target_id) else {
        return outputs;
    };
    let events = service_worker::version_updated_events_for_target(
        context,
        target,
        &service_worker_domain_sessions,
    );
    push_service_worker_version_events(&mut outputs, version, events);
    outputs
}

pub(super) fn remove_service_worker_target(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    active_renderer_run: Option<RendererServiceWorkerRunIdentity>,
) -> TargetPreparedOutputs {
    remove_service_worker_target_with_reason(
        conn,
        browser_context_id,
        renderer_version_id,
        ServiceWorkerTargetRemovalAuthority::RendererDestroyed {
            active_renderer_run,
        },
        "Target closed",
    )
}

pub(super) fn remove_service_worker_target_with_reason(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    authority: ServiceWorkerTargetRemovalAuthority,
    reason: &'static str,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let should_emit_destroyed = conn.has_any_target_discovery();
    let service_worker_domain_sessions =
        service_worker::enabled_sessions_for_browser_context(conn, browser_context_id);
    let target_id = {
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        let Some(target_id) = context
            .service_worker_target_id_for_renderer_version(renderer_version_id)
            .map(str::to_owned)
        else {
            return outputs;
        };
        let Some(target) = context.service_worker_target_mut(&target_id) else {
            return outputs;
        };
        if !authority.authorizes(target) {
            return outputs;
        }
        target_id
    };
    let destroyed_delta = should_emit_destroyed
        .then(|| conn.prepare_destroyed_target_host_delta(&target_id))
        .flatten();
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(mut target) =
        context.remove_service_worker_target_by_renderer_version(renderer_version_id)
    else {
        return outputs;
    };
    let registration_deleted = !context
        .service_worker_targets
        .values()
        .any(|other| other.renderer_registration_id == target.renderer_registration_id);
    let version = target
        .version_identity(browser_context_id)
        .expect("removed service-worker target must retain its exact version identity");
    let run_retirement = target.take_current_run_retirement(browser_context_id);
    let deleted_events = service_worker::deleted_target_events(
        &target,
        registration_deleted,
        &service_worker_domain_sessions,
    );
    push_service_worker_version_events(&mut outputs, version.clone(), deleted_events);
    let session_ids = target.session_ids();
    let attachments = session_ids
        .iter()
        .map(|session_id| {
            (
                session_id.clone(),
                target
                    .protocol_attachment_identity(browser_context_id, session_id)
                    .expect("removed service-worker session must retain its exact attachment"),
            )
        })
        .collect::<Vec<_>>();
    for session_id in &session_ids {
        let mut pending_await_direct_events = Vec::new();
        let mut pending_await_claimed_events = Vec::new();
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            &mut pending_await_direct_events,
            &mut pending_await_claimed_events,
            Some(session_id),
            reason,
        );
        pending_await_direct_events.extend(pending_await_claimed_events);
        let attachment = attachments
            .iter()
            .find(|(candidate, _)| candidate == session_id)
            .map(|(_, attachment)| attachment.clone())
            .expect("removed service-worker session must retain its prepared attachment");
        push_service_worker_attachment_events(
            &mut outputs,
            attachment,
            pending_await_direct_events,
        );
    }
    let mut target_pending_await_events = Vec::new();
    CdpConnection::fail_pending_inspector_awaits_from_service_worker_target_state_background_events_into(
        &mut target_pending_await_events,
        &mut target,
        reason,
    );
    push_service_worker_version_events(&mut outputs, version.clone(), target_pending_await_events);
    if let Some(retirement) = run_retirement {
        outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerRunRetired { retirement });
    }
    for session_id in session_ids {
        let retirement = target
            .take_protocol_attachment_retirement(browser_context_id, &session_id)
            .expect("removed service-worker session must transfer its attachment scope");
        outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerDetached {
            cleanup_plan: TargetSessionDetachCleanupPlan::new(
                target_id.clone(),
                session_id,
                None,
                None,
            ),
            retirement,
        });
    }
    let retirement = target
        .take_version_retirement(browser_context_id)
        .expect("removed service-worker target must transfer its version scope");
    outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerDestroyed {
        retirement,
        target_delta: destroyed_delta,
    });
    outputs
}

pub(in crate::domains::target) async fn close_browser_context_worker_targets_for_dispose_async(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    reason: &'static str,
) -> Vec<BackgroundProtocolEvent> {
    let Some((renderer_runtime, shared_worker_ids, service_worker_ids)) = conn
        .browser_context_by_id(browser_context_id)
        .map(|context| {
            (
                context.renderer_runtime(),
                context
                    .shared_worker_targets
                    .keys()
                    .copied()
                    .collect::<Vec<_>>(),
                context
                    .service_worker_targets
                    .values()
                    .map(|target| target.renderer_version_id)
                    .collect::<Vec<_>>(),
            )
        })
    else {
        return Vec::new();
    };

    let mut outputs = TargetPreparedOutputs::default();
    for instance_id in shared_worker_ids {
        renderer_runtime.close_shared_worker_for_target_close(instance_id);
        outputs.extend(remove_shared_worker_target_with_reason(
            conn,
            browser_context_id,
            instance_id,
            reason,
        ));
    }
    if let Err(error) = renderer_runtime.stop_all_service_workers_for_devtools() {
        tracing::warn!(
            browser_context_id,
            error,
            "browser context disposal could not stop all renderer service workers"
        );
    }
    for version_id in service_worker_ids {
        outputs.extend(remove_service_worker_target_with_reason(
            conn,
            browser_context_id,
            version_id,
            ServiceWorkerTargetRemovalAuthority::BrowserContextDisposal,
            reason,
        ));
    }

    worker_target_removal_background_events_async(conn, outputs).await
}

pub(super) async fn worker_target_removal_background_events_async(
    conn: &mut CdpConnection,
    outputs: TargetPreparedOutputs,
) -> Vec<BackgroundProtocolEvent> {
    let owner = CommandOwnerScope::capture(conn, None);
    let mut command_context = crate::conn::CommandDispatchContext::default();
    let mut prepared_outputs =
        ProtocolOutputPayloads::from_slot(TargetPreparedOutputSlot::from_outputs(outputs));
    emit_target_lifecycle_events(
        conn,
        &mut ProtocolOutputProjectionContext::new(&owner, &mut command_context),
        &mut prepared_outputs,
    )
    .await;
    command_context.take_protocol_events()
}

pub(super) fn record_service_worker_target_console_message(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    renderer_run: RendererServiceWorkerRunIdentity,
    message: RendererServiceWorkerConsoleMessage,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(target_id) = context
        .service_worker_target_id_for_renderer_version(renderer_version_id)
        .map(str::to_owned)
    else {
        return outputs;
    };
    let Some(target) = context.service_worker_target_mut(&target_id) else {
        return outputs;
    };
    let Some(run) = target.observe_worker_run(browser_context_id, renderer_run) else {
        return outputs;
    };
    target.record_console_message(message.message, message.args, message.stack);
    for session_id in target.session_ids() {
        let Some(attachment) = target.protocol_attachment_identity(browser_context_id, &session_id)
        else {
            continue;
        };
        let runtime = TargetServiceWorkerRuntimeAttachmentIdentity::new(attachment, run.clone());
        let console_messages = target.pending_console_domain_messages(&session_id).to_vec();
        if !console_messages.is_empty() {
            let console_end = target.console_message_count();
            target.mark_console_domain_emitted(&session_id, console_end);
            outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerConsoleMessages {
                runtime: runtime.clone(),
                messages: console_messages,
                console_end,
            });
        }
        let runtime_messages = target
            .pending_runtime_console_messages(&session_id)
            .to_vec();
        if !runtime_messages.is_empty() {
            let console_end = target.console_message_count();
            target.mark_runtime_console_emitted(&session_id, console_end);
            outputs.push(
                WorkerTargetLifecycleOutput::ServiceWorkerRuntimeConsoleMessages {
                    runtime,
                    messages: runtime_messages,
                    console_end,
                },
            );
        }
    }
    outputs
}

pub(super) fn record_service_worker_target_exception_message(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    renderer_run: RendererServiceWorkerRunIdentity,
    message: RendererServiceWorkerExceptionMessage,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let service_worker_domain_sessions =
        service_worker::enabled_sessions_for_browser_context(conn, browser_context_id);
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(target_id) = context
        .service_worker_target_id_for_renderer_version(renderer_version_id)
        .map(str::to_owned)
    else {
        return outputs;
    };
    let Some(target) = context.service_worker_target_mut(&target_id) else {
        return outputs;
    };
    let Some(run) = target.observe_worker_run(browser_context_id, renderer_run) else {
        return outputs;
    };
    let service_worker_error_events =
        service_worker::error_reported_events(target, &message, &service_worker_domain_sessions);
    target.record_exception_message(message);
    push_service_worker_run_events(&mut outputs, run.clone(), service_worker_error_events);
    for session_id in target.session_ids() {
        let Some(attachment) = target.protocol_attachment_identity(browser_context_id, &session_id)
        else {
            continue;
        };
        let exception_start = target
            .exception_message_count()
            .saturating_sub(target.pending_runtime_exception_messages(&session_id).len());
        let exception_messages = target
            .pending_runtime_exception_messages(&session_id)
            .to_vec();
        if !exception_messages.is_empty() {
            let exception_end = target.exception_message_count();
            target.mark_runtime_exception_emitted(&session_id, exception_end);
            outputs.push(
                WorkerTargetLifecycleOutput::ServiceWorkerRuntimeExceptionMessages {
                    runtime: TargetServiceWorkerRuntimeAttachmentIdentity::new(
                        attachment,
                        run.clone(),
                    ),
                    messages: exception_messages,
                    exception_start,
                    exception_end,
                },
            );
        }
    }
    outputs
}

pub(super) fn record_service_worker_target_fetch_diagnostic(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    renderer_run: RendererServiceWorkerRunIdentity,
    diagnostic: RendererServiceWorkerFetchDiagnostic,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(target_id) = context
        .service_worker_target_id_for_renderer_version(renderer_version_id)
        .map(str::to_owned)
    else {
        return outputs;
    };
    let Some(target) = context.service_worker_target_mut(&target_id) else {
        return outputs;
    };
    let Some(run) = target.observe_worker_run(browser_context_id, renderer_run) else {
        return outputs;
    };
    target.record_fetch_diagnostic(diagnostic);
    for session_id in target.session_ids() {
        let Some(attachment) = target.protocol_attachment_identity(browser_context_id, &session_id)
        else {
            continue;
        };
        let diagnostics = target.pending_fetch_diagnostics(&session_id).to_vec();
        if !diagnostics.is_empty() {
            let diagnostic_end = target.fetch_diagnostic_count();
            let diagnostic_start = diagnostic_end.saturating_sub(diagnostics.len());
            target.mark_fetch_diagnostics_emitted(&session_id, diagnostic_end);
            outputs.push(WorkerTargetLifecycleOutput::ServiceWorkerFetchDiagnostics {
                runtime: TargetServiceWorkerRuntimeAttachmentIdentity::new(attachment, run.clone()),
                diagnostics,
                diagnostic_start,
                diagnostic_end,
            });
        }
    }
    outputs
}

pub(super) fn record_service_worker_target_runtime_inspector_messages(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_version_id: u64,
    renderer_run: RendererServiceWorkerRunIdentity,
    inspector_session_id: Option<String>,
    messages: Vec<RendererRuntimeInspectorMessage>,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    if messages.is_empty() {
        return outputs;
    }
    let runtimes = {
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        let Some(target_id) = context
            .service_worker_target_id_for_renderer_version(renderer_version_id)
            .map(str::to_owned)
        else {
            return outputs;
        };
        let Some(target) = context.service_worker_target_mut(&target_id) else {
            return outputs;
        };
        let Some(run) = target.observe_worker_run(browser_context_id, renderer_run) else {
            return outputs;
        };
        let session_ids = if let Some(session_id) = inspector_session_id {
            if !target.is_session(&session_id) {
                return outputs;
            }
            vec![session_id]
        } else {
            target.session_ids()
        };
        session_ids
            .into_iter()
            .filter_map(|session_id| {
                Some(TargetServiceWorkerRuntimeAttachmentIdentity::new(
                    target.protocol_attachment_identity(browser_context_id, &session_id)?,
                    run.clone(),
                ))
            })
            .collect::<Vec<_>>()
    };
    for runtime in runtimes {
        let session_id = runtime.session_id().to_owned();
        let mut response_events = Vec::new();
        let mut background_events = Vec::new();
        let current_response_seen = route_worker_runtime_inspector_messages_into(
            conn,
            messages.clone(),
            &session_id,
            &mut response_events,
            &mut background_events,
        );
        debug_assert!(!current_response_seen);

        let mut pending_runtime_console = None;
        let mut pending_runtime_exceptions = None;
        if let Some(target) = exact_service_worker_runtime_target_mut(conn, &runtime) {
            let pending_console = target
                .pending_runtime_console_messages(&session_id)
                .to_vec();
            if !pending_console.is_empty() {
                let console_end = target.console_message_count();
                target.mark_runtime_console_emitted(&session_id, console_end);
                pending_runtime_console = Some((pending_console, console_end));
            }
            let exception_start = target
                .exception_message_count()
                .saturating_sub(target.pending_runtime_exception_messages(&session_id).len());
            let pending_exceptions = target
                .pending_runtime_exception_messages(&session_id)
                .to_vec();
            if !pending_exceptions.is_empty() {
                let exception_end = target.exception_message_count();
                target.mark_runtime_exception_emitted(&session_id, exception_end);
                pending_runtime_exceptions =
                    Some((pending_exceptions, exception_start, exception_end));
            }
        }
        outputs.push(
            WorkerTargetLifecycleOutput::ServiceWorkerRuntimeInspectorMessages {
                runtime,
                background_events,
                response_events,
                pending_runtime_console,
                pending_runtime_exceptions,
            },
        );
    }
    outputs
}

pub(super) fn service_worker_fetch_diagnostic_events(
    session_id: &str,
    target_id: &str,
    diagnostics: &[RendererServiceWorkerFetchDiagnostic],
    diagnostic_start: usize,
) -> Vec<BackgroundProtocolEvent> {
    let mut events = Vec::new();
    emit_service_worker_fetch_diagnostic_events(
        &mut events,
        session_id,
        target_id,
        diagnostics,
        diagnostic_start,
    );
    events
}

pub(super) fn emit_service_worker_fetch_diagnostic_events(
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: &str,
    target_id: &str,
    diagnostics: &[RendererServiceWorkerFetchDiagnostic],
    diagnostic_start: usize,
) {
    let base_timestamp = monotonic_timestamp_seconds();
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        let timestamp = base_timestamp + ((index + 1) as f64 * 0.000_001);
        let request_id = service_worker_fetch_diagnostic_request_id(
            target_id,
            diagnostic.internal_id,
            diagnostic_start + index,
        );
        let loader_id = service_worker_fetch_diagnostic_loader_id(target_id);
        let resource_type = service_worker_fetch_diagnostic_resource_type(&diagnostic.destination);
        let request_url = parse_service_worker_fetch_diagnostic_url(&diagnostic.request_url);
        let document_url =
            Url::parse(&diagnostic.document_url).unwrap_or_else(|_| request_url.clone());

        let request_event_index = out.len();
        network::emit_request_will_be_sent(
            out,
            Some(session_id),
            &request_id,
            target_id,
            &loader_id,
            timestamp,
            &document_url,
            &request_url,
            &diagnostic.method,
            diagnostic.request_body.as_deref(),
            &diagnostic.request_headers,
            resource_type,
            SubresourceRequestInitiatorType::Other,
            None,
            false,
            None,
            &[],
            true,
        );
        tag_service_worker_fetch_diagnostic_event(out.get_mut(request_event_index), diagnostic);

        match &diagnostic.result {
            RendererServiceWorkerFetchDiagnosticResult::Fallback => {
                let failed_event_index = out.len();
                network::emit_loading_failed(
                    out,
                    Some(session_id),
                    &request_id,
                    target_id,
                    &loader_id,
                    timestamp + 0.000_000_5,
                    "ServiceWorkerFallback",
                    resource_type,
                );
                tag_service_worker_fetch_diagnostic_event(
                    out.get_mut(failed_event_index),
                    diagnostic,
                );
            }
            RendererServiceWorkerFetchDiagnosticResult::Response {
                final_url,
                status,
                status_text,
                response_headers,
                body_len,
            } => {
                let final_url = Url::parse(final_url).unwrap_or_else(|_| request_url.clone());
                let response_event_index = out.len();
                network::emit_response_received(
                    out,
                    Some(session_id),
                    &request_id,
                    target_id,
                    &loader_id,
                    timestamp + 0.000_000_5,
                    &final_url,
                    *status,
                    Some(status_text),
                    response_headers,
                    &[],
                    *body_len,
                    false,
                    None,
                    false,
                    resource_type,
                    &[],
                    None,
                );
                tag_service_worker_fetch_diagnostic_event(
                    out.get_mut(response_event_index),
                    diagnostic,
                );
                if let Some(response) = out
                    .get_mut(response_event_index)
                    .and_then(BackgroundProtocolEvent::protocol_params_mut)
                    .and_then(|params| params.get_mut("response"))
                {
                    response["fromServiceWorker"] = json!(true);
                }
                let body_finished_event_start = out.len();
                network::emit_body_finished(
                    out,
                    Some(session_id),
                    &request_id,
                    target_id,
                    &loader_id,
                    timestamp + 0.000_001,
                    *body_len,
                    resource_type,
                );
                for event in &mut out[body_finished_event_start..] {
                    tag_service_worker_fetch_diagnostic_event(Some(event), diagnostic);
                }
            }
            RendererServiceWorkerFetchDiagnosticResult::Failure { message } => {
                let failed_event_index = out.len();
                network::emit_loading_failed(
                    out,
                    Some(session_id),
                    &request_id,
                    target_id,
                    &loader_id,
                    timestamp + 0.000_000_5,
                    message,
                    resource_type,
                );
                tag_service_worker_fetch_diagnostic_event(
                    out.get_mut(failed_event_index),
                    diagnostic,
                );
            }
        }
    }
}

pub(super) fn service_worker_fetch_diagnostic_request_id(
    target_id: &str,
    internal_id: u64,
    diagnostic_index: usize,
) -> String {
    format!("{target_id}.sw-fetch.{internal_id}.{diagnostic_index}")
}

pub(super) fn service_worker_fetch_diagnostic_loader_id(target_id: &str) -> String {
    format!("{target_id}.service-worker")
}

pub(super) fn parse_service_worker_fetch_diagnostic_url(value: &str) -> Url {
    Url::parse(value).unwrap_or_else(|_| Url::parse("about:blank").expect("valid fallback URL"))
}

pub(super) fn service_worker_runtime_is_registry_current(
    conn: &CdpConnection,
    runtime: &TargetServiceWorkerRuntimeAttachmentIdentity,
) -> bool {
    if !runtime.is_current() {
        return false;
    }
    let attachment = runtime.attachment();
    let Some(target) = conn
        .browser_context_by_id(attachment.browser_context_id())
        .and_then(|context| context.service_worker_target(attachment.target_id()))
    else {
        return false;
    };
    target.observes_runtime_identity(attachment.browser_context_id(), runtime)
}

pub(super) fn exact_service_worker_runtime_target_mut<'a>(
    conn: &'a mut CdpConnection,
    runtime: &TargetServiceWorkerRuntimeAttachmentIdentity,
) -> Option<&'a mut ServiceWorkerTargetState> {
    if !runtime.is_current() {
        return None;
    }
    let attachment = runtime.attachment();
    let target = conn
        .browser_context_by_id_mut(attachment.browser_context_id())?
        .service_worker_target_mut(attachment.target_id())?;
    let is_exact = target.observes_runtime_identity(attachment.browser_context_id(), runtime);
    is_exact.then_some(target)
}

pub(super) fn tag_service_worker_fetch_diagnostic_event(
    event: Option<&mut BackgroundProtocolEvent>,
    diagnostic: &RendererServiceWorkerFetchDiagnostic,
) {
    let Some(params) = event.and_then(BackgroundProtocolEvent::protocol_params_mut) else {
        return;
    };
    params["__moliServiceWorkerFetchDiagnostic"] = json!(true);
    params["__moliServiceWorkerFetchInternalId"] = json!(diagnostic.internal_id);
    params["__moliServiceWorkerFetchResult"] =
        json!(service_worker_fetch_diagnostic_result_name(diagnostic));
}

pub(super) fn service_worker_fetch_diagnostic_result_name(
    diagnostic: &RendererServiceWorkerFetchDiagnostic,
) -> &'static str {
    match &diagnostic.result {
        RendererServiceWorkerFetchDiagnosticResult::Fallback => "fallback",
        RendererServiceWorkerFetchDiagnosticResult::Response { .. } => "response",
        RendererServiceWorkerFetchDiagnosticResult::Failure { .. } => "failure",
    }
}

pub(super) fn service_worker_fetch_diagnostic_resource_type(
    destination: &str,
) -> DevToolsNetworkResourceType {
    match destination {
        "document" | "iframe" | "frame" => DevToolsNetworkResourceType::Document,
        "style" => DevToolsNetworkResourceType::Stylesheet,
        "image" => DevToolsNetworkResourceType::Image,
        "font" => DevToolsNetworkResourceType::Font,
        "audio" | "video" => DevToolsNetworkResourceType::Media,
        "script" | "worker" | "sharedworker" | "serviceworker" => {
            DevToolsNetworkResourceType::Script
        }
        "track" => DevToolsNetworkResourceType::TextTrack,
        "manifest" => DevToolsNetworkResourceType::Manifest,
        "report" => DevToolsNetworkResourceType::CspViolationReport,
        "" => DevToolsNetworkResourceType::Fetch,
        _ => DevToolsNetworkResourceType::Other,
    }
}
