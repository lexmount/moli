use serde::Deserialize;

use crate::conn::{BackgroundProtocolEvent, BrowserContext, CdpConnection, Cmd};
use crate::devtools_runtime::{
    DevToolsActivateTargetCommand, DevToolsCloseTargetCommand, DevToolsCommand,
    DevToolsCommandResult, DevToolsCreateTargetCommand, DevToolsError, DevToolsErrorKind,
    DevToolsTargetFilterEntry, DevToolsTargetInfo, DevToolsTargetKind,
};
use crate::domains::actions::TargetAction;
use crate::domains::command_output::CommandOutputPlan;

use super::page;

mod attachment;
mod browser_context;
mod browser_context_disposal;
mod events;
mod lifecycle;
#[cfg(test)]
mod tests;
mod worker_target;

pub(in crate::domains) use browser_context::devtools_client_window_info_for_target;
pub(crate) use lifecycle::{
    PopupTargetCreation, PopupTargetOpenerIdentity, complete_popup_target_activation_action_async,
    complete_popup_target_navigation_owner_action_async,
    create_popup_target_from_renderer_output_background_events_async,
    emit_target_info_changed_for_session_owner_background_event,
    start_initial_document_target_url_navigation_if_needed_background_events_async,
};
pub(crate) fn popup_activation_creates_new_target(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
    target_name: &str,
) -> bool {
    if let Some((browser_context_id, _)) = conn.target_owner_identity_for_session(owner_session_id)
    {
        return conn
            .browser_context_by_id(&browser_context_id)
            .is_none_or(|browser_context| {
                browser_context
                    .target_id_for_window_name(target_name)
                    .is_none()
            });
    }
    conn.browser_context.as_ref().is_none_or(|browser_context| {
        browser_context
            .target_id_for_window_name(target_name)
            .is_none()
    })
}
pub(in crate::domains) use worker_target::{
    TargetPreparedOutputSlot, dedicated_worker_main_script_network_replay_for_session,
    dedicated_worker_target_lifecycle_prepared_outputs_for_event,
    project_worker_target_output_async,
    release_failed_dedicated_worker_target_after_debugger_resume,
    retire_dedicated_worker_targets_for_replaced_page_async,
    service_worker_target_lifecycle_prepared_outputs_for_event,
    shared_worker_target_lifecycle_prepared_outputs_for_event,
};

/// Browser-owned auto-attach policies may observe browser-level targets.
///
/// A target filter narrows the target kinds requested by one TargetHandler; it
/// does not expand a page or worker TargetHandler to browser-global targets.
fn browser_level_auto_attach_owner_session_allowed(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
) -> bool {
    owner_session_id.is_none() || conn.is_browser_session_id(owner_session_id)
}

pub(crate) struct PendingTargetCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    kind: Box<PendingTargetCommandKind>,
}

pub(crate) struct CompletedTargetCommandDispatch {
    command_id: Option<u64>,
    session_id: Option<String>,
    kind: CompletedTargetCommandKind,
}

pub(crate) enum TargetCommandTaskStep {
    Pending(PendingTargetCommandDispatch),
    Complete(CommandOutputPlan),
}

pub(super) fn target_command_error(code: i32, message: impl Into<String>) -> TargetCommandTaskStep {
    TargetCommandTaskStep::Complete(CommandOutputPlan::error(code, message))
}

fn transient_no_page_devtools_target_info_error(
    conn: &CdpConnection,
    target_info: &DevToolsTargetInfo,
) -> Option<String> {
    if target_info.kind != DevToolsTargetKind::Page {
        return None;
    }
    let target_id = target_info.target_id.as_ref()?.as_str();
    let reason = conn.browser_contexts().find_map(|browser_context| {
        browser_context.target_transient_no_page_reason_for_protocol_output(target_id)
    })?;
    Some(format!(
        "TargetPageNotReady: target {target_id} still has transient no-page reason {reason}"
    ))
}

pub(in crate::domains) fn set_service_worker_pause_on_start_owner(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    enabled: bool,
) {
    conn.set_service_worker_pause_on_start_owner(session_id, enabled);
}

fn sync_dedicated_worker_pause_on_start_for_devtools(conn: &CdpConnection) {
    let pause = conn.dedicated_worker_pause_on_start_for_devtools();
    let runtimes = conn
        .browser_contexts()
        .map(BrowserContext::renderer_runtime)
        .collect::<Vec<_>>();
    for runtime in runtimes {
        runtime.set_dedicated_worker_pause_on_start_for_devtools(pause);
    }
}

pub(in crate::domains) fn set_dedicated_worker_pause_on_start_owner(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    enabled: bool,
) {
    conn.set_dedicated_worker_pause_on_start_owner(session_id, enabled);
    sync_dedicated_worker_pause_on_start_for_devtools(conn);
}

pub(in crate::domains::target) async fn clear_detached_target_fetch_state_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: &str,
) {
    clear_detached_target_owner_fetch_state_background_events_async(conn, out, Some(session_id))
        .await;
}

async fn clear_detached_target_owner_fetch_state_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    session_id: Option<&str>,
) {
    let Some((pending_fetch_state, pending_page_command)) =
        (match conn.start_disable_fetch_for_session_owner(session_id) {
            Ok(disable_state) => disable_state,
            Err(error) => {
                tracing::warn!(
                    ?session_id,
                    error,
                    "failed to disable Fetch interception while detaching target owner"
                );
                return;
            }
        })
    else {
        return;
    };

    if let Some(pending_page_command) = pending_page_command {
        match pending_page_command.wait().await {
            Ok(completion) => match conn.loaded_page_mut_for_protocol_access(session_id) {
                Ok(page) => {
                    if let Err(error) = page.finish_set_fetch_subresource_interception(completion) {
                        tracing::warn!(
                            ?session_id,
                            %error,
                            "failed to finish Fetch interception disable while detaching target owner"
                        );
                    }
                }
                Err(message) if message == "NoDocumentLoaded" => {}
                Err(message) => {
                    tracing::warn!(
                        ?session_id,
                        message,
                        "failed to find page while detaching target owner Fetch state"
                    );
                }
            },
            Err(error) => {
                tracing::warn!(
                    ?session_id,
                    %error,
                    "renderer failed Fetch interception disable while detaching target owner"
                );
            }
        }
    }

    let (
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    ) = pending_fetch_state;
    // Target-owner teardown has no command response to fence. The concrete
    // renderer publication remains ordered on its own stream and will reach
    // protocol ingress independently.
    let _ = page::fail_pending_fetch_state_background_events_async(
        conn,
        out,
        session_id,
        "Target detached",
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    )
    .await;
}

impl CdpConnection {
    /// Releases root-owned Target control state after its transport frontend
    /// disconnects, while preserving sessions owned by direct page frontends.
    pub async fn release_root_target_frontend_state_async(&mut self) {
        let previously_active_browser_context_id = previously_active_browser_context_id(self);
        let mut side_effects = events::TargetProtocolSideEffects::default();
        let mut command_context = crate::conn::CommandDispatchContext::default();

        self.fail_pending_inspector_awaits_for_session_owner_background_events_into(
            side_effects.background_events_mut(),
            command_context.protocol_events_mut(),
            None,
            "Inspector detached",
        );
        clear_detached_target_owner_fetch_state_background_events_async(
            self,
            side_effects.background_events_mut(),
            None,
        )
        .await;
        let _ = self
            .detach_runtime_inspector_session_for_session_owner_async(None)
            .await;
        lifecycle::release_attached_sessions_for_root_frontend_async(
            self,
            &mut side_effects,
            &mut command_context,
        )
        .await;
        self.cancel_tracing_for_session_owner_async(None).await;
        self.release_root_target_frontend_owner_without_event();
        set_service_worker_pause_on_start_owner(self, None, false);
        set_dedicated_worker_pause_on_start_owner(self, None, false);
        restore_previously_active_browser_context(
            self,
            previously_active_browser_context_id.as_deref(),
        );
    }
}

enum PendingTargetCommandKind {
    AttachToTarget {
        attached_session_id: String,
        target_info: DevToolsTargetInfo,
        initial_document: Option<Box<crate::conn::PendingInitialDocumentPageBuild>>,
    },
    ActivateTarget {
        command: DevToolsActivateTargetCommand,
    },
    SetAutoAttach {
        auto_attach: bool,
        owner_session_id: Option<String>,
        owner_was_enabled: bool,
        legacy_disable_all: bool,
    },
    CreateTarget {
        response_plan: CommandOutputPlan,
        protocol_events: lifecycle::CreatedTargetProtocolEvents,
        initial_document_route: Option<crate::conn::CdpSessionRoute>,
        initial_document: Option<Box<crate::conn::PendingInitialDocumentPageBuild>>,
    },
    DetachFromTarget {
        target_id: Option<String>,
        detach_session_id: Option<String>,
    },
    CloseTarget {
        command: DevToolsCloseTargetCommand,
    },
    DisposeBrowserContext {
        browser_context_id: String,
    },
    SendMessageToTarget {
        message: String,
        target_session_id: Option<String>,
    },
}

enum CompletedTargetCommandKind {
    AttachToTarget {
        attached_session_id: String,
        target_info: DevToolsTargetInfo,
        initial_document: Option<
            Result<
                Box<crate::conn::CompletedInitialDocumentPageBuild>,
                crate::conn::FailedInitialDocumentPageBuild,
            >,
        >,
    },
    ActivateTarget {
        command: DevToolsActivateTargetCommand,
    },
    SetAutoAttach {
        auto_attach: bool,
        owner_session_id: Option<String>,
        owner_was_enabled: bool,
        legacy_disable_all: bool,
    },
    CreateTarget {
        response_plan: CommandOutputPlan,
        protocol_events: lifecycle::CreatedTargetProtocolEvents,
        initial_document_route: Option<crate::conn::CdpSessionRoute>,
        initial_document: Option<
            Result<
                Box<crate::conn::CompletedInitialDocumentPageBuild>,
                crate::conn::FailedInitialDocumentPageBuild,
            >,
        >,
    },
    DetachFromTarget {
        target_id: Option<String>,
        detach_session_id: Option<String>,
    },
    CloseTarget {
        command: DevToolsCloseTargetCommand,
    },
    DisposeBrowserContext {
        browser_context_id: String,
    },
    SendMessageToTarget {
        message: String,
        target_session_id: Option<String>,
    },
}

impl PendingTargetCommandDispatch {
    pub(crate) async fn wait(self) -> CompletedTargetCommandDispatch {
        let kind = match *self.kind {
            PendingTargetCommandKind::AttachToTarget {
                attached_session_id,
                target_info,
                initial_document,
            } => CompletedTargetCommandKind::AttachToTarget {
                attached_session_id,
                target_info,
                initial_document: match initial_document {
                    Some(pending) => Some(pending.wait().await.map(Box::new)),
                    None => None,
                },
            },
            PendingTargetCommandKind::ActivateTarget { command } => {
                CompletedTargetCommandKind::ActivateTarget { command }
            }
            PendingTargetCommandKind::SetAutoAttach {
                auto_attach,
                owner_session_id,
                owner_was_enabled,
                legacy_disable_all,
            } => CompletedTargetCommandKind::SetAutoAttach {
                auto_attach,
                owner_session_id,
                owner_was_enabled,
                legacy_disable_all,
            },
            PendingTargetCommandKind::CreateTarget {
                response_plan,
                protocol_events,
                initial_document_route,
                initial_document,
            } => CompletedTargetCommandKind::CreateTarget {
                response_plan,
                protocol_events,
                initial_document_route,
                initial_document: match initial_document {
                    Some(pending) => Some(pending.wait().await.map(Box::new)),
                    None => None,
                },
            },
            PendingTargetCommandKind::DetachFromTarget {
                target_id,
                detach_session_id,
            } => CompletedTargetCommandKind::DetachFromTarget {
                target_id,
                detach_session_id,
            },
            PendingTargetCommandKind::CloseTarget { command } => {
                CompletedTargetCommandKind::CloseTarget { command }
            }
            PendingTargetCommandKind::DisposeBrowserContext { browser_context_id } => {
                CompletedTargetCommandKind::DisposeBrowserContext { browser_context_id }
            }
            PendingTargetCommandKind::SendMessageToTarget {
                message,
                target_session_id,
            } => CompletedTargetCommandKind::SendMessageToTarget {
                message,
                target_session_id,
            },
        };
        CompletedTargetCommandDispatch {
            command_id: self.command_id,
            session_id: self.session_id,
            kind,
        }
    }
}

impl CompletedTargetCommandDispatch {
    pub(crate) fn command_id(&self) -> Option<u64> {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
}

pub(crate) fn try_start_target_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<TargetCommandTaskStep> {
    match cmd.parse_action::<TargetAction>() {
        Some(TargetAction::GetTargets) => {
            Some(browser_context::start_get_targets_command(conn, cmd))
        }
        Some(TargetAction::GetBrowserContexts) => Some(TargetCommandTaskStep::Complete(
            browser_context::get_browser_contexts(conn),
        )),
        Some(TargetAction::CreateBrowserContext) => Some(TargetCommandTaskStep::Complete(
            browser_context::create_browser_context(conn, cmd),
        )),
        Some(TargetAction::CreateTarget) => Some(lifecycle::start_create_target_command(conn, cmd)),
        Some(TargetAction::AttachToTarget) => {
            Some(attachment::start_attach_to_target_command(conn, cmd))
        }
        Some(TargetAction::AttachToBrowserTarget) => Some(TargetCommandTaskStep::Complete(
            attachment::attach_to_browser_target_command(conn, cmd),
        )),
        Some(TargetAction::GetTargetInfo) => Some(TargetCommandTaskStep::Complete(
            lifecycle::get_target_info(conn, cmd),
        )),
        Some(TargetAction::SetDiscoverTargets) => Some(TargetCommandTaskStep::Complete(
            set_discover_targets(conn, cmd),
        )),
        Some(TargetAction::ActivateTarget) => {
            Some(lifecycle::start_activate_target_command(conn, cmd))
        }
        Some(TargetAction::SetAutoAttach) => {
            Some(lifecycle::start_set_auto_attach_command(conn, cmd))
        }
        Some(TargetAction::AutoAttachRelated) => Some(TargetCommandTaskStep::Complete(
            lifecycle::auto_attach_related(conn, cmd),
        )),
        Some(TargetAction::DetachFromTarget) => {
            Some(attachment::start_detach_from_target_command(cmd))
        }
        Some(TargetAction::CloseTarget) => Some(lifecycle::start_close_target_command(conn, cmd)),
        Some(TargetAction::DisposeBrowserContext) => {
            Some(browser_context::start_dispose_browser_context_command(cmd))
        }
        Some(TargetAction::SendMessageToTarget) => {
            Some(attachment::start_send_message_to_target_command(cmd))
        }
        None => Some(target_command_error(-32601, "UnknownMethod")),
    }
}

fn start_devtools_target_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsCommand,
) -> TargetCommandTaskStep {
    match command {
        DevToolsCommand::CreateTarget(command) => lifecycle::start_devtools_create_target_command(
            conn,
            command_id,
            command_session_id,
            command,
        ),
        DevToolsCommand::CloseTarget(command) => {
            lifecycle::start_devtools_close_target_command(command_id, command_session_id, command)
        }
        DevToolsCommand::ActivateTarget(command) => {
            pending_activate_target_command(command_id, command_session_id, command)
        }
        DevToolsCommand::GetTargets(command) => TargetCommandTaskStep::Complete(
            browser_context::start_devtools_get_targets_command(conn, command),
        ),
        DevToolsCommand::GetServiceWorkerLogs(command) => TargetCommandTaskStep::Complete(
            match browser_context::execute_devtools_get_service_worker_logs_command(conn, &command)
            {
                Ok(result) => CommandOutputPlan::from_devtools_result(
                    DevToolsCommandResult::ServiceWorkerLogs(result),
                ),
                Err(error) => CommandOutputPlan::from_devtools_error(error),
            },
        ),
        DevToolsCommand::GetClientWindows(command) => TargetCommandTaskStep::Complete(
            match browser_context::execute_devtools_get_client_windows_command(conn, &command) {
                Ok(result) => CommandOutputPlan::from_devtools_result(
                    DevToolsCommandResult::ClientWindows(result),
                ),
                Err(error) => CommandOutputPlan::from_devtools_error(error),
            },
        ),
        DevToolsCommand::CreateBrowserContext(command) => {
            let plan = match browser_context::execute_devtools_create_browser_context_command(
                conn, command,
            ) {
                Ok(result) => CommandOutputPlan::from_devtools_result(
                    DevToolsCommandResult::CreateBrowserContext(result),
                ),
                Err(error) => CommandOutputPlan::from_devtools_error(error),
            };
            TargetCommandTaskStep::Complete(plan)
        }
        DevToolsCommand::GetBrowserContexts(command) => TargetCommandTaskStep::Complete(
            CommandOutputPlan::from_devtools_result(DevToolsCommandResult::GetBrowserContexts(
                browser_context::devtools_get_browser_contexts_result(conn, &command),
            )),
        ),
        DevToolsCommand::GetTargetInfo(command) => TargetCommandTaskStep::Complete(
            lifecycle::start_devtools_get_target_info_command(conn, command),
        ),
        _ => target_command_error(-32000, "UnsupportedDevToolsCommand"),
    }
}

pub(crate) fn execute_immediate_devtools_target_command_with_protocol_events(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
) -> (
    Result<DevToolsCommandResult, DevToolsError>,
    Vec<crate::conn::BackgroundProtocolEvent>,
) {
    match command {
        DevToolsCommand::CreateTarget(command) => {
            let result = lifecycle::execute_devtools_create_target_command(conn, command)
                .map(|execution| DevToolsCommandResult::CreateTarget(execution.result));
            (result, Vec::new())
        }
        DevToolsCommand::GetTargets(command) => (
            browser_context::execute_devtools_get_targets_command(conn, &command)
                .map(DevToolsCommandResult::GetTargets),
            Vec::new(),
        ),
        DevToolsCommand::GetServiceWorkerLogs(command) => (
            browser_context::execute_devtools_get_service_worker_logs_command(conn, &command)
                .map(DevToolsCommandResult::ServiceWorkerLogs),
            Vec::new(),
        ),
        DevToolsCommand::GetClientWindows(command) => (
            browser_context::execute_devtools_get_client_windows_command(conn, &command)
                .map(DevToolsCommandResult::ClientWindows),
            Vec::new(),
        ),
        DevToolsCommand::CreateBrowserContext(command) => (
            browser_context::execute_devtools_create_browser_context_command(conn, command)
                .map(DevToolsCommandResult::CreateBrowserContext),
            Vec::new(),
        ),
        DevToolsCommand::GetBrowserContexts(command) => (
            Ok(DevToolsCommandResult::GetBrowserContexts(
                browser_context::devtools_get_browser_contexts_result(conn, &command),
            )),
            Vec::new(),
        ),
        DevToolsCommand::GetTargetInfo(command) => (
            lifecycle::execute_devtools_get_target_info_command(conn, command)
                .map(DevToolsCommandResult::GetTargetInfo),
            Vec::new(),
        ),
        _ => (
            Err(DevToolsError::new(
                DevToolsErrorKind::Unsupported,
                "UnsupportedDevToolsCommand",
            )),
            Vec::new(),
        ),
    }
}

pub(crate) async fn execute_devtools_create_target_command_async_with_protocol_events(
    conn: &mut CdpConnection,
    command: DevToolsCreateTargetCommand,
) -> (
    Result<DevToolsCommandResult, DevToolsError>,
    Vec<crate::conn::BackgroundProtocolEvent>,
    Option<moli_core::RendererOutputFence>,
) {
    let execution = match lifecycle::execute_devtools_create_target_command(conn, command) {
        Ok(execution) => execution,
        Err(error) => return (Err(error), Vec::new(), None),
    };
    let result = execution.result;
    let mut protocol_events = Vec::new();
    let (initial_document_events, renderer_output_predecessor) = conn
        .ensure_created_target_initial_document_page(&result.target_id)
        .await;
    protocol_events.extend(initial_document_events);
    if let Err(error) = lifecycle::emit_created_target_protocol_events(
        conn,
        execution.protocol_events,
        &mut protocol_events,
    ) {
        return (Err(error), Vec::new(), renderer_output_predecessor);
    }
    (
        Ok(DevToolsCommandResult::CreateTarget(result)),
        protocol_events,
        renderer_output_predecessor,
    )
}

pub(crate) async fn execute_devtools_target_command_async_with_protocol_events(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
) -> (
    Result<DevToolsCommandResult, DevToolsError>,
    Vec<crate::conn::BackgroundProtocolEvent>,
) {
    match command {
        DevToolsCommand::CloseTarget(command) => {
            let mut command_context = crate::conn::CommandDispatchContext::default();
            let mut side_effects = events::TargetProtocolSideEffects::default();
            let result = lifecycle::execute_devtools_close_target_command_async(
                conn,
                command,
                &mut side_effects,
                &mut command_context,
            )
            .await
            .map(DevToolsCommandResult::CloseTarget);
            let mut protocol_events = side_effects.into_background_events();
            protocol_events.append(&mut command_context.take_protocol_events());
            (result, protocol_events)
        }
        DevToolsCommand::ActivateTarget(command) => {
            match lifecycle::execute_devtools_activate_target_command_async(conn, command).await {
                Ok(events) => (Ok(DevToolsCommandResult::Empty), events),
                Err(error) => (Err(error), Vec::new()),
            }
        }
        DevToolsCommand::RemoveBrowserContext(_) => {
            let DevToolsCommand::RemoveBrowserContext(command) = command else {
                unreachable!("matched remove browser context command");
            };
            browser_context::execute_devtools_remove_browser_context_command_async(conn, command)
                .await
        }
        DevToolsCommand::CreateTarget(command) => {
            let (result, events, _) =
                execute_devtools_create_target_command_async_with_protocol_events(conn, command)
                    .await;
            (result, events)
        }
        DevToolsCommand::GetTargets(_)
        | DevToolsCommand::GetClientWindows(_)
        | DevToolsCommand::CreateBrowserContext(_)
        | DevToolsCommand::GetBrowserContexts(_) => {
            execute_immediate_devtools_target_command_with_protocol_events(conn, command)
        }
        _ => (
            Err(DevToolsError::new(
                DevToolsErrorKind::Unsupported,
                "UnsupportedDevToolsCommand",
            )),
            Vec::new(),
        ),
    }
}

async fn created_target_response_plan_after_initial_document(
    conn: &mut CdpConnection,
    response_plan: CommandOutputPlan,
    protocol_events: lifecycle::CreatedTargetProtocolEvents,
) -> CommandOutputPlan {
    let target_id = protocol_events.target_id().to_owned();
    let mut plan = CommandOutputPlan::default();
    let mut events = Vec::new();
    if let Err(error) =
        lifecycle::emit_created_target_protocol_events(conn, protocol_events, &mut events)
    {
        return CommandOutputPlan::from_devtools_error(error);
    }
    lifecycle::start_target_url_navigation_if_allowed_background_events_async(
        conn,
        &mut events,
        &target_id,
    )
    .await;
    for event in events {
        plan.push_background_event(event);
    }
    plan.extend(response_plan);
    plan
}

pub(crate) async fn complete_pending_target_command(
    conn: &mut CdpConnection,
    completed: CompletedTargetCommandDispatch,
    command_context: &mut crate::conn::CommandDispatchContext,
) -> TargetCommandTaskStep {
    match completed.kind {
        CompletedTargetCommandKind::AttachToTarget {
            attached_session_id,
            target_info,
            initial_document,
        } => {
            return TargetCommandTaskStep::Complete(
                attachment::complete_attach_to_target_command_async(
                    conn,
                    completed.session_id.as_deref(),
                    attached_session_id,
                    target_info,
                    initial_document,
                )
                .await,
            );
        }
        CompletedTargetCommandKind::ActivateTarget { command } => {
            return TargetCommandTaskStep::Complete(
                lifecycle::complete_activate_target_command_async(conn, command).await,
            );
        }
        CompletedTargetCommandKind::SetAutoAttach {
            auto_attach,
            owner_session_id,
            owner_was_enabled,
            legacy_disable_all,
        } => {
            return TargetCommandTaskStep::Complete(
                lifecycle::complete_set_auto_attach_command_async(
                    conn,
                    auto_attach,
                    owner_session_id.as_deref(),
                    owner_was_enabled,
                    legacy_disable_all,
                    command_context,
                )
                .await,
            );
        }
        CompletedTargetCommandKind::CreateTarget {
            response_plan,
            protocol_events,
            initial_document_route,
            initial_document,
        } => {
            match initial_document {
                Some(Ok(completed_initial_document)) => {
                    let completed_initial_document = *completed_initial_document;
                    let result = if let Some(route) = initial_document_route {
                        let mut route_scope = conn.scoped_none_session_owner_route_override(route);
                        route_scope
                            .conn_mut()
                            .complete_initial_document_page_build_for_owner_with_creation_diagnostics(
                                completed_initial_document,
                            )
                            .await
                    } else {
                        conn.complete_initial_document_page_build_for_owner_with_creation_diagnostics(
                            completed_initial_document,
                        )
                        .await
                    };
                    match result {
                        Ok(diagnostics) => {
                            if let Some(predecessor) = diagnostics.renderer_output_predecessor {
                                command_context.set_renderer_output_predecessor(predecessor);
                            }
                        }
                        Err(message) => {
                            return TargetCommandTaskStep::Complete(CommandOutputPlan::error(
                                -32000, message,
                            ));
                        }
                    }
                }
                Some(Err(failed)) => {
                    let message = conn.reset_failed_initial_document_page_build_for_owner(failed);
                    return TargetCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000, message,
                    ));
                }
                None => {}
            }
            TargetCommandTaskStep::Complete(
                created_target_response_plan_after_initial_document(
                    conn,
                    response_plan,
                    protocol_events,
                )
                .await,
            )
        }
        CompletedTargetCommandKind::DetachFromTarget {
            target_id,
            detach_session_id,
        } => {
            return TargetCommandTaskStep::Complete(
                attachment::complete_detach_from_target_command_async(
                    conn,
                    completed.session_id.as_deref(),
                    target_id,
                    detach_session_id,
                    command_context,
                )
                .await,
            );
        }
        CompletedTargetCommandKind::CloseTarget { command } => {
            return TargetCommandTaskStep::Complete(
                lifecycle::complete_close_target_command_async(conn, command, command_context)
                    .await,
            );
        }
        CompletedTargetCommandKind::DisposeBrowserContext { browser_context_id } => {
            return TargetCommandTaskStep::Complete(
                browser_context::complete_dispose_browser_context_command_async(
                    conn,
                    browser_context_id,
                    command_context,
                )
                .await,
            );
        }
        CompletedTargetCommandKind::SendMessageToTarget {
            message,
            target_session_id,
        } => {
            return TargetCommandTaskStep::Complete(
                attachment::complete_send_message_to_target_command_async(
                    conn,
                    command_context,
                    message,
                    target_session_id,
                )
                .await,
            );
        }
    }
}

fn pending_activate_target_command(
    command_id: Option<u64>,
    session_id: Option<&str>,
    command: DevToolsActivateTargetCommand,
) -> TargetCommandTaskStep {
    TargetCommandTaskStep::Pending(PendingTargetCommandDispatch {
        command_id,
        session_id: session_id.map(str::to_owned),
        kind: Box::new(PendingTargetCommandKind::ActivateTarget { command }),
    })
}

fn pending_set_auto_attach_command(
    command_id: Option<u64>,
    session_id: Option<&str>,
    auto_attach: bool,
    owner_session_id: Option<&str>,
    owner_was_enabled: bool,
    legacy_disable_all: bool,
) -> TargetCommandTaskStep {
    TargetCommandTaskStep::Pending(PendingTargetCommandDispatch {
        command_id,
        session_id: session_id.map(str::to_owned),
        kind: Box::new(PendingTargetCommandKind::SetAutoAttach {
            auto_attach,
            owner_session_id: owner_session_id.map(str::to_owned),
            owner_was_enabled,
            legacy_disable_all,
        }),
    })
}

fn pending_detach_from_target_command(
    command_id: Option<u64>,
    session_id: Option<&str>,
    target_id: Option<String>,
    detach_session_id: Option<String>,
) -> TargetCommandTaskStep {
    TargetCommandTaskStep::Pending(PendingTargetCommandDispatch {
        command_id,
        session_id: session_id.map(str::to_owned),
        kind: Box::new(PendingTargetCommandKind::DetachFromTarget {
            target_id,
            detach_session_id,
        }),
    })
}

fn pending_close_target_command(
    command_id: Option<u64>,
    session_id: Option<&str>,
    command: DevToolsCloseTargetCommand,
) -> TargetCommandTaskStep {
    TargetCommandTaskStep::Pending(PendingTargetCommandDispatch {
        command_id,
        session_id: session_id.map(str::to_owned),
        kind: Box::new(PendingTargetCommandKind::CloseTarget { command }),
    })
}

fn pending_dispose_browser_context_command(
    command_id: Option<u64>,
    session_id: Option<&str>,
    browser_context_id: String,
) -> TargetCommandTaskStep {
    TargetCommandTaskStep::Pending(PendingTargetCommandDispatch {
        command_id,
        session_id: session_id.map(str::to_owned),
        kind: Box::new(PendingTargetCommandKind::DisposeBrowserContext { browser_context_id }),
    })
}

fn pending_send_message_to_target_command(
    command_id: Option<u64>,
    session_id: Option<&str>,
    message: String,
    target_session_id: Option<String>,
) -> TargetCommandTaskStep {
    TargetCommandTaskStep::Pending(PendingTargetCommandDispatch {
        command_id,
        session_id: session_id.map(str::to_owned),
        kind: Box::new(PendingTargetCommandKind::SendMessageToTarget {
            message,
            target_session_id,
        }),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetDiscoverTargetsParams {
    discover: bool,
    filter: Option<Vec<SetDiscoverTargetsFilterEntry>>,
}

#[derive(Deserialize)]
struct SetDiscoverTargetsFilterEntry {
    #[serde(default)]
    exclude: bool,
    #[serde(rename = "type")]
    target_type: Option<String>,
}

fn set_discover_targets(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> CommandOutputPlan {
    let params: SetDiscoverTargetsParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return CommandOutputPlan::error_without_session(-32602, "InvalidParams");
        }
    };
    if !params.discover
        && params
            .filter
            .as_ref()
            .is_some_and(|filter| !filter.is_empty())
    {
        return CommandOutputPlan::error_without_session(
            -32602,
            "Filter should not be present with `discover` is off",
        );
    }
    let mut plan = CommandOutputPlan::default();
    if params.discover {
        let filter = params.filter.map(|filter| {
            filter
                .into_iter()
                .map(|entry| DevToolsTargetFilterEntry {
                    exclude: entry.exclude,
                    target_type: entry.target_type,
                })
                .collect::<Vec<_>>()
        });
        let target_infos =
            match browser_context::devtools_target_infos_for_discovery(conn, filter.as_deref()) {
                Ok(target_infos) => target_infos,
                Err(error) => {
                    return CommandOutputPlan::from_devtools_error(error);
                }
            };
        conn.set_target_discovery_for_owner_from_devtools_filter(cmd.session_id, filter);
        let events =
            conn.initial_target_created_events_for_discovery_owner(cmd.session_id, target_infos);
        for event in events {
            plan.push_background_event(event);
        }
    } else {
        conn.clear_target_discovery_for_owner(cmd.session_id);
    }
    plan.push_success();
    plan
}

fn previously_active_browser_context_id(conn: &CdpConnection) -> Option<String> {
    conn.browser_context.as_ref().map(|bc| bc.id.clone())
}

fn restore_previously_active_browser_context(
    conn: &mut CdpConnection,
    browser_context_id: Option<&str>,
) {
    if let Some(browser_context_id) = browser_context_id
        && conn.has_browser_context_id(browser_context_id)
        && conn
            .browser_context
            .as_ref()
            .is_none_or(|bc| bc.id != browser_context_id)
    {
        let _ = conn.activate_browser_context_by_id(browser_context_id);
    }
}

fn select_browser_context_for_target(
    conn: &mut CdpConnection,
    target_id: &str,
) -> Result<(), &'static str> {
    if conn.browser_context.is_none() && conn.inactive_browser_contexts.is_empty() {
        return Err("BrowserContextNotLoaded");
    }
    if !conn.browser_contexts().any(|bc| {
        bc.has_active_target()
            || !bc.background_targets.is_empty()
            || bc.has_any_shared_worker_targets()
            || bc.has_any_dedicated_worker_targets()
            || bc.has_any_service_worker_targets()
    }) {
        return Err("TargetNotLoaded");
    }
    if conn.activate_browser_context_for_target(target_id) {
        return Ok(());
    }
    Err("UnknownTargetId")
}

#[cfg(test)]
mod devtools_runtime_entry_tests {
    use crate::devtools_runtime::{
        AutomationEvent, DevToolsActivateTargetCommand, DevToolsCloseTargetCommand,
        DevToolsCommand, DevToolsCommandContext, DevToolsCreateTargetCommand,
        DevToolsGetClientWindowsCommand, DevToolsGetTargetInfoCommand, DevToolsGetTargetsCommand,
        DevToolsProtocol, DevToolsTargetId, DevToolsTargetKind,
    };
    use serde_json::{Value, json};

    use super::*;

    fn cdp_context() -> DevToolsCommandContext {
        DevToolsCommandContext {
            protocol: DevToolsProtocol::Cdp,
            session_id: None,
            target_id: None,
            browser_context_id: None,
        }
    }

    fn complete_messages_for_test(
        step: TargetCommandTaskStep,
        command_id: u64,
    ) -> Vec<serde_json::Value> {
        let TargetCommandTaskStep::Complete(plan) = step else {
            panic!("expected complete Target command step");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, Some(command_id), None);
        out
    }

    async fn complete_messages_for_target_step_for_test(
        conn: &mut CdpConnection,
        mut step: TargetCommandTaskStep,
        command_id: u64,
    ) -> Vec<serde_json::Value> {
        let mut command_context = crate::conn::CommandDispatchContext::default();
        loop {
            match step {
                TargetCommandTaskStep::Complete(plan) => {
                    let mut out = Vec::new();
                    plan.emit_into(&mut out, Some(command_id), None);
                    return out;
                }
                TargetCommandTaskStep::Pending(pending) => {
                    step = complete_pending_target_command(
                        conn,
                        pending.wait().await,
                        &mut command_context,
                    )
                    .await;
                }
            }
        }
    }

    #[tokio::test]
    async fn devtools_target_entry_routes_create_target_to_initial_document_lifecycle_work() {
        let mut conn = CdpConnection::new();
        let step = start_devtools_target_command(
            &mut conn,
            Some(41),
            None,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: cdp_context(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );

        let TargetCommandTaskStep::Pending(pending) = step else {
            panic!("Target.createTarget should enter target lifecycle pending work");
        };
        assert_eq!(pending.command_id, Some(41));
        assert_eq!(pending.session_id.as_deref(), None);
        match &*pending.kind {
            PendingTargetCommandKind::CreateTarget {
                initial_document, ..
            } => {
                assert!(
                    initial_document.is_some(),
                    "Target.createTarget should start initial document page ensure"
                );
            }
            _ => panic!("Target.createTarget should preserve create-target lifecycle work"),
        }

        let out = complete_messages_for_target_step_for_test(
            &mut conn,
            TargetCommandTaskStep::Pending(pending),
            41,
        )
        .await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["id"], json!(41));
        assert!(out[0]["result"]["targetId"].as_str().is_some());
        assert!(
            conn.browser_context
                .as_ref()
                .expect("browser context")
                .active_target
                .runtime_slot
                .has_loaded_page()
        );
    }

    #[tokio::test]
    async fn attach_to_target_completion_plan_preserves_typed_attached_sidecar() {
        let mut conn = CdpConnection::new();
        let plan = attachment::complete_attach_to_target_command_async(
            &mut conn,
            Some("SID-parent"),
            "SID-child".to_owned(),
            DevToolsTargetInfo {
                target_id: Some(DevToolsTargetId::from("TID-child")),
                kind: DevToolsTargetKind::Page,
                title: String::new(),
                url: "about:blank".to_owned(),
                attached: true,
                opener_id: None,
                opener_frame_id: None,
                can_access_opener: false,
                browser_context_id: None,
                moli_popup_id: None,
            },
            None,
        )
        .await;

        let (status, mut protocol_events) = plan.into_command_status_and_background_events();
        status
            .expect("attach completion should record command status")
            .expect("attach completion should succeed");
        assert_eq!(protocol_events.len(), 1);

        let (message, automation_event) = protocol_events.remove(0).into_parts();
        assert_eq!(message["method"], json!("Target.attachedToTarget"));
        assert_eq!(message["sessionId"], json!("SID-parent"));
        assert_eq!(message["params"]["sessionId"], json!("SID-child"));
        assert!(
            message.get("id").is_none(),
            "attachedToTarget must not be emitted as a command response sidecar"
        );
        let Some(AutomationEvent::TargetAttached(event)) = automation_event else {
            panic!("expected typed TargetAttached sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-child");
        assert_eq!(event.session_id.as_str(), "SID-child");
        assert_eq!(
            event.parent_session_id.as_ref().map(|id| id.as_str()),
            Some("SID-parent")
        );
    }

    #[tokio::test]
    async fn devtools_target_legacy_close_drains_runtime_ready_events_without_serializing_them() {
        let mut conn = CdpConnection::new();
        let mut browser_context = BrowserContext::new("BID-runtime-ready-close".to_owned());
        browser_context.set_active_target_id("TID-runtime-ready-close");
        browser_context.attach_active_session("SID-runtime-ready-close");
        conn.browser_context = Some(browser_context);
        let page = conn
            .load_page_via_runtime_async("data:text/html,<p>runtime ready close</p>")
            .await
            .expect("page should load");
        conn.browser_context
            .as_mut()
            .expect("browser context")
            .active_target
            .runtime_slot
            .set_loaded_page_for_test(page);
        conn.register_pending_inspector_await(7101, Some("SID-runtime-ready-close"));
        assert!(
            conn.claim_pending_inspector_await_for_scheduler_deferred_reply(
                7101,
                Some("SID-runtime-ready-close"),
            )
            .is_some(),
            "test must cover scheduler-deferred Runtime await owner cleanup"
        );

        let (result, protocol_events) = execute_devtools_target_command_async_with_protocol_events(
            &mut conn,
            DevToolsCommand::CloseTarget(DevToolsCloseTargetCommand {
                context: cdp_context(),
                target_id: DevToolsTargetId::from("TID-runtime-ready-close"),
            }),
        )
        .await;

        let DevToolsCommandResult::CloseTarget(close_result) =
            result.expect("close target should succeed")
        else {
            panic!("expected close target result");
        };
        assert!(close_result.success);
        assert!(
            protocol_events.iter().all(|event| event
                .protocol_message()
                .and_then(|message| message.get("id"))
                != Some(&Value::Null)),
            "direct Target.closeTarget must not route its own command response as a protocol event"
        );
        assert!(
            protocol_events.iter().any(|event| event
                .as_runtime_inspector_response_ready()
                .is_some_and(|response| response.command_id() == 7101
                    && response.error() == Some("Target closed"))),
            "pending Runtime await cancellation must remain a typed runtime-ready event"
        );
        assert!(
            protocol_events.iter().all(|event| {
                event.protocol_message().is_none_or(|message| {
                    message.pointer("/error/message").and_then(Value::as_str)
                        != Some("InternalRuntimeInspectorResponseReadyNotRouted")
                })
            }),
            "legacy Target executor must not serialize runtime-ready events as internal errors"
        );
    }

    #[test]
    fn immediate_create_target_staging_does_not_emit_target_created_before_initial_document() {
        let mut conn = CdpConnection::new();
        conn.set_root_target_discovery_enabled(true);
        let (result, protocol_events) =
            execute_immediate_devtools_target_command_with_protocol_events(
                &mut conn,
                DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                    context: cdp_context(),
                    url: "about:blank".to_owned(),
                    browser_context_id: None,
                    activate: false,
                }),
            );

        let DevToolsCommandResult::CreateTarget(result) =
            result.expect("create target staging should succeed")
        else {
            panic!("expected create target result");
        };
        assert_eq!(result.target_id.as_str(), "TID-1");
        assert!(
            protocol_events.is_empty(),
            "Target.targetCreated must be generated after initial document Page build"
        );
        assert_eq!(
            conn.browser_context
                .as_ref()
                .expect("browser context")
                .pending_document_page_build_count(),
            1,
            "staging should leave the target explicitly pending initial document Page build"
        );
    }

    #[test]
    fn devtools_target_entry_routes_close_target_to_pending_command() {
        let mut conn = CdpConnection::new();
        let step = start_devtools_target_command(
            &mut conn,
            Some(42),
            Some("SID-1"),
            DevToolsCommand::CloseTarget(DevToolsCloseTargetCommand {
                context: cdp_context(),
                target_id: DevToolsTargetId::from("TARGET-1"),
            }),
        );

        let TargetCommandTaskStep::Pending(pending) = step else {
            panic!("Target.closeTarget should enter the pending path through the unified entry");
        };
        assert_eq!(pending.command_id, Some(42));
        assert_eq!(pending.session_id.as_deref(), Some("SID-1"));
        match *pending.kind {
            PendingTargetCommandKind::CloseTarget { command } => {
                assert_eq!(command.target_id.as_str(), "TARGET-1");
            }
            _ => panic!("Target.closeTarget should preserve the close command payload"),
        }
    }

    #[test]
    fn devtools_target_entry_routes_activate_target_to_pending_command() {
        let mut conn = CdpConnection::new();
        let step = start_devtools_target_command(
            &mut conn,
            Some(43),
            Some("SID-2"),
            DevToolsCommand::ActivateTarget(DevToolsActivateTargetCommand {
                context: cdp_context(),
                target_id: DevToolsTargetId::from("TARGET-2"),
            }),
        );

        let TargetCommandTaskStep::Pending(pending) = step else {
            panic!("Target.activateTarget should enter the pending path through the unified entry");
        };
        assert_eq!(pending.command_id, Some(43));
        assert_eq!(pending.session_id.as_deref(), Some("SID-2"));
        match *pending.kind {
            PendingTargetCommandKind::ActivateTarget { command } => {
                assert_eq!(command.target_id.as_str(), "TARGET-2");
            }
            _ => panic!("Target.activateTarget should preserve the activate command payload"),
        }
    }

    #[tokio::test]
    async fn devtools_target_entry_routes_get_targets_to_shared_result() {
        let mut conn = CdpConnection::new();
        let create_step = start_devtools_target_command(
            &mut conn,
            Some(40),
            None,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: cdp_context(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
        let _ = complete_messages_for_target_step_for_test(&mut conn, create_step, 40).await;
        let step = start_devtools_target_command(
            &mut conn,
            Some(44),
            None,
            DevToolsCommand::GetTargets(DevToolsGetTargetsCommand {
                context: cdp_context(),
                root: None,
                max_depth: None,
                filter: None,
            }),
        );

        let out = complete_messages_for_test(step, 44);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["id"], json!(44));
        let target_infos = out[0]["result"]["targetInfos"]
            .as_array()
            .expect("targetInfos array");
        assert_eq!(target_infos.len(), 1);
        assert!(target_infos[0]["browserContextId"].as_str().is_some());
        assert_eq!(target_infos[0]["type"], json!("page"));
    }

    #[tokio::test]
    async fn devtools_target_entry_routes_get_client_windows_to_shared_result() {
        let mut conn = CdpConnection::new();
        let first_create_step = start_devtools_target_command(
            &mut conn,
            Some(46),
            None,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: cdp_context(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: true,
            }),
        );
        let _ = complete_messages_for_target_step_for_test(&mut conn, first_create_step, 46).await;
        let second_create_step = start_devtools_target_command(
            &mut conn,
            Some(47),
            None,
            DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
                context: cdp_context(),
                url: "about:blank".to_owned(),
                browser_context_id: None,
                activate: false,
            }),
        );
        let _ = complete_messages_for_target_step_for_test(&mut conn, second_create_step, 47).await;

        let step = start_devtools_target_command(
            &mut conn,
            Some(45),
            None,
            DevToolsCommand::GetClientWindows(DevToolsGetClientWindowsCommand {
                context: cdp_context(),
            }),
        );

        let out = complete_messages_for_test(step, 45);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["id"], json!(45));
        let client_windows = out[0]["result"]["clientWindows"]
            .as_array()
            .expect("clientWindows array");
        assert_eq!(client_windows.len(), 2);
        assert_eq!(
            client_windows
                .iter()
                .filter(|window| window["active"] == json!(true))
                .count(),
            1
        );
        assert_ne!(
            client_windows[0]["clientWindow"],
            client_windows[1]["clientWindow"]
        );
    }

    #[test]
    fn devtools_target_entry_routes_get_target_info_to_shared_result() {
        let mut conn = CdpConnection::new();
        let step = start_devtools_target_command(
            &mut conn,
            Some(45),
            None,
            DevToolsCommand::GetTargetInfo(DevToolsGetTargetInfoCommand {
                context: cdp_context(),
                target_id: None,
            }),
        );

        let out = complete_messages_for_test(step, 45);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["id"], json!(45));
        assert_eq!(out[0]["result"]["targetInfo"]["type"], json!("browser"));
        assert_eq!(out[0]["result"]["targetInfo"]["targetId"], json!("browser"));
        assert_eq!(out[0]["result"]["targetInfo"]["url"], json!(""));
    }
}
