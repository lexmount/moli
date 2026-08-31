use serde::Deserialize;

use crate::conn::{CdpTargetFilter, CdpTargetFilterEntry, PreparedTargetAttach};

use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AutoAttachParams {
    auto_attach: bool,
    #[serde(rename = "waitForDebuggerOnStart")]
    wait_for_debugger_on_start: bool,
    flatten: Option<bool>,
    filter: Option<Vec<TargetFilterEntry>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AutoAttachRelatedParams {
    target_id: String,
    #[serde(rename = "waitForDebuggerOnStart")]
    wait_for_debugger_on_start: bool,
    filter: Option<Vec<TargetFilterEntry>>,
}

#[derive(Deserialize)]
struct TargetFilterEntry {
    #[serde(default)]
    exclude: bool,
    #[serde(rename = "type")]
    target_type: Option<String>,
}

#[derive(Debug)]
struct ServiceWorkerAutoAttachRelatedTarget {
    target_id: String,
    browser_context_id: String,
    registration_id: u64,
    version_id: u64,
    script_url: String,
    scope_url: String,
}

pub(super) fn auto_attach_related(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params: AutoAttachRelatedParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        _ => {
            return CommandOutputPlan::error_without_session(-32602, "InvalidParams");
        }
    };
    if cmd.session_id.is_some() && !conn.is_browser_session_id(cmd.session_id) {
        return CommandOutputPlan::error_without_session(
            -32000,
            "Target.autoAttachRelated is only supported on the Browser target",
        );
    }

    let restore_browser_context_id = previously_active_browser_context_id(conn);
    let target = match service_worker_auto_attach_related_target(conn, params.target_id.as_str()) {
        Ok(target) => target,
        Err(plan) => {
            restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
            return plan;
        }
    };
    let allow_service_worker_targets =
        cdp_target_filter_from_entries(params.filter).matches("service_worker");
    conn.replace_service_worker_auto_attach_related_owner(
        cmd.session_id,
        &target.browser_context_id,
        target.registration_id,
        target.version_id,
        target.script_url,
        target.scope_url,
        allow_service_worker_targets,
        params.wait_for_debugger_on_start,
    );
    let mut plan = CommandOutputPlan::default();
    if allow_service_worker_targets
        && !owner_already_auto_attached_to_target(conn, cmd.session_id, &target.target_id)
        && let Some((session_id, target_info)) =
            attach_service_worker_target_for_auto_attach_related(conn, &target.target_id)
    {
        let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
            &target.target_id,
            target_info,
            [conn.prepare_auto_attach_session_commit(
                session_id,
                cmd.session_id.map(str::to_owned),
                false,
            )],
        ));
        for event in event_plan {
            plan.push_background_event(event);
        }
    }
    restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
    // Chromium completes autoAttachRelated only after its existing-target
    // attach pass. Existing targets are already running, so they cannot be
    // paused on start even when the policy requests it.
    plan.push_success();
    plan
}

fn service_worker_auto_attach_related_target(
    conn: &mut CdpConnection,
    target_id: &str,
) -> Result<ServiceWorkerAutoAttachRelatedTarget, CommandOutputPlan> {
    if let Err(message) = select_browser_context_for_target(conn, target_id) {
        return Err(CommandOutputPlan::error_without_session(-31998, message));
    }
    let Some(browser_context) = conn.browser_context.as_ref() else {
        return Err(CommandOutputPlan::error_without_session(
            -31998,
            "BrowserContextNotLoaded",
        ));
    };
    let Some(target) = browser_context.service_worker_target(target_id) else {
        return Err(CommandOutputPlan::error_without_session(
            -32000,
            "Target does not support auto-attaching",
        ));
    };
    Ok(ServiceWorkerAutoAttachRelatedTarget {
        target_id: target.target_id.clone(),
        browser_context_id: browser_context.id.clone(),
        registration_id: target.renderer_registration_id,
        version_id: target.renderer_version_id,
        script_url: target.script_url.clone(),
        scope_url: target.scope_url.clone(),
    })
}

fn attach_service_worker_target_for_auto_attach_related(
    conn: &mut CdpConnection,
    target_id: &str,
) -> Option<(String, DevToolsTargetInfo)> {
    let session_id = conn.gen_session_id();
    let target_info = conn
        .prepare_auto_attached_service_worker_session_binding_info(target_id, session_id.clone())?;
    Some((session_id, target_info))
}

fn owner_already_auto_attached_to_target(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    target_id: &str,
) -> bool {
    conn.auto_attached_sessions_for_owner(owner_session_id)
        .into_iter()
        .any(|session_id| {
            matches!(
                conn.session_route(Some(&session_id)),
                Some(crate::conn::CdpSessionRoute::ServiceWorkerTarget {
                    target_id: attached_target_id,
                    ..
                }) if attached_target_id == target_id
            )
        })
}

fn owner_already_auto_attached_to_exact_target(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    target_id: &str,
) -> bool {
    let attached_sessions = conn.attached_sessions_for_target(target_id);
    conn.auto_attached_sessions_for_owner(owner_session_id)
        .iter()
        .any(|session_id| attached_sessions.contains(session_id))
}

fn owner_already_auto_attached_to_page_target(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    target_id: &str,
) -> bool {
    conn.auto_attached_sessions_for_owner(owner_session_id)
        .into_iter()
        .any(|session_id| match conn.session_route(Some(&session_id)) {
            Some(crate::conn::CdpSessionRoute::PageTarget {
                target_id: route_target_id,
                ..
            }) => route_target_id == target_id,
            _ => false,
        })
}

fn owner_already_auto_attached_to_tab_target(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    tab_target_id: &str,
) -> bool {
    conn.auto_attached_sessions_for_owner(owner_session_id)
        .into_iter()
        .any(|session_id| {
            matches!(
                conn.session_route(Some(&session_id)),
                Some(crate::conn::CdpSessionRoute::TabTarget {
                    tab_target_id: attached_target_id,
                    ..
                }) if attached_target_id == tab_target_id
            )
        })
}

fn should_auto_attach_page_target_for_owner(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    target_id: &str,
    target_has_primary_session: bool,
) -> bool {
    if owner_session_id.is_none() && target_has_primary_session {
        return false;
    }
    if owner_session_is_attached_to_page_target(conn, owner_session_id, target_id) {
        return false;
    }
    !owner_already_auto_attached_to_page_target(conn, owner_session_id, target_id)
}

fn should_auto_attach_tab_target_for_owner(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    tab_target_id: &str,
) -> bool {
    if owner_session_id.is_none()
        && conn
            .primary_session_id_for_tab_target_id(tab_target_id)
            .is_some()
    {
        return false;
    }
    !owner_already_auto_attached_to_tab_target(conn, owner_session_id, tab_target_id)
}

fn owner_session_is_attached_to_page_target(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    target_id: &str,
) -> bool {
    let Some(owner_session_id) = owner_session_id else {
        return false;
    };
    page_session_route_matches_target(conn, owner_session_id, target_id)
}

fn page_session_route_matches_target(
    conn: &CdpConnection,
    session_id: &str,
    target_id: &str,
) -> bool {
    match conn.session_route(Some(session_id)) {
        Some(crate::conn::CdpSessionRoute::PageTarget {
            target_id: route_target_id,
            ..
        }) => route_target_id == target_id,
        _ => false,
    }
}

fn cdp_target_filter_from_entries(filter: Option<Vec<TargetFilterEntry>>) -> CdpTargetFilter {
    match filter {
        Some(entries) => CdpTargetFilter::from_entries(
            entries
                .into_iter()
                .map(|entry| CdpTargetFilterEntry {
                    exclude: entry.exclude,
                    target_type: entry.target_type,
                })
                .collect(),
        ),
        None => CdpTargetFilter::default_auto_attach(),
    }
}

pub(super) fn start_set_auto_attach_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> TargetCommandTaskStep {
    let params: AutoAttachParams = match cmd.get_params() {
        Ok(Some(p)) => p,
        _ => {
            return super::target_command_error(-32602, "InvalidParams");
        }
    };
    if cmd
        .session_id
        .is_some_and(|session_id| conn.is_browser_session_id(Some(session_id)))
        && params.flatten != Some(true)
    {
        return super::target_command_error(
            -32602,
            "Only flatten protocol is supported with browser level auto-attach",
        );
    }
    if !params.auto_attach
        && params
            .filter
            .as_ref()
            .is_some_and(|filter| !filter.is_empty())
    {
        return super::target_command_error(
            -32602,
            "Target filter should be empty when disabling auto-attach",
        );
    }
    let target_filter = cdp_target_filter_from_entries(params.filter);
    let owner_is_browser_or_root =
        cmd.session_id.is_none() || conn.is_browser_session_id(cmd.session_id);
    if params.auto_attach
        && owner_is_browser_or_root
        && target_filter.matches("tab")
        && target_filter.matches("page")
    {
        return super::target_command_error(
            -32602,
            "Filter should not simultaneously allow \"tab\" and \"page\", page targets are attached via tab targets",
        );
    }
    let owner_was_enabled = conn.has_auto_attach_owner(cmd.session_id);
    let legacy_disable_all = !params.auto_attach
        && !owner_was_enabled
        && conn.auto_attach_owner_count() == 0
        && conn.auto_attach;
    if params.auto_attach {
        conn.install_default_browser_target_for_auto_attach_if_enabled();
    }
    let pause_service_workers_on_start = params.auto_attach
        && params.wait_for_debugger_on_start
        && target_filter.matches("service_worker")
        && super::browser_level_auto_attach_owner_session_allowed(conn, cmd.session_id);
    let pause_dedicated_workers_on_start =
        params.auto_attach && params.wait_for_debugger_on_start && target_filter.matches("worker");
    conn.set_auto_attach_owner(
        cmd.session_id,
        params.auto_attach,
        params.wait_for_debugger_on_start,
        target_filter,
    );
    super::set_service_worker_pause_on_start_owner(
        conn,
        cmd.session_id,
        pause_service_workers_on_start,
    );
    super::set_dedicated_worker_pause_on_start_owner(
        conn,
        cmd.session_id,
        pause_dedicated_workers_on_start,
    );
    pending_set_auto_attach_command(
        cmd.id,
        cmd.session_id,
        params.auto_attach,
        cmd.session_id,
        legacy_disable_all,
    )
}

pub(super) async fn complete_set_auto_attach_command_async(
    conn: &mut CdpConnection,
    auto_attach: bool,
    owner_session_id: Option<&str>,
    legacy_disable_all: bool,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> CommandOutputPlan {
    let mut side_effects = events::TargetProtocolSideEffects::default();
    let restore_browser_context_id = previously_active_browser_context_id(conn);
    set_auto_attach_inner_async(
        conn,
        &mut side_effects,
        auto_attach,
        owner_session_id,
        legacy_disable_all,
        command_context,
    )
    .await;
    restore_previously_active_browser_context(conn, restore_browser_context_id.as_deref());
    // Chromium runs the AddClient existing-target sweep before invoking the
    // SetAutoAttach completion callback. Puppeteer consumes attachedToTarget
    // during that sweep and assumes every pre-existing session is registered
    // by the time the command response resolves.
    let mut plan = side_effects.into_plan();
    plan.push_success();
    plan
}

async fn set_auto_attach_inner_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    auto_attach: bool,
    owner_session_id: Option<&str>,
    legacy_disable_all: bool,
    command_context: &mut crate::conn::CommandDispatchContext,
) {
    if !auto_attach && !legacy_disable_all {
        detach_auto_attached_sessions_for_owner_async(conn, out, owner_session_id, command_context)
            .await;
        return;
    }
    if auto_attach
        && let Some(owner_session_id) = owner_session_id
        && let Some(tab_target_id) = conn
            .tab_target_id_for_session_id(owner_session_id)
            .map(str::to_owned)
    {
        auto_attach_child_page_for_tab_session_async(conn, out, owner_session_id, &tab_target_id)
            .await;
        return;
    }

    let context_ids: Vec<String> = conn.browser_contexts().map(|bc| bc.id.clone()).collect();
    for context_id in context_ids {
        if !conn.activate_browser_context_by_id_async(&context_id).await {
            continue;
        }
        if auto_attach {
            let attach_page_targets = conn
                .auto_attach_owner_allows_target_type(owner_session_id, "page")
                && super::browser_level_auto_attach_owner_session_allowed(conn, owner_session_id);
            let attach_tab_targets = conn
                .auto_attach_owner_allows_target_type(owner_session_id, "tab")
                && super::browser_level_auto_attach_owner_session_allowed(conn, owner_session_id);
            let attach_shared_worker_targets = conn
                .auto_attach_owner_allows_target_type(owner_session_id, "shared_worker")
                && super::browser_level_auto_attach_owner_session_allowed(conn, owner_session_id);
            let attach_dedicated_worker_targets =
                conn.auto_attach_owner_allows_target_type(owner_session_id, "worker");
            let attach_service_worker_targets = conn
                .auto_attach_owner_allows_target_type(owner_session_id, "service_worker")
                && super::browser_level_auto_attach_owner_session_allowed(conn, owner_session_id);
            // waitForDebuggerOnStart applies to targets created after the
            // policy is installed. Chromium reports every target found by the
            // initial AddClient sweep as already running.
            let waiting_for_debugger = false;
            let pending_attach_target_ids = {
                let bc = conn
                    .browser_context
                    .as_ref()
                    .expect("browser context must exist when attaching existing targets");
                if !attach_page_targets {
                    Vec::new()
                } else {
                    let mut target_ids = Vec::new();
                    if let Some(target_id) = bc.active_target_id_owned()
                        && should_auto_attach_page_target_for_owner(
                            conn,
                            owner_session_id,
                            &target_id,
                            bc.has_active_session(),
                        )
                    {
                        target_ids.push(target_id);
                    }
                    target_ids.extend(
                        bc.background_targets()
                            .filter(|target| {
                                should_auto_attach_page_target_for_owner(
                                    conn,
                                    owner_session_id,
                                    target.target_id(),
                                    target.has_session(),
                                )
                            })
                            .map(|target| target.target_id().to_owned()),
                    );
                    target_ids
                }
            };
            let pending_attach_tab_target_ids = {
                let bc = conn
                    .browser_context
                    .as_ref()
                    .expect("browser context must exist when attaching existing tab targets");
                if !attach_tab_targets {
                    Vec::new()
                } else {
                    let mut tab_target_ids = Vec::new();
                    if let Some(page_target_id) = bc.active_target_id()
                        && let Some(tab_target_id) =
                            conn.tab_target_id_for_page_target_id(page_target_id)
                        && should_auto_attach_tab_target_for_owner(
                            conn,
                            owner_session_id,
                            tab_target_id,
                        )
                    {
                        tab_target_ids.push(tab_target_id.to_owned());
                    }
                    tab_target_ids.extend(bc.background_targets().filter_map(|target| {
                        let tab_target_id =
                            conn.tab_target_id_for_page_target_id(target.target_id())?;
                        should_auto_attach_tab_target_for_owner(
                            conn,
                            owner_session_id,
                            tab_target_id,
                        )
                        .then(|| tab_target_id.to_owned())
                    }));
                    tab_target_ids
                }
            };
            let pending_attach_shared_worker_target_ids = {
                let bc = conn
                    .browser_context
                    .as_ref()
                    .expect("browser context must exist when attaching existing targets");
                if attach_shared_worker_targets {
                    bc.shared_worker_targets
                        .values()
                        .filter(|target| {
                            !owner_already_auto_attached_to_exact_target(
                                conn,
                                owner_session_id,
                                &target.target_id,
                            )
                        })
                        .map(|target| target.target_id.clone())
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            };
            let pending_attach_service_worker_target_ids = {
                let bc = conn
                    .browser_context
                    .as_ref()
                    .expect("browser context must exist when attaching existing targets");
                if attach_service_worker_targets {
                    bc.service_worker_targets
                        .values()
                        .filter(|target| {
                            !owner_already_auto_attached_to_exact_target(
                                conn,
                                owner_session_id,
                                &target.target_id,
                            )
                        })
                        .map(|target| target.target_id.clone())
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            };
            let pending_attach_dedicated_worker_target_ids = {
                let bc = conn
                    .browser_context
                    .as_ref()
                    .expect("browser context must exist when attaching existing targets");
                if attach_dedicated_worker_targets {
                    bc.dedicated_worker_targets
                        .values()
                        .filter(|target| {
                            super::worker_target::dedicated_worker_auto_attach_owner_session_allowed(
                                conn,
                                owner_session_id,
                                &target.owner_page,
                            ) && !owner_already_auto_attached_to_exact_target(
                                conn,
                                owner_session_id,
                                &target.target_id,
                            )
                        })
                        .map(|target| target.target_id.clone())
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            };
            let attached_targets = pending_attach_target_ids
                .into_iter()
                .map(|target_id| (target_id, conn.gen_session_id()))
                .collect::<Vec<_>>();
            let attached_tab_targets = pending_attach_tab_target_ids
                .into_iter()
                .map(|target_id| (target_id, conn.gen_session_id()))
                .collect::<Vec<_>>();
            let attached_shared_worker_targets = pending_attach_shared_worker_target_ids
                .into_iter()
                .map(|target_id| (target_id, conn.gen_session_id()))
                .collect::<Vec<_>>();
            let attached_service_worker_targets = pending_attach_service_worker_target_ids
                .into_iter()
                .map(|target_id| (target_id, conn.gen_session_id()))
                .collect::<Vec<_>>();
            let attached_dedicated_worker_targets = pending_attach_dedicated_worker_target_ids
                .into_iter()
                .map(|target_id| (target_id, conn.gen_session_id()))
                .collect::<Vec<_>>();
            let promote_target_id = {
                let bc = conn
                    .browser_context
                    .as_ref()
                    .expect("browser context must exist when considering auto-attach promotion");
                if !attach_page_targets || bc.has_loaded_page() {
                    None
                } else {
                    bc.background_targets()
                        .rev()
                        .find(|target| !target.has_session() && target.has_loaded_page())
                        .map(|target| target.target_id().to_owned())
                        .or_else(|| {
                            bc.background_targets()
                                .find(|target| !target.has_session())
                                .map(|target| target.target_id().to_owned())
                        })
                }
            };
            {
                for (target_id, session_id) in &attached_targets {
                    let assigned = conn
                        .prepare_auto_attached_page_session_binding(target_id, session_id.clone());
                    debug_assert!(assigned, "attached target must remain addressable");
                }
                for (target_id, session_id) in &attached_shared_worker_targets {
                    let assigned = conn.prepare_auto_attached_shared_worker_session_binding(
                        target_id,
                        session_id.clone(),
                    );
                    debug_assert!(
                        assigned,
                        "attached shared worker target must remain addressable"
                    );
                }
                for (target_id, session_id) in &attached_dedicated_worker_targets {
                    let assigned = conn.prepare_auto_attached_dedicated_worker_session_binding(
                        target_id,
                        session_id.clone(),
                    );
                    debug_assert!(
                        assigned,
                        "attached dedicated worker target must remain addressable"
                    );
                }
                for (target_id, session_id) in &attached_service_worker_targets {
                    let assigned = conn.prepare_auto_attached_service_worker_session_binding(
                        target_id,
                        session_id.clone(),
                    );
                    debug_assert!(
                        assigned,
                        "attached service worker target must remain addressable"
                    );
                }
            }
            for (target_id, session_id) in &attached_tab_targets {
                let assigned = conn.prepare_auto_attached_tab_session_binding(
                    target_id,
                    session_id.clone(),
                    owner_session_id,
                );
                debug_assert!(assigned, "attached tab target must remain addressable");
            }
            if let Some(promote_target_id) = promote_target_id {
                match conn
                    .promote_background_target_to_active_for_connection_async(&promote_target_id)
                    .await
                {
                    Ok(Some(activation)) => {
                        out.extend_background_events(activation.into_protocol_events());
                    }
                    Ok(None) => {}
                    Err(message) => {
                        panic!(
                            "same-context target should remain promotable during auto-attach: {message}"
                        );
                    }
                }
            }
            ensure_initial_document_for_attached_page_targets_async(conn, &attached_targets).await;
            for (target_id, session_id) in &attached_targets {
                if let Err(message) = conn
                    .apply_runtime_binding_state_for_session_owner_async(Some(session_id))
                    .await
                    && message != "NoDocumentLoaded"
                {
                    tracing::warn!(
                        %message,
                        target_id = target_id.as_str(),
                        session_id = session_id.as_str(),
                        "failed to apply renderer binding state during target auto-attach"
                    );
                }
            }
            for (target_id, session_id) in attached_tab_targets {
                let ti = conn
                    .tab_target_info(&target_id)
                    .expect("attached tab target must remain addressable");
                let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
                    &target_id,
                    ti,
                    [conn.prepare_auto_attach_session_commit(
                        session_id,
                        owner_session_id.map(str::to_owned),
                        waiting_for_debugger,
                    )],
                ));
                out.extend_background_events(event_plan);
            }
            for (target_id, session_id) in attached_targets {
                let ti = {
                    let bc = conn
                        .browser_context
                        .as_ref()
                        .expect("browser context must exist when emitting attach events");
                    bc.devtools_target_info(&target_id)
                        .expect("attached target must remain addressable")
                };
                if let Some(message) =
                    super::transient_no_page_devtools_target_info_error(conn, &ti)
                {
                    warn_target_protocol_side_effect_failure(
                        &target_id,
                        "emit_attached_to_target",
                        &message,
                    );
                }
                let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
                    &target_id,
                    ti,
                    [conn.prepare_auto_attach_session_commit(
                        session_id,
                        owner_session_id.map(str::to_owned),
                        waiting_for_debugger,
                    )],
                ));
                out.extend_background_events(event_plan);
            }
            for (target_id, session_id) in attached_shared_worker_targets {
                let ti = {
                    let bc = conn
                        .browser_context
                        .as_ref()
                        .expect("browser context must exist when emitting attach events");
                    bc.devtools_target_info(&target_id)
                        .expect("attached shared worker target must remain addressable")
                };
                let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
                    &target_id,
                    ti,
                    [conn.prepare_auto_attach_session_commit(
                        session_id,
                        owner_session_id.map(str::to_owned),
                        waiting_for_debugger,
                    )],
                ));
                out.extend_background_events(event_plan);
            }
            for (target_id, session_id) in attached_dedicated_worker_targets {
                let ti = {
                    let bc = conn
                        .browser_context
                        .as_ref()
                        .expect("browser context must exist when emitting attach events");
                    bc.devtools_target_info(&target_id)
                        .expect("attached dedicated worker target must remain addressable")
                };
                let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
                    &target_id,
                    ti,
                    [conn.prepare_auto_attach_session_commit(
                        session_id,
                        owner_session_id.map(str::to_owned),
                        waiting_for_debugger,
                    )],
                ));
                out.extend_background_events(event_plan);
            }
            for (target_id, session_id) in attached_service_worker_targets {
                let ti = {
                    let bc = conn
                        .browser_context
                        .as_ref()
                        .expect("browser context must exist when emitting attach events");
                    bc.devtools_target_info(&target_id)
                        .expect("attached service worker target must remain addressable")
                };
                let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
                    &target_id,
                    ti,
                    [conn.prepare_auto_attach_session_commit(
                        session_id,
                        owner_session_id.map(str::to_owned),
                        waiting_for_debugger,
                    )],
                ));
                out.extend_background_events(event_plan);
            }
            continue;
        }

        if let Some((target_id, Some(session_id))) = conn
            .browser_context
            .as_ref()
            .and_then(BrowserContext::active_target_identity)
        {
            events::fail_pending_fetch_state_for_target_background_events_async(
                conn,
                out.background_events_mut(),
                Some(&session_id),
                "Target detached",
            )
            .await;

            let event_plan = conn
                .detach_active_target_session_binding_event_plan_async(
                    crate::conn::TargetSessionDetachCleanupPlan::new(
                        target_id, session_id, None, None,
                    ),
                )
                .await
                .expect("clearing session-scoped state during auto-attach reset should succeed");
            out.extend_background_events(event_plan);
        }

        let detached_background_targets =
            conn.background_target_session_detach_cleanup_plans(None, None);
        for cleanup_plan in detached_background_targets {
            if let Some(event_plan) = conn
                .detach_background_target_session_binding_event_plan_async(cleanup_plan)
                .await
                .expect("clearing background target session-scoped state during auto-attach reset should succeed")
            {
                out.extend_background_events(event_plan);
            }
        }

        let shared_worker_sessions_to_release = conn
            .browser_context
            .as_ref()
            .map(|bc| {
                bc.shared_worker_targets
                    .values()
                    .flat_map(|target| target.session_ids())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for session_id in &shared_worker_sessions_to_release {
            conn.release_shared_worker_runtime_remote_objects_for_session_best_effort_async(
                session_id,
            )
            .await;
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                Some(session_id),
                "Target detached",
            );
        }

        let shared_worker_event_plan = conn
            .detach_all_shared_worker_target_sessions_event_plan_async(None, None)
            .await;
        out.extend_background_events(shared_worker_event_plan);

        let dedicated_worker_sessions_to_release = conn
            .browser_context
            .as_ref()
            .map(|bc| {
                bc.dedicated_worker_targets
                    .values()
                    .flat_map(|target| target.session_ids())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for session_id in &dedicated_worker_sessions_to_release {
            conn.release_shared_worker_runtime_remote_objects_for_session_best_effort_async(
                session_id,
            )
            .await;
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                Some(session_id),
                "Target detached",
            );
        }
        let dedicated_worker_event_plan = conn
            .detach_all_dedicated_worker_target_sessions_event_plan_async(None, None)
            .await;
        out.extend_background_events(dedicated_worker_event_plan);

        let service_worker_sessions_to_detach = conn
            .browser_context
            .as_ref()
            .map(|bc| {
                bc.service_worker_targets
                    .values()
                    .flat_map(|target| target.session_ids())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let service_worker_event_plan = conn
            .detach_all_service_worker_target_sessions_event_plan_async(None, None)
            .await;
        for session_id in service_worker_sessions_to_detach {
            super::set_service_worker_pause_on_start_owner(conn, Some(&session_id), false);
        }
        out.extend_background_events(service_worker_event_plan);
    }
}

async fn auto_attach_child_page_for_tab_session_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    tab_session_id: &str,
    tab_target_id: &str,
) {
    if !conn.auto_attach_owner_allows_target_type(Some(tab_session_id), "page") {
        return;
    }
    let Some(page_target_id) = conn
        .primary_page_target_id_for_tab_target_id(tab_target_id)
        .map(str::to_owned)
    else {
        return;
    };
    let Some(browser_context_id) = conn.browser_context_id_for_tab_target_id(tab_target_id) else {
        return;
    };
    if !conn
        .activate_browser_context_by_id_async(&browser_context_id)
        .await
    {
        return;
    }
    let target_has_primary_session = {
        let Some(bc) = conn.browser_context.as_ref() else {
            return;
        };
        if bc.active_target_id() == Some(page_target_id.as_str()) {
            bc.has_active_session()
        } else {
            bc.background_target(&page_target_id)
                .is_some_and(|target| target.has_session())
        }
    };
    if !should_auto_attach_page_target_for_owner(
        conn,
        Some(tab_session_id),
        &page_target_id,
        target_has_primary_session,
    ) {
        return;
    }
    let session_id = conn.gen_session_id();
    let assigned =
        conn.prepare_auto_attached_page_session_binding(&page_target_id, session_id.clone());
    if !assigned {
        return;
    }
    ensure_initial_document_for_attached_page_targets_async(
        conn,
        &[(page_target_id.clone(), session_id.clone())],
    )
    .await;
    if let Err(message) = conn
        .apply_runtime_binding_state_for_session_owner_async(Some(&session_id))
        .await
        && message != "NoDocumentLoaded"
    {
        tracing::warn!(
            %message,
            target_id = page_target_id.as_str(),
            session_id = session_id.as_str(),
            "failed to apply renderer binding state during tab child page auto-attach"
        );
    }
    let prepared_session = conn.prepare_auto_attach_session_commit(
        session_id.clone(),
        Some(tab_session_id.to_owned()),
        false,
    );
    let Some(target_info) = conn
        .browser_context
        .as_ref()
        .and_then(|bc| bc.devtools_target_info(&page_target_id))
    else {
        conn.rollback_prepared_attach_session_without_event_async(&prepared_session)
            .await;
        return;
    };
    if let Some(message) = super::transient_no_page_devtools_target_info_error(conn, &target_info) {
        warn_target_protocol_side_effect_failure(
            &page_target_id,
            "emit_tab_child_attached_to_target",
            &message,
        );
        conn.rollback_prepared_attach_session_without_event_async(&prepared_session)
            .await;
        return;
    }
    let event_plan = conn.commit_prepared_attach_event_plan(PreparedTargetAttach::new(
        &page_target_id,
        target_info,
        [prepared_session],
    ));
    out.extend_background_events(event_plan);
}

pub(super) async fn detach_auto_attached_sessions_for_owner_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    owner_session_id: Option<&str>,
    command_context: &mut crate::conn::CommandDispatchContext,
) {
    let session_ids = conn.auto_attached_session_cascade_for_owner(owner_session_id);
    for session_id in session_ids {
        detach_attached_session_for_owner_async(conn, out, &session_id, command_context).await;
    }
}

pub(super) async fn detach_attached_sessions_for_owner_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    owner_session_id: Option<&str>,
    command_context: &mut crate::conn::CommandDispatchContext,
) {
    let session_ids = conn.attached_session_cascade_for_owner(owner_session_id);
    for session_id in session_ids {
        detach_attached_session_for_owner_async(conn, out, &session_id, command_context).await;
    }
}

pub(super) async fn release_attached_sessions_for_root_frontend_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    command_context: &mut crate::conn::CommandDispatchContext,
) {
    // Root-owned browser sessions include the scheduler's private page-control
    // session and must outlive a browser frontend disconnect. A target session
    // owned directly by that frontend still belongs to the release cascade.
    let session_ids = conn.attached_session_cascade_for_root_frontend();
    for session_id in session_ids {
        let detach_plan = conn.auto_attached_session_detach_plan(&session_id);
        let preserves_other_frontends = matches!(
            detach_plan.cleanup_plan().map(|plan| plan.action()),
            Some(crate::conn::TargetBindingCleanupAction::PageTarget {
                is_attached_session: false,
                ..
            })
        );
        if !preserves_other_frontends {
            detach_attached_session_for_owner_async(conn, out, &session_id, command_context).await;
            continue;
        }

        let Some(browser_context_id) = detach_plan.browser_context_id().map(str::to_owned) else {
            conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
            continue;
        };
        if !conn
            .activate_browser_context_by_id_async(&browser_context_id)
            .await
        {
            conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
            continue;
        }
        conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            out.background_events_mut(),
            command_context.protocol_events_mut(),
            Some(&session_id),
            "Target detached",
        );
        super::clear_detached_target_fetch_state_background_events_async(
            conn,
            out.background_events_mut(),
            &session_id,
        )
        .await;
        let _ = conn
            .detach_runtime_inspector_session_for_session_owner_async(Some(&session_id))
            .await;
        clear_detached_session_target_overrides_best_effort(conn, &session_id).await;
        if !conn.release_primary_target_session_binding_without_event(&session_id) {
            conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
        }
    }
}

async fn detach_attached_session_for_owner_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    session_id: &str,
    command_context: &mut crate::conn::CommandDispatchContext,
) {
    if conn.is_browser_session_id(Some(session_id)) {
        conn.cancel_tracing_for_session_owner_async(Some(session_id))
            .await;
        let detached = conn.detach_browser_session_owner_without_event(session_id);
        debug_assert!(detached.is_some());
        return;
    }
    let detach_plan = conn.auto_attached_session_detach_plan(session_id);
    let Some(browser_context_id) = detach_plan.browser_context_id().map(str::to_owned) else {
        conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
        return;
    };
    if !conn
        .activate_browser_context_by_id_async(&browser_context_id)
        .await
    {
        conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
        return;
    }

    let Some(cleanup_plan) = detach_plan.cleanup_plan() else {
        conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
        return;
    };
    match cleanup_plan.action().clone() {
        crate::conn::TargetBindingCleanupAction::PageTarget {
            target_id,
            is_attached_session,
        } => {
            // A parent-session detach cascade bypasses the direct
            // Target.detachFromTarget path. Release the renderer inspector
            // here before resetting protocol state so the replacement primary
            // session gets a fresh Runtime.enable context inventory.
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                Some(session_id),
                "Target detached",
            );
            let _ = conn
                .detach_runtime_inspector_session_for_session_owner_async(Some(session_id))
                .await;
            if is_attached_session {
                clear_detached_session_target_overrides_best_effort(conn, session_id).await;
                super::clear_detached_target_fetch_state_background_events_async(
                    conn,
                    out.background_events_mut(),
                    session_id,
                )
                .await;
            } else if conn
                .browser_context
                .as_ref()
                .is_some_and(|browser_context| browser_context.is_active_target(&target_id))
            {
                events::fail_pending_fetch_state_for_target_background_events_async(
                    conn,
                    out.background_events_mut(),
                    Some(session_id),
                    "Target detached",
                )
                .await;
            } else {
                clear_detached_session_target_overrides_best_effort(conn, session_id).await;
            }
            let event_plan = conn
                .detach_session_with_binding_cleanup_event_plan_async(
                    crate::conn::TargetSessionDetachCleanupPlan::new(
                        target_id, session_id, None, None,
                    ),
                )
                .await;
            out.extend_background_events(event_plan);
        }
        crate::conn::TargetBindingCleanupAction::TabTarget { tab_target_id } => {
            let event_plan = conn
                .detach_session_with_binding_cleanup_event_plan_async(
                    crate::conn::TargetSessionDetachCleanupPlan::new(
                        tab_target_id,
                        session_id,
                        None,
                        None,
                    ),
                )
                .await;
            out.extend_background_events(event_plan);
        }
        crate::conn::TargetBindingCleanupAction::SharedWorkerTarget { target_id } => {
            let renderer_detach = conn.browser_context.as_ref().and_then(|bc| {
                bc.shared_worker_target(&target_id)
                    .map(|target| (bc.renderer_runtime(), target.renderer_instance_id))
            });
            conn.release_shared_worker_runtime_remote_objects_for_session_best_effort_async(
                session_id,
            )
            .await;
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                Some(session_id),
                "Target detached",
            );
            if let Some((renderer_runtime, instance_id)) = renderer_detach {
                renderer_runtime.detach_shared_worker_runtime_inspector_session(
                    instance_id,
                    Some(session_id.to_owned()),
                );
            }
            let event_plan = conn
                .detach_session_with_binding_cleanup_event_plan_async(
                    crate::conn::TargetSessionDetachCleanupPlan::new(
                        target_id, session_id, None, None,
                    ),
                )
                .await;
            out.extend_background_events(event_plan);
        }
        crate::conn::TargetBindingCleanupAction::DedicatedWorkerTarget { target_id } => {
            let renderer_detach = conn.browser_context.as_ref().and_then(|bc| {
                bc.dedicated_worker_target(&target_id)
                    .map(|target| (bc.renderer_runtime(), target.renderer_instance_id))
            });
            conn.release_shared_worker_runtime_remote_objects_for_session_best_effort_async(
                session_id,
            )
            .await;
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                Some(session_id),
                "Target detached",
            );
            if let Some((renderer_runtime, instance_id)) = renderer_detach {
                renderer_runtime.detach_dedicated_worker_runtime_inspector_session(
                    instance_id,
                    Some(session_id.to_owned()),
                );
            }
            let event_plan = conn
                .detach_session_with_binding_cleanup_event_plan_async(
                    crate::conn::TargetSessionDetachCleanupPlan::new(
                        target_id, session_id, None, None,
                    ),
                )
                .await;
            out.extend_background_events(event_plan);
        }
        crate::conn::TargetBindingCleanupAction::ServiceWorkerTarget { target_id } => {
            conn.fail_pending_inspector_awaits_for_session_owner_background_events_into(
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                Some(session_id),
                "Target detached",
            );
            super::set_service_worker_pause_on_start_owner(conn, Some(session_id), false);
            let event_plan = conn
                .detach_session_with_binding_cleanup_event_plan_async(
                    crate::conn::TargetSessionDetachCleanupPlan::new(
                        target_id, session_id, None, None,
                    ),
                )
                .await;
            out.extend_background_events(event_plan);
        }
        crate::conn::TargetBindingCleanupAction::None => {
            conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
        }
    }
}

pub(super) async fn clear_detached_session_target_overrides_best_effort(
    conn: &mut CdpConnection,
    session_id: &str,
) {
    if let Err(error) =
        crate::domains::emulation::clear_emulated_media_for_detached_session_async(conn, session_id)
            .await
    {
        tracing::warn!(
            session_id,
            error,
            "failed to clear emulated media while detaching target session"
        );
    }
    if let Err(error) = conn.clear_target_session_overrides_async(session_id).await {
        tracing::warn!(
            session_id,
            error,
            "failed to restore target overrides while detaching target session"
        );
    }
}

async fn ensure_initial_document_for_attached_page_targets_async(
    conn: &mut CdpConnection,
    attached_targets: &[(String, String)],
) {
    for (target_id, _session_id) in attached_targets {
        if !conn.browser_contexts().any(|browser_context| {
            browser_context.target_has_pending_initial_document_page_build(target_id)
        }) {
            continue;
        }
        let Some(route) = conn.target_session_route_for_target_id(target_id) else {
            continue;
        };
        let pending = {
            let mut route_scope = conn.scoped_none_session_owner_route_override(route);
            match route_scope
                .conn_mut()
                .start_initial_document_page_ensure_for_session_owner(None)
            {
                Ok(pending) => pending,
                Err(message) => {
                    warn_target_protocol_side_effect_failure(
                        target_id,
                        "start_initial_document_page_ensure",
                        &message,
                    );
                    continue;
                }
            }
        };
        let Some(pending) = pending else {
            continue;
        };
        match pending.wait().await {
            Ok(completed) => {
                if let Err(message) = conn
                    .complete_initial_document_page_build_for_owner(completed)
                    .await
                {
                    warn_target_protocol_side_effect_failure(
                        target_id,
                        "complete_initial_document_page_build",
                        &message,
                    );
                }
            }
            Err(failed) => {
                let message = conn.reset_failed_initial_document_page_build_for_owner(failed);
                warn_target_protocol_side_effect_failure(
                    target_id,
                    "reset_failed_initial_document_page_build",
                    &message,
                );
            }
        }
    }
}

fn warn_target_protocol_side_effect_failure(
    target_id: &str,
    operation: &'static str,
    message: &str,
) {
    tracing::warn!(
        target_id,
        operation,
        %message,
        "target protocol side effect failed; continuing auto-attach event emission"
    );
}
