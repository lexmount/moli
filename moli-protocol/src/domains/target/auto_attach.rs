use serde::Deserialize;

use crate::conn::{
    CdpSessionRoute, CdpTargetFilter, CdpTargetFilterEntry, CommandOwnerScope,
    PreparedTargetAttach, TargetAttachSessionCommit,
};

use super::*;

mod workflow;
pub(crate) use workflow::{
    AutoAttachSweepRollback, PreparedAutoAttachCommand, PreparedAutoAttachSweep,
};

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
        let route = CdpSessionRoute::ServiceWorkerTarget {
            browser_context_id: target.browser_context_id.clone(),
            target_id: target.target_id.clone(),
        };
        let attachment = conn
            .worker_protocol_attachment_identity_for_route(&route, &session_id)
            .expect("related worker attach retains its original native scope");
        let event_plan = conn.commit_prepared_attach_event_plan(
            PreparedTargetAttach::new(
                &target.target_id,
                target_info,
                [TargetAttachSessionCommit::auto_attached(
                    session_id,
                    cmd.session_id.map(str::to_owned),
                    route,
                    false,
                )],
            )
            .with_worker_attachment(attachment),
        );
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
        .filter(|session_id| !conn.session_is_closing(session_id))
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
        .filter(|session_id| !conn.session_is_closing(session_id))
        .any(|session_id| attached_sessions.contains(session_id))
}

fn owner_already_auto_attached_to_page_target(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    target_id: &str,
) -> bool {
    conn.auto_attached_sessions_for_owner(owner_session_id)
        .into_iter()
        .filter(|session_id| !conn.session_is_closing(session_id))
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
        .filter(|session_id| !conn.session_is_closing(session_id))
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
    let initial_document = params
        .auto_attach
        .then(|| {
            let tab_target = conn.tab_target_id_for_session_id(cmd.session_id?)?;
            let page_target = conn.primary_page_target_id_for_tab_target_id(tab_target)?;
            if !conn.auto_attach_owner_allows_target_type(cmd.session_id, "page") {
                return None;
            }
            let route = conn.target_session_route_for_target_id(page_target)?;
            conn.runtime_session_owner_slot_for_owner(&CommandOwnerScope::for_route(route))?
                .initial_document_page_build_waiter()
        })
        .flatten();
    let command = PreparedAutoAttachCommand::prepare(
        conn,
        params.auto_attach,
        cmd.session_id,
        initial_document,
    );
    if let Some(access) = conn.protocol_state_access() {
        TargetCommandTaskStep::Workflow(command.into_workflow(access))
    } else {
        TargetCommandTaskStep::Pending(PendingTargetCommandDispatch {
            command_id: cmd.id,
            session_id: cmd.session_id.map(str::to_owned),
            kind: Box::new(PendingTargetCommandKind::SetAutoAttach(Box::new(command))),
        })
    }
}

fn prepare_context_sweep(
    conn: &mut CdpConnection,
    context_id: &str,
    owner_session_id: Option<&str>,
    rollback: &AutoAttachSweepRollback,
) -> Option<workflow::AutoAttachSweepContext> {
    let restore = previously_active_browser_context_id(conn);
    if !conn.activate_browser_context_by_id(context_id) {
        return None;
    }
    let prepared = {
        let attach_page_targets = conn
            .auto_attach_owner_allows_target_type(owner_session_id, "page")
            && super::browser_level_auto_attach_owner_session_allowed(conn, owner_session_id);
        let attach_tab_targets = conn.auto_attach_owner_allows_target_type(owner_session_id, "tab")
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
                    should_auto_attach_tab_target_for_owner(conn, owner_session_id, tab_target_id)
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
        let foreground_target_id = {
            let bc = conn.browser_context.as_ref().expect(
                "browser context must exist when choosing an auto-attached foreground target",
            );
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
        let attached_targets = attached_targets
            .into_iter()
            .filter_map(|(target_id, session_id)| {
                let route =
                    conn.prepare_auto_attached_page_session_binding(&target_id, session_id.clone());
                debug_assert!(route.is_some(), "attached target must remain addressable");
                route.map(|route| {
                    rollback.record(TargetAttachSessionCommit::auto_attached(
                        session_id.clone(),
                        owner_session_id.map(str::to_owned),
                        route.clone(),
                        false,
                    ));
                    (target_id, session_id, route)
                })
            })
            .collect::<Vec<_>>();
        let attached_shared_worker_targets = attached_shared_worker_targets
            .into_iter()
            .filter_map(|(target_id, session_id)| {
                let route = conn.prepare_auto_attached_shared_worker_session_binding(
                    &target_id,
                    session_id.clone(),
                );
                debug_assert!(
                    route.is_some(),
                    "attached shared worker target must remain addressable"
                );
                let route = route?;
                rollback.record(TargetAttachSessionCommit::auto_attached(
                    session_id.clone(),
                    owner_session_id.map(str::to_owned),
                    route.clone(),
                    false,
                ));
                let attachment =
                    conn.worker_protocol_attachment_identity_for_route(&route, &session_id)?;
                let info = conn
                    .browser_context_by_id(route.browser_context_id()?)?
                    .devtools_target_info(&target_id)?;
                Some(
                    PreparedTargetAttach::new(
                        target_id,
                        info,
                        [TargetAttachSessionCommit::auto_attached(
                            session_id,
                            owner_session_id.map(str::to_owned),
                            route,
                            waiting_for_debugger,
                        )],
                    )
                    .with_worker_attachment(attachment),
                )
            })
            .collect::<Vec<_>>();
        let attached_dedicated_worker_targets = attached_dedicated_worker_targets
            .into_iter()
            .filter_map(|(target_id, session_id)| {
                let route = conn.prepare_auto_attached_dedicated_worker_session_binding(
                    &target_id,
                    session_id.clone(),
                );
                debug_assert!(
                    route.is_some(),
                    "attached dedicated worker target must remain addressable"
                );
                let route = route?;
                rollback.record(TargetAttachSessionCommit::auto_attached(
                    session_id.clone(),
                    owner_session_id.map(str::to_owned),
                    route.clone(),
                    false,
                ));
                let attachment =
                    conn.worker_protocol_attachment_identity_for_route(&route, &session_id)?;
                let info = conn
                    .browser_context_by_id(route.browser_context_id()?)?
                    .devtools_target_info(&target_id)?;
                Some(
                    PreparedTargetAttach::new(
                        target_id,
                        info,
                        [TargetAttachSessionCommit::auto_attached(
                            session_id,
                            owner_session_id.map(str::to_owned),
                            route,
                            waiting_for_debugger,
                        )],
                    )
                    .with_worker_attachment(attachment),
                )
            })
            .collect::<Vec<_>>();
        let attached_service_worker_targets = attached_service_worker_targets
            .into_iter()
            .filter_map(|(target_id, session_id)| {
                let route = conn.prepare_auto_attached_service_worker_session_binding(
                    &target_id,
                    session_id.clone(),
                );
                debug_assert!(
                    route.is_some(),
                    "attached service worker target must remain addressable"
                );
                let route = route?;
                rollback.record(TargetAttachSessionCommit::auto_attached(
                    session_id.clone(),
                    owner_session_id.map(str::to_owned),
                    route.clone(),
                    false,
                ));
                let attachment =
                    conn.worker_protocol_attachment_identity_for_route(&route, &session_id)?;
                let info = conn
                    .browser_context_by_id(route.browser_context_id()?)?
                    .devtools_target_info(&target_id)?;
                Some(
                    PreparedTargetAttach::new(
                        target_id,
                        info,
                        [TargetAttachSessionCommit::auto_attached(
                            session_id,
                            owner_session_id.map(str::to_owned),
                            route,
                            waiting_for_debugger,
                        )],
                    )
                    .with_worker_attachment(attachment),
                )
            })
            .collect::<Vec<_>>();
        let attached_tab_targets = attached_tab_targets
            .into_iter()
            .filter_map(|(target_id, session_id)| {
                let route = conn.prepare_auto_attached_tab_session_binding(
                    &target_id,
                    session_id.clone(),
                    owner_session_id,
                );
                debug_assert!(
                    route.is_some(),
                    "attached tab target must remain addressable"
                );
                route.map(|route| {
                    rollback.record(TargetAttachSessionCommit::auto_attached(
                        session_id.clone(),
                        owner_session_id.map(str::to_owned),
                        route.clone(),
                        false,
                    ));
                    (target_id, session_id, route)
                })
            })
            .collect::<Vec<_>>();

        let activation = foreground_target_id.and_then(|target| {
            match super::activation::prepare_target_activation_for_target(conn, &target) {
                Ok(prepared) => prepared,
                Err(error) => {
                    tracing::warn!(?error, "auto-attach foreground selection retired");
                    None
                }
            }
        });
        let pages = attached_targets
            .into_iter()
            .map(|(_, session, route)| {
                TargetAttachSessionCommit::auto_attached(
                    session,
                    owner_session_id.map(str::to_owned),
                    route,
                    false,
                )
            })
            .collect::<Vec<_>>();
        let tabs = attached_tab_targets
            .into_iter()
            .map(|(_, session, route)| {
                TargetAttachSessionCommit::auto_attached(
                    session,
                    owner_session_id.map(str::to_owned),
                    route,
                    false,
                )
            })
            .collect::<Vec<_>>();
        let workers = attached_shared_worker_targets
            .into_iter()
            .chain(attached_dedicated_worker_targets)
            .chain(attached_service_worker_targets)
            .collect::<Vec<_>>();
        Some(workflow::AutoAttachSweepContext {
            pages,
            tabs,
            workers,
            activation,
        })
    };
    restore_previously_active_browser_context(conn, restore.as_deref());
    prepared
}

fn prepare_tab_child_sweep(
    conn: &mut CdpConnection,
    owner: &str,
    tab: &str,
    rollback: &AutoAttachSweepRollback,
) -> Option<workflow::AutoAttachSweepContext> {
    if !conn.auto_attach_owner_allows_target_type(Some(owner), "page") {
        return None;
    }
    let page = conn
        .primary_page_target_id_for_tab_target_id(tab)?
        .to_owned();
    let context_id = conn.browser_context_id_for_tab_target_id(tab)?;
    let target = conn
        .browser_context_by_id(&context_id)?
        .page_target(&page)?;
    if !should_auto_attach_page_target_for_owner(conn, Some(owner), &page, target.has_session()) {
        return None;
    }
    let session = conn.gen_session_id();
    let route = conn.prepare_auto_attached_page_session_binding_in_browser_context(
        &context_id,
        &page,
        session.clone(),
    )?;
    let prepared =
        TargetAttachSessionCommit::auto_attached(session, Some(owner.to_owned()), route, false);
    rollback.record(prepared.clone());
    Some(workflow::AutoAttachSweepContext {
        pages: vec![prepared],
        tabs: Vec::new(),
        workers: Vec::new(),
        activation: None,
    })
}

fn prepared_session_can_commit(conn: &CdpConnection, session: &TargetAttachSessionCommit) -> bool {
    let owner = CommandOwnerScope::for_route(session.route().clone());
    if conn.target_is_closing_for_owner(&owner) {
        return false;
    }
    match session.route() {
        CdpSessionRoute::PageTarget {
            browser_context_id,
            target_id,
            session_key,
        } => conn
            .browser_context_by_id(browser_context_id)
            .and_then(|context| context.page_target(target_id))
            .is_some_and(|target| {
                !target.closing
                    && target
                        .devtools_sessions
                        .key_for_wire_session_id(session.session_id())
                        .as_ref()
                        == Some(session_key)
            }),
        CdpSessionRoute::TabTarget { tab_target_id, .. } => {
            conn.tab_target_id_for_session_id(session.session_id()) == Some(tab_target_id.as_str())
                && conn
                    .primary_page_target_id_for_tab_target_id(tab_target_id)
                    .is_some_and(|page| !conn.page_target_is_closing(page))
        }
        CdpSessionRoute::SharedWorkerTarget { .. }
        | CdpSessionRoute::DedicatedWorkerTarget { .. }
        | CdpSessionRoute::ServiceWorkerTarget { .. } => session
            .worker_attachment()
            .is_some_and(|attachment| attachment.is_current()),
        CdpSessionRoute::Browser | CdpSessionRoute::BrowserContext { .. } => false,
    }
}

#[cfg(test)]
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
            detach_plan.cleanup_plan().map(|plan| plan.target()),
            Some(crate::conn::SessionDisposalTarget::PageTarget {
                session_key: moli_page_types::DevToolsSessionKey::Primary,
                ..
            })
        );
        if !preserves_other_frontends {
            detach_attached_session_for_owner_async(conn, out, &session_id, command_context).await;
            continue;
        }

        let predecessor =
            match super::session_disposal::dispose_primary_page_session_preserving_frontend_async(
                conn,
                out.background_events_mut(),
                command_context.protocol_events_mut(),
                &session_id,
            )
            .await
            {
                Ok(predecessor) => predecessor,
                Err(error) => {
                    tracing::warn!(
                    session_id,
                    %error,
                        "failed to dispose root-owned page session runtime state"
                    );
                    continue;
                }
            };
        if let Some(predecessor) = predecessor {
            command_context.set_renderer_output_predecessor(predecessor);
        }
    }
}

#[cfg(test)]
async fn detach_attached_session_for_owner_async(
    conn: &mut CdpConnection,
    out: &mut events::TargetProtocolSideEffects,
    session_id: &str,
    command_context: &mut crate::conn::CommandDispatchContext,
) {
    if conn.is_browser_session_id(Some(session_id)) {
        if let Err(error) =
            super::session_disposal::dispose_browser_session_without_event_async(conn, session_id)
                .await
        {
            tracing::warn!(
                session_id,
                %error,
                "failed to dispose auto-attached browser session"
            );
        }
        return;
    }
    let detach_plan = conn.auto_attached_session_detach_plan(session_id);
    let Some(cleanup_plan) = detach_plan.cleanup_plan() else {
        conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
        return;
    };
    let Some(target_id) = cleanup_plan.target_id().map(str::to_owned) else {
        debug_assert!(false, "browser sessions use the browser disposal path");
        conn.rollback_auto_attached_session_detach_plan_without_event(&detach_plan);
        return;
    };
    match super::session_disposal::dispose_target_session_async(
        conn,
        out.background_events_mut(),
        command_context.protocol_events_mut(),
        crate::conn::TargetSessionDetachCleanupPlan::new(target_id, session_id, None, None),
    )
    .await
    {
        Ok(outcome) => {
            let (event_plan, predecessor) = outcome.into_parts();
            if let Some(predecessor) = predecessor {
                command_context.set_renderer_output_predecessor(predecessor);
            }
            out.extend_background_events(event_plan);
        }
        Err(error) => {
            tracing::warn!(
                session_id,
                %error,
                "failed to dispose auto-attached target session"
            );
        }
    }
}
