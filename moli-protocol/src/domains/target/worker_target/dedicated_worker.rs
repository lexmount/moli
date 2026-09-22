use super::*;

pub(super) fn dedicated_worker_target_is_current(
    conn: &CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: u64,
    target_id: &str,
) -> bool {
    let Some(context) = conn.browser_context_by_id(browser_context_id) else {
        return false;
    };
    context
        .dedicated_worker_targets
        .get(&renderer_instance_id)
        .is_some_and(|target| {
            target.target_id == target_id
                && context.target_page_residence_is_current(&target.owner_page)
        })
}

pub(super) fn register_dedicated_worker_target(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    owner_page: TargetPageResidenceIdentity,
    owner_renderer_page: RendererPageResidenceIdentity,
    owner_page_network_sessions: Vec<Option<String>>,
    info: RendererDedicatedWorkerTargetInfo,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    if info.owner_local_host_id != owner_renderer_page.owner_local_host_id()
        || info.page_id != owner_renderer_page.page_id()
    {
        return outputs;
    }
    if conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| {
            context.dedicated_worker_target_id_for_renderer_instance(info.instance_id)
        })
        .is_some()
    {
        return outputs;
    }
    let target_id = conn.gen_target_id();
    let request_url = match Url::parse(&info.request_url) {
        Ok(url) => url,
        Err(_) => return outputs,
    };
    let document_url = match Url::parse(&info.document_url) {
        Ok(url) => url,
        Err(_) => return outputs,
    };
    let owner_target_id = owner_page.target_id().unwrap_or_default().to_owned();
    let should_emit_created = conn.has_any_target_discovery();
    let created_snapshot = {
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return TargetPreparedOutputs::default();
        };
        context.insert_dedicated_worker_target(crate::conn::DedicatedWorkerTargetState::new(
            owner_page.clone(),
            info.owner_local_host_id,
            info.instance_id,
            target_id.clone(),
            info.name,
            owner_page_network_sessions.clone(),
        ));
        should_emit_created
            .then(|| context.devtools_target_info(&target_id))
            .flatten()
    };
    if let Some(target_info) = created_snapshot {
        outputs.push(WorkerTargetLifecycleOutput::DedicatedWorkerCreated {
            browser_context_id: browser_context_id.to_owned(),
            renderer_instance_id: info.instance_id,
            target_delta: PreparedTargetHostDelta::created(target_id.clone(), Some(target_info)),
        });
    }
    let timestamp = monotonic_timestamp_seconds();
    for session_id in &owner_page_network_sessions {
        let mut events = Vec::new();
        network::emit_request_will_be_sent(
            &mut events,
            session_id.as_deref(),
            &target_id,
            &owner_target_id,
            &owner_target_id,
            timestamp,
            &document_url,
            &request_url,
            "GET",
            None,
            &[],
            DevToolsNetworkResourceType::Script,
            SubresourceRequestInitiatorType::Other,
            None,
            false,
            None,
            &[],
            true,
        );
        if !events.is_empty() {
            outputs.push(WorkerTargetLifecycleOutput::DedicatedWorkerEvents {
                browser_context_id: browser_context_id.to_owned(),
                renderer_instance_id: info.instance_id,
                target_id: target_id.clone(),
                events,
            });
        }
    }
    outputs
}

pub(super) fn record_dedicated_worker_main_script(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: u64,
    script_url: String,
    outcome: crate::conn::DedicatedWorkerMainScriptOutcome,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some(owner_page) = conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| context.dedicated_worker_targets.get(&renderer_instance_id))
        .map(|target| target.owner_page.clone())
    else {
        return outputs;
    };
    let auto_attach_owners = dedicated_worker_auto_attach_owner_sessions(conn, &owner_page);
    let attached_sessions = auto_attach_owners
        .into_iter()
        .map(|owner| {
            let waiting = conn.auto_attach_owner_waits_for_debugger_on_start(owner.as_deref());
            (owner, conn.gen_session_id(), waiting)
        })
        .collect::<Vec<_>>();
    let pause_failed_target_until_debugger_resume = attached_sessions
        .iter()
        .any(|(_, _, waiting_for_debugger)| *waiting_for_debugger);
    let should_emit_info_changed = conn.has_any_target_discovery();
    let (target_id, page_extra_events) = {
        let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return outputs;
        };
        let Some(target) = context
            .dedicated_worker_targets
            .get_mut(&renderer_instance_id)
        else {
            return outputs;
        };
        let target_id = target.target_id.clone();
        let owner_page_network_sessions = target.owner_page_network_sessions.clone();
        let page_extra_events = dedicated_worker_main_script_page_extra_events(
            &target_id,
            &owner_page_network_sessions,
            &outcome,
        );
        target.record_main_script(
            script_url,
            outcome,
            pause_failed_target_until_debugger_resume,
        );
        (target_id, page_extra_events)
    };
    for (_session_id, events) in page_extra_events {
        outputs.push(WorkerTargetLifecycleOutput::DedicatedWorkerEvents {
            browser_context_id: browser_context_id.to_owned(),
            renderer_instance_id,
            target_id: target_id.clone(),
            events,
        });
    }
    let mut prepared_attaches = Vec::new();
    for (owner_session_id, session_id, waiting_for_debugger) in attached_sessions {
        let Some(target_info) = conn
            .prepare_auto_attached_dedicated_worker_session_binding_info_in_browser_context(
                browser_context_id,
                &target_id,
                session_id.clone(),
            )
        else {
            continue;
        };
        let prepared_session = TargetAttachSessionCommit::auto_attached(
            session_id.clone(),
            owner_session_id,
            CdpSessionRoute::DedicatedWorkerTarget {
                browser_context_id: browser_context_id.to_owned(),
                target_id: target_id.clone(),
            },
            waiting_for_debugger,
        );
        if waiting_for_debugger
            && let Some(context) = conn.browser_context_by_id_mut(browser_context_id)
            && let Some(target) = context
                .dedicated_worker_targets
                .get_mut(&renderer_instance_id)
        {
            target.allow_main_script_network_replay_to(&session_id);
        }
        prepared_attaches.push(WorkerTargetLifecycleOutput::DedicatedWorkerAttached {
            browser_context_id: browser_context_id.to_owned(),
            renderer_instance_id,
            target_id: target_id.clone(),
            session_id,
            prepared_attach: PreparedTargetAttach::new(
                target_id.clone(),
                target_info,
                [prepared_session],
            ),
        });
    }
    let changed_snapshot = should_emit_info_changed
        .then(|| {
            conn.browser_context_by_id(browser_context_id)
                .and_then(|context| context.devtools_target_info(&target_id))
        })
        .flatten();
    if let Some(target_info) = changed_snapshot {
        outputs.push(WorkerTargetLifecycleOutput::DedicatedWorkerInfoChanged {
            browser_context_id: browser_context_id.to_owned(),
            renderer_instance_id,
            target_id: target_id.clone(),
            target_delta: PreparedTargetHostDelta::info_changed(
                target_id.clone(),
                Some(target_info),
            ),
        });
    }
    outputs
        .worker_target_lifecycle_outputs
        .extend(prepared_attaches);
    let enabled_sessions = conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| context.dedicated_worker_targets.get(&renderer_instance_id))
        .map(|target| {
            target
                .session_ids()
                .into_iter()
                .filter(|session_id| {
                    target.network_enabled(session_id)
                        && !target.main_script_was_delivered_to(session_id)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for session_id in enabled_sessions {
        let events = conn
            .browser_context_by_id(browser_context_id)
            .and_then(|context| context.dedicated_worker_targets.get(&renderer_instance_id))
            .and_then(|target| target.main_script())
            .map(|script| {
                dedicated_worker_main_script_worker_events(&target_id, Some(&session_id), script)
            })
            .unwrap_or_default();
        if let Some(context) = conn.browser_context_by_id_mut(browser_context_id)
            && let Some(target) = context
                .dedicated_worker_targets
                .get_mut(&renderer_instance_id)
        {
            target.mark_main_script_delivered_to(&session_id);
        }
        if !events.is_empty() {
            outputs.push(WorkerTargetLifecycleOutput::DedicatedWorkerEvents {
                browser_context_id: browser_context_id.to_owned(),
                renderer_instance_id,
                target_id: target_id.clone(),
                events,
            });
        }
    }
    outputs
}

pub(in crate::domains::target) fn dedicated_worker_auto_attach_owner_session_allowed(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    owner_page: &TargetPageResidenceIdentity,
) -> bool {
    let Some(owner_session_id) = owner_session_id else {
        return false;
    };
    conn.target_page_residence_identity_for_session(Some(owner_session_id))
        .as_ref()
        == Some(owner_page)
}

pub(super) fn shared_worker_auto_attach_owner_sessions(
    conn: &CdpConnection,
) -> Vec<Option<String>> {
    conn.auto_attach_owner_sessions_for_target_type("shared_worker")
        .into_iter()
        .filter(|owner_session_id| {
            super::super::browser_level_auto_attach_owner_session_allowed(
                conn,
                owner_session_id.as_deref(),
            )
        })
        .collect()
}

pub(super) fn service_worker_auto_attach_owner_sessions(
    conn: &CdpConnection,
) -> Vec<Option<String>> {
    conn.auto_attach_owner_sessions_for_target_type("service_worker")
        .into_iter()
        .filter(|owner_session_id| {
            super::super::browser_level_auto_attach_owner_session_allowed(
                conn,
                owner_session_id.as_deref(),
            )
        })
        .collect()
}

pub(super) fn dedicated_worker_auto_attach_owner_sessions(
    conn: &CdpConnection,
    owner_page: &TargetPageResidenceIdentity,
) -> Vec<Option<String>> {
    conn.auto_attach_owner_sessions_for_target_type("worker")
        .into_iter()
        .filter(|owner_session_id| {
            dedicated_worker_auto_attach_owner_session_allowed(
                conn,
                owner_session_id.as_deref(),
                owner_page,
            )
        })
        .collect()
}

pub(super) fn dedicated_worker_main_script_page_extra_events(
    request_id: &str,
    sessions: &[Option<String>],
    outcome: &crate::conn::DedicatedWorkerMainScriptOutcome,
) -> Vec<(Option<String>, Vec<BackgroundProtocolEvent>)> {
    let response = match outcome {
        crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(response) => Some(response.as_ref()),
        crate::conn::DedicatedWorkerMainScriptOutcome::Failed { response, .. } => {
            response.as_deref()
        }
    };
    let Some(response) = response else {
        return Vec::new();
    };
    let has_network_extra_info =
        response.network_request_headers().is_some() || response.request_cookie_report.is_some();
    if !has_network_extra_info {
        return Vec::new();
    }
    let default_cookie_report = moli_cookie_jar::StoredCookieQueryReport::default();
    let cookie_report = response
        .request_cookie_report
        .as_ref()
        .unwrap_or(&default_cookie_report);
    sessions
        .iter()
        .map(|session_id| {
            let mut events = Vec::new();
            network::emit_request_will_be_sent_extra_info(
                &mut events,
                session_id.as_deref(),
                request_id,
                response.network_request_headers().unwrap_or_default(),
                cookie_report,
                monotonic_timestamp_seconds(),
            );
            network::emit_response_received_extra_info(
                &mut events,
                session_id.as_deref(),
                request_id,
                &response.headers,
                response.status,
                &response.cookie_set_reports,
            );
            (session_id.clone(), events)
        })
        .collect()
}

pub(super) fn dedicated_worker_main_script_worker_events(
    target_id: &str,
    session_id: Option<&str>,
    script: &crate::conn::DedicatedWorkerMainScriptSnapshot,
) -> Vec<BackgroundProtocolEvent> {
    let mut events = Vec::new();
    let timestamp = monotonic_timestamp_seconds();
    let response = match &script.outcome {
        crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(response) => Some(response.as_ref()),
        crate::conn::DedicatedWorkerMainScriptOutcome::Failed { response, .. } => {
            response.as_deref()
        }
    };
    if let Some(response) = response {
        let has_extra_info = response.network_request_headers().is_some()
            || response.request_cookie_report.is_some();
        network::emit_response_received_without_extra_info_event(
            &mut events,
            session_id,
            target_id,
            target_id,
            target_id,
            timestamp,
            &response.final_url,
            response.status,
            None,
            &response.headers,
            response.body_bytes().len(),
            response.from_cache,
            response.negotiated_http_version,
            has_extra_info,
            DevToolsNetworkResourceType::Script,
        );
    }
    match &script.outcome {
        crate::conn::DedicatedWorkerMainScriptOutcome::Loaded(response) => {
            network::emit_loading_finished(
                &mut events,
                session_id,
                target_id,
                target_id,
                target_id,
                timestamp,
                response.body_bytes().len(),
                DevToolsNetworkResourceType::Script,
            );
        }
        crate::conn::DedicatedWorkerMainScriptOutcome::Failed { error_message, .. } => {
            network::emit_loading_failed(
                &mut events,
                session_id,
                target_id,
                target_id,
                target_id,
                timestamp,
                dedicated_worker_loading_error_text(error_message),
                DevToolsNetworkResourceType::Script,
            );
        }
    }
    events
}

pub(super) fn dedicated_worker_loading_error_text(error_message: &str) -> &str {
    error_message
        .split_ascii_whitespace()
        .find(|part| part.starts_with("net::ERR_"))
        .map(|part| {
            part.trim_end_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != '_'
            })
        })
        .unwrap_or("net::ERR_FAILED")
}

pub(in crate::domains) fn dedicated_worker_main_script_network_replay_for_session(
    conn: &mut CdpConnection,
    session_id: &str,
) -> Vec<BackgroundProtocolEvent> {
    let Some(crate::conn::CdpSessionRoute::DedicatedWorkerTarget {
        browser_context_id,
        target_id,
    }) = conn.session_route(Some(session_id))
    else {
        return Vec::new();
    };
    let Some(renderer_instance_id) = conn
        .browser_context_by_id(&browser_context_id)
        .and_then(|context| context.dedicated_worker_target(&target_id))
        .map(|target| target.renderer_instance_id)
    else {
        return Vec::new();
    };
    let events = conn
        .browser_context_by_id(&browser_context_id)
        .and_then(|context| context.dedicated_worker_targets.get(&renderer_instance_id))
        .filter(|target| {
            target.network_enabled(session_id)
                && target.main_script_network_replay_allowed_for(session_id)
                && !target.main_script_was_delivered_to(session_id)
        })
        .and_then(|target| target.main_script())
        .map(|script| {
            dedicated_worker_main_script_worker_events(&target_id, Some(session_id), script)
        })
        .unwrap_or_default();
    if events.is_empty() {
        return events;
    }
    if let Some(context) = conn.browser_context_by_id_mut(&browser_context_id)
        && let Some(target) = context
            .dedicated_worker_targets
            .get_mut(&renderer_instance_id)
    {
        target.mark_main_script_delivered_to(session_id);
    }
    events
}

pub(in crate::domains) fn release_failed_dedicated_worker_target_after_debugger_resume(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> Option<Vec<BackgroundProtocolEvent>> {
    let session_id = session_id?;
    let crate::conn::CdpSessionRoute::DedicatedWorkerTarget {
        browser_context_id,
        target_id,
    } = conn.session_route(Some(session_id))?
    else {
        return None;
    };
    let renderer_instance_id = {
        let target = conn
            .browser_context_by_id_mut(&browser_context_id)?
            .dedicated_worker_target_mut(&target_id)?;
        if !target.release_deferred_renderer_destroyed_for_debugger_resume() {
            return None;
        }
        target.renderer_instance_id
    };
    let outputs = prepare_dedicated_worker_target_retirement(
        conn,
        &browser_context_id,
        renderer_instance_id,
        DedicatedWorkerRetirementCause::OwnerRetired,
    );
    Some(commit_failed_dedicated_worker_retirement_sync(
        conn, outputs,
    ))
}

pub(in crate::domains) async fn retire_dedicated_worker_targets_for_replaced_page_async(
    conn: &mut CdpConnection,
    replaced_page_owner: &TargetPageResidenceIdentity,
) -> Vec<BackgroundProtocolEvent> {
    let renderer_instance_ids = conn
        .browser_context_by_id(replaced_page_owner.browser_context_id())
        .map(|context| {
            context
                .dedicated_worker_targets
                .iter()
                .filter_map(|(renderer_instance_id, target)| {
                    (&target.owner_page == replaced_page_owner).then_some(*renderer_instance_id)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut events = Vec::new();
    for renderer_instance_id in renderer_instance_ids {
        let outputs = prepare_dedicated_worker_target_retirement(
            conn,
            replaced_page_owner.browser_context_id(),
            renderer_instance_id,
            DedicatedWorkerRetirementCause::OwnerRetired,
        );
        for output in outputs.worker_target_lifecycle_outputs {
            match commit_dedicated_worker_retirement_output_async(conn, output).await {
                Ok(output_events) => events.extend(output_events),
                Err(output) => {
                    debug_assert!(
                        false,
                        "Page replacement DedicatedWorker retirement contained non-terminal output: {output:?}"
                    );
                }
            }
        }
    }
    events
}

pub(super) async fn commit_dedicated_worker_retirement_output_async(
    conn: &mut CdpConnection,
    output: WorkerTargetLifecycleOutput,
) -> Result<Vec<BackgroundProtocolEvent>, WorkerTargetLifecycleOutput> {
    let mut events = Vec::new();
    match output {
        WorkerTargetLifecycleOutput::DedicatedWorkerDetached {
            target_delta,
            cleanup_plan,
        } => {
            if let Some(target_delta) = target_delta {
                events.extend(
                    conn.prepared_target_info_changed_event_plan_for_discovery_owners(target_delta),
                );
            }
            let mut response_events = Vec::new();
            let outcome = super::super::session_disposal::dispose_dedicated_worker_session_after_prepared_state_delta_async(
                conn,
                &mut events,
                &mut response_events,
                cleanup_plan,
            )
            .await
            .expect("retired dedicated-worker session cleanup should succeed");
            let (event_plan, predecessor) = outcome.into_parts();
            debug_assert!(
                predecessor.is_none(),
                "DedicatedWorker disposal cannot publish a Page renderer fence"
            );
            events.extend(response_events);
            events.extend(event_plan);
        }
        WorkerTargetLifecycleOutput::DedicatedWorkerDestroyed {
            browser_context_id,
            renderer_instance_id,
            target_id,
            target_delta,
        } => {
            let removed = conn
                .browser_context_by_id_mut(&browser_context_id)
                .and_then(|context| {
                    let target = context
                        .dedicated_worker_targets
                        .get(&renderer_instance_id)?;
                    (target.target_id == target_id).then(|| {
                        context.remove_dedicated_worker_target_by_renderer_instance(
                            renderer_instance_id,
                        )
                    })
                })
                .flatten();
            if removed.is_none() {
                return Ok(events);
            }
            if let Some(target_delta) = target_delta {
                events.extend(conn.prepared_target_host_delta_event_plan(target_delta));
            }
        }
        output => return Err(output),
    }
    Ok(events)
}

pub(super) fn commit_failed_dedicated_worker_retirement_sync(
    conn: &mut CdpConnection,
    outputs: TargetPreparedOutputs,
) -> Vec<BackgroundProtocolEvent> {
    let retirement_identity = outputs
        .worker_target_lifecycle_outputs
        .iter()
        .find_map(|output| match output {
            WorkerTargetLifecycleOutput::DedicatedWorkerDestroyed {
                browser_context_id,
                renderer_instance_id,
                target_id,
                ..
            } => Some((
                browser_context_id.clone(),
                *renderer_instance_id,
                target_id.clone(),
            )),
            _ => None,
        });
    let Some((browser_context_id, renderer_instance_id, target_id)) = retirement_identity else {
        return Vec::new();
    };
    let removed = conn
        .browser_context_by_id_mut(&browser_context_id)
        .and_then(|context| {
            let target = context
                .dedicated_worker_targets
                .get(&renderer_instance_id)?;
            (target.target_id == target_id).then(|| {
                context.remove_dedicated_worker_target_by_renderer_instance(renderer_instance_id)
            })
        })
        .flatten();
    if removed.is_none() {
        return Vec::new();
    }
    let mut events = Vec::new();
    for output in outputs.worker_target_lifecycle_outputs {
        match output {
            WorkerTargetLifecycleOutput::DedicatedWorkerDetached {
                target_delta,
                cleanup_plan,
            } => {
                if let Some(target_delta) = target_delta {
                    events.extend(
                        conn.prepared_target_info_changed_event_plan_for_discovery_owners(
                            target_delta,
                        ),
                    );
                }
                events.extend(
                    super::super::session_disposal::dispose_removed_dedicated_worker_session_after_failed_retirement(
                        conn,
                        cleanup_plan,
                    ),
                );
            }
            WorkerTargetLifecycleOutput::DedicatedWorkerDestroyed { target_delta, .. } => {
                if let Some(target_delta) = target_delta {
                    events.extend(conn.prepared_target_host_delta_event_plan(target_delta));
                }
            }
            output => {
                debug_assert!(
                    false,
                    "failed DedicatedWorker retirement contained non-terminal output: {output:?}"
                );
            }
        }
    }
    events
}

pub(super) fn record_dedicated_worker_target_runtime_inspector_messages(
    conn: &CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: u64,
    inspector_session_id: Option<String>,
    messages: Vec<RendererRuntimeInspectorMessage>,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some(target) = conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| context.dedicated_worker_targets.get(&renderer_instance_id))
    else {
        return outputs;
    };
    let target_id = target.target_id.clone();
    let session_ids = if let Some(session_id) = inspector_session_id {
        target
            .is_session(&session_id)
            .then_some(vec![session_id])
            .unwrap_or_default()
    } else {
        target.session_ids()
    };
    for session_id in session_ids {
        outputs.push(
            WorkerTargetLifecycleOutput::DedicatedWorkerRuntimeInspectorMessages {
                browser_context_id: browser_context_id.to_owned(),
                renderer_instance_id,
                target_id: target_id.clone(),
                session_id,
                messages: messages.clone(),
            },
        );
    }
    outputs
}

pub(super) fn record_dedicated_worker_target_console_message(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: u64,
    message: RendererSharedWorkerConsoleMessage,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    let Some(context) = conn.browser_context_by_id_mut(browser_context_id) else {
        return outputs;
    };
    let Some(target) = context
        .dedicated_worker_targets
        .get_mut(&renderer_instance_id)
    else {
        return outputs;
    };
    target.record_console_message(message);
    let target_id = target.target_id.clone();
    let console_end = target.console_message_count();
    for session_id in target.session_ids() {
        let console_messages = target.pending_console_domain_messages(&session_id).to_vec();
        let runtime_messages = target
            .pending_runtime_console_messages(&session_id)
            .to_vec();
        if console_messages.is_empty() && runtime_messages.is_empty() {
            continue;
        }
        outputs.push(
            WorkerTargetLifecycleOutput::DedicatedWorkerConsoleMessages {
                browser_context_id: browser_context_id.to_owned(),
                renderer_instance_id,
                target_id: target_id.clone(),
                session_id,
                console_messages,
                runtime_messages,
                console_end,
            },
        );
    }
    outputs
}

pub(super) fn prepare_dedicated_worker_target_retirement(
    conn: &mut CdpConnection,
    browser_context_id: &str,
    renderer_instance_id: u64,
    cause: DedicatedWorkerRetirementCause,
) -> TargetPreparedOutputs {
    let mut outputs = TargetPreparedOutputs::default();
    if matches!(cause, DedicatedWorkerRetirementCause::RendererDestroyed)
        && conn
            .browser_context_by_id_mut(browser_context_id)
            .and_then(|context| {
                context
                    .dedicated_worker_targets
                    .get_mut(&renderer_instance_id)
            })
            .is_some_and(|target| target.defer_renderer_destroyed_for_debugger_resume())
    {
        return outputs;
    }
    let target_id = match conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| {
            context.dedicated_worker_target_id_for_renderer_instance(renderer_instance_id)
        }) {
        Some(target_id) => target_id.to_owned(),
        None => return outputs,
    };
    let destroyed_delta = conn
        .has_any_target_discovery()
        .then(|| conn.prepare_destroyed_target_host_delta(&target_id))
        .flatten();
    let mut detached_delta = conn
        .browser_context_by_id(browser_context_id)
        .and_then(|context| {
            let target = context.dedicated_worker_target(&target_id)?;
            if !target.has_session() {
                return None;
            }
            let mut target_info = context.devtools_target_info(&target_id)?;
            target_info.attached = false;
            Some(PreparedTargetHostDelta::info_changed(
                target_id.clone(),
                Some(target_info),
            ))
        });
    let Some(context) = conn.browser_context_by_id(browser_context_id) else {
        return outputs;
    };
    let Some(target) = context.dedicated_worker_targets.get(&renderer_instance_id) else {
        return outputs;
    };
    for session_id in target.session_ids() {
        outputs.push(WorkerTargetLifecycleOutput::DedicatedWorkerDetached {
            target_delta: detached_delta.take(),
            cleanup_plan: TargetSessionDetachCleanupPlan::new(
                target_id.clone(),
                session_id,
                None,
                None,
            ),
        });
    }
    outputs.push(WorkerTargetLifecycleOutput::DedicatedWorkerDestroyed {
        browser_context_id: browser_context_id.to_owned(),
        renderer_instance_id,
        target_id,
        target_delta: destroyed_delta,
    });
    outputs
}
