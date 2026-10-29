use anyhow::Result;
use serde::Deserialize;

use crate::automation::{
    AutomationCommand, AutomationResult, DevToolsActivateTargetCommand, DevToolsCloseTargetCommand,
    DevToolsCreateTargetCommand, DevToolsError, DevToolsErrorKind, DevToolsTargetFilterEntry,
    DevToolsTargetInfo, DevToolsTargetKind,
};
use crate::conn::{
    BackgroundProtocolEvent, BrowserContext, CdpConnection, Cmd, CommandOwnerScope,
    ParsedCdpCommand, TargetAttachSessionCommit, TargetHandlerAccessMode,
};
use crate::domains::actions::TargetAction;
use crate::domains::command_output::CommandOutputPlan;

pub(crate) mod activation;
mod automation;
pub(crate) use automation::{PreparedAutomationTargetCreation, start_automation_lifecycle_command};
mod attachment;
pub(crate) use attachment::commit_target_attachment;
pub(crate) use attachment::{TargetDetachmentFailureCleanup, TargetDetachmentRecovery};
mod auto_attach;
mod browser_context;
mod browser_context_disposal;
mod closing;
mod creation;
mod events;
mod info;
mod opening;
pub(crate) use attachment::TargetDetachmentOutput;
pub(crate) use auto_attach::{AutoAttachSweepRollback, PreparedAutoAttachSweep};
pub(crate) use browser_context_disposal::{
    BrowserContextDisposalIdentity, PreparedBrowserContextDisposal,
};
pub(crate) use opening::prepare_creation_outputs;
#[cfg(test)]
pub(crate) use opening::rollback_attachment;
mod popup;
#[cfg(test)]
mod protocol_neutral_tests;
pub(crate) mod session_disposal;
#[cfg(test)]
mod tests;
mod worker_target;
pub(crate) use creation::TargetCreationCommit;
pub(crate) use worker_target::{
    CapturedWorkerAttachment, DedicatedWorkerRetirementIdentity,
    PreparedDedicatedWorkerProjectionDisposal, PreparedDedicatedWorkerRetirement,
    RemovedSharedWorkerAttachment, WorkerRetirementFailureCleanup, WorkerTargetLifecycleOutput,
    WorkerTargetOutputProjection, prepare_worker_inspector_projection, project_worker_target_event,
    retain_events,
};

pub(crate) enum TargetCommandFailureCleanup {
    Detachment(TargetDetachmentFailureCleanup),
    PageClose(closing::PreparedPageTargetCloseCommand),
}

pub(in crate::domains) use browser_context::devtools_client_window_info_for_target;
pub(crate) use popup::{
    NewPopupTarget, PopupTargetCreation, PopupTargetOpenerIdentity, PreparedPopupNavigation,
    PreparedPopupTarget, emit_target_info_changed_for_owner_background_event, finish_popup_target,
    prepare_popup_target, prepare_popup_target_activation,
    schedule_initial_document_target_url_navigation_after_debugger_barrier_release_for_target,
    schedule_initial_document_target_url_navigation_after_debugger_resume,
};
pub(crate) fn popup_activation_creates_new_target_for_owner(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    target_name: &str,
) -> bool {
    if let Some((browser_context_id, _)) = conn.target_owner_identity_for_owner(owner) {
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
    release_failed_dedicated_worker_target_after_debugger_resume,
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
    kind: Box<CompletedTargetCommandKind>,
}

pub(crate) enum TargetCommandTaskStep {
    Pending(PendingTargetCommandDispatch),
    Workflow(crate::conn::PreparedCommandWorkflow),
    Complete(CommandOutputPlan),
}

pub(crate) use attachment::retain_target_message_output;

pub(super) fn target_command_error(
    code: i32,
    message: impl std::fmt::Display,
) -> TargetCommandTaskStep {
    TargetCommandTaskStep::Complete(CommandOutputPlan::error(code, message))
}

fn target_handler_access_error(
    conn: &CdpConnection,
    cmd: &Cmd<'_>,
    action: TargetAction,
) -> Option<TargetCommandTaskStep> {
    let access_mode = conn.target_handler_access_mode(cmd.session_id);
    let allowed = match action {
        action if target_action_requires_browser_access(action) => {
            access_mode == TargetHandlerAccessMode::Browser
        }
        TargetAction::GetTargets
        | TargetAction::CreateTarget
        | TargetAction::AttachToTarget
        | TargetAction::SetDiscoverTargets
        | TargetAction::ActivateTarget => access_mode != TargetHandlerAccessMode::AutoAttachOnly,
        _ => true,
    };
    (!allowed).then(|| {
        let message = if action == TargetAction::AutoAttachRelated {
            "Target.autoAttachRelated is only supported on the Browser target"
        } else {
            "Not allowed"
        };
        target_command_error(-32000, message)
    })
}

fn target_action_requires_browser_access(action: TargetAction) -> bool {
    matches!(
        action,
        TargetAction::GetBrowserContexts
            | TargetAction::CreateBrowserContext
            | TargetAction::AttachToBrowserTarget
            | TargetAction::AutoAttachRelated
            | TargetAction::DisposeBrowserContext
    )
}

/// Whether a nested Target message inherits the envelope's session.
///
/// Browser-only Target operations retain their unscoped browser access.
/// Explicit payload sessions are preserved for ordinary ownership and access
/// validation rather than replaced with an implicit session.
pub fn target_message_uses_implicit_session(command: &ParsedCdpCommand) -> bool {
    command.session_id().is_none()
        && !command
            .method()
            .strip_prefix("Target.")
            .and_then(|action| action.parse::<TargetAction>().ok())
            .is_some_and(target_action_requires_browser_access)
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

/// Disables Target-domain policy owned by one DevTools session.
pub(in crate::domains) fn dispose_session_handler(conn: &mut CdpConnection, session_id: &str) {
    conn.clear_auto_attach_owner(Some(session_id));
    conn.clear_target_discovery_for_owner(Some(session_id));
    set_service_worker_pause_on_start_owner(conn, Some(session_id), false);
    set_dedicated_worker_pause_on_start_owner(conn, Some(session_id), false);
}

enum PendingTargetCommandKind {
    AttachToTarget {
        prepared_session: TargetAttachSessionCommit,
        target_info: DevToolsTargetInfo,
        initial_document: Option<Box<crate::conn::PendingInitialDocumentPageBuild>>,
    },
    ActivateTarget {
        activation: activation::PreparedTargetActivation,
        access: crate::conn::ProtocolStateHandle,
    },

    CreateTarget {
        response_plan: CommandOutputPlan,
        creation_commit: creation::TargetCreationCommit,
        initial_document: Option<Box<crate::conn::PendingInitialDocumentPageBuild>>,
    },
    DetachFromTarget {
        detachment: attachment::TargetDetachment,
        access: crate::conn::ProtocolStateHandle,
    },
    RecoverDetachment(attachment::TargetDetachmentRecovery),

    ClosePageTarget {
        closing: closing::PreparedPageTargetCloseCommand,
        response: CommandOutputPlan,
        access: crate::conn::ProtocolStateHandle,
    },
}

enum CompletedTargetCommandKind {
    AttachToTarget {
        prepared_session: TargetAttachSessionCommit,
        target_info: DevToolsTargetInfo,
        initial_document: Option<
            Result<
                Box<crate::conn::CompletedInitialDocumentPageBuild>,
                crate::conn::FailedInitialDocumentPageBuild,
            >,
        >,
    },
    ActivateTarget(Result<activation::TargetActivationOutput>),

    CreateTarget {
        response_plan: CommandOutputPlan,
        creation_commit: creation::TargetCreationCommit,
        initial_document: Option<
            Result<
                Box<crate::conn::CompletedInitialDocumentPageBuild>,
                crate::conn::FailedInitialDocumentPageBuild,
            >,
        >,
    },
    DetachFromTarget(attachment::TargetDetachmentOutput),
    RecoveredDetachment(attachment::TargetDetachmentRecovery),

    ClosePageTarget {
        closing: closing::PreparedPageTargetCloseCommand,
        completed: Option<Box<crate::domains::page::CompletedTargetTermination>>,
        response: CommandOutputPlan,
    },
}

impl PendingTargetCommandDispatch {
    pub(crate) fn owns_session_disposal(&self) -> bool {
        matches!(
            *self.kind,
            PendingTargetCommandKind::DetachFromTarget { .. }
                | PendingTargetCommandKind::RecoverDetachment(_)
                | PendingTargetCommandKind::ClosePageTarget { .. }
        )
    }

    pub(crate) fn failure_cleanup(&self) -> Option<TargetCommandFailureCleanup> {
        match self.kind.as_ref() {
            PendingTargetCommandKind::DetachFromTarget { detachment, .. } => Some(
                TargetCommandFailureCleanup::Detachment(detachment.failure_cleanup()),
            ),
            PendingTargetCommandKind::ClosePageTarget { closing, .. } => {
                Some(TargetCommandFailureCleanup::PageClose(closing.clone()))
            }
            _ => None,
        }
    }

    pub(crate) fn take_start_protocol_events(&self) -> Vec<BackgroundProtocolEvent> {
        match self.kind.as_ref() {
            PendingTargetCommandKind::ClosePageTarget { closing, .. } => {
                closing.take_protocol_events()
            }
            _ => Vec::new(),
        }
    }

    pub(crate) async fn wait(self) -> CompletedTargetCommandDispatch {
        let kind = match *self.kind {
            PendingTargetCommandKind::RecoverDetachment(recovery) => {
                CompletedTargetCommandKind::RecoveredDetachment(recovery.wait().await)
            }
            PendingTargetCommandKind::AttachToTarget {
                prepared_session,
                target_info,
                initial_document,
            } => CompletedTargetCommandKind::AttachToTarget {
                prepared_session,
                target_info,
                initial_document: match initial_document {
                    Some(pending) => Some(pending.wait().await.map(Box::new)),
                    None => None,
                },
            },
            PendingTargetCommandKind::ActivateTarget { activation, access } => {
                CompletedTargetCommandKind::ActivateTarget(
                    activation
                        .run(&mut crate::conn::ProtocolWorkflowDriver::new(access))
                        .await,
                )
            }

            PendingTargetCommandKind::CreateTarget {
                response_plan,
                creation_commit,
                initial_document,
            } => CompletedTargetCommandKind::CreateTarget {
                response_plan,
                creation_commit,
                initial_document: match initial_document {
                    Some(pending) => Some(pending.wait().await.map(Box::new)),
                    None => None,
                },
            },
            PendingTargetCommandKind::DetachFromTarget { detachment, access } => {
                CompletedTargetCommandKind::DetachFromTarget(
                    detachment
                        .run(&mut crate::conn::ProtocolWorkflowDriver::new(access))
                        .await,
                )
            }

            PendingTargetCommandKind::ClosePageTarget {
                closing,
                response,
                access,
            } => {
                let completed = closing
                    .wait(&mut crate::conn::ProtocolWorkflowDriver::new(access))
                    .await;
                CompletedTargetCommandKind::ClosePageTarget {
                    closing,
                    completed,
                    response,
                }
            }
        };
        CompletedTargetCommandDispatch {
            command_id: self.command_id,
            session_id: self.session_id,
            kind: Box::new(kind),
        }
    }
}

impl CompletedTargetCommandDispatch {
    pub(crate) fn try_into_opening_workflow(
        self,
        access: crate::conn::ProtocolStateHandle,
    ) -> Result<crate::conn::PreparedCommandWorkflow, Self> {
        match *self.kind {
            CompletedTargetCommandKind::AttachToTarget {
                prepared_session,
                target_info,
                initial_document,
            } => {
                let recovery_session = prepared_session.clone();
                let recovery_access = access.clone();
                Ok(
                    crate::conn::PreparedCommandWorkflow::new(move |mut context| async move {
                        let result = Box::pin(opening::complete_attachment(
                            &mut crate::conn::ProtocolWorkflowDriver::new(access),
                            prepared_session,
                            target_info,
                            initial_document,
                            &mut context,
                        ))
                        .await;
                        match result {
                            Ok(plan) => context.complete(plan),
                            Err(error) => context
                                .fail(CommandOutputPlan::error_without_session(-32000, error)),
                        }
                    })
                    .with_failure_cleanup(move |response| {
                        opening::attachment_rollback_workflow(
                            recovery_access,
                            recovery_session,
                            response,
                        )
                    }),
                )
            }
            CompletedTargetCommandKind::CreateTarget {
                response_plan,
                creation_commit,
                initial_document,
            } => Ok(crate::conn::PreparedCommandWorkflow::new(
                move |mut context| async move {
                    let result = Box::pin(opening::complete_creation(
                        &mut crate::conn::ProtocolWorkflowDriver::new(access),
                        response_plan,
                        creation_commit,
                        initial_document,
                        &mut context,
                    ))
                    .await;
                    match result {
                        Ok(plan) => context.complete(plan),
                        Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
                    }
                },
            )),
            kind => Err(Self {
                kind: Box::new(kind),
                ..self
            }),
        }
    }

    pub(crate) fn finish_ready(
        self,
        conn: &mut CdpConnection,
        context: &mut crate::conn::CommandDispatchContext,
    ) -> CommandOutputPlan {
        match *self.kind {
            CompletedTargetCommandKind::ClosePageTarget {
                closing,
                completed,
                response,
            } => {
                context.take_command_failure_cleanup();
                closing.finish(conn, context, completed, response)
            }
            CompletedTargetCommandKind::ActivateTarget(output) => {
                activation::activation_result_plan(conn, output)
            }
            CompletedTargetCommandKind::DetachFromTarget(output) => {
                context.take_command_failure_cleanup();
                output.finish(conn, context)
            }
            CompletedTargetCommandKind::RecoveredDetachment(recovery) => recovery.finish(conn),
            CompletedTargetCommandKind::AttachToTarget { .. }
            | CompletedTargetCommandKind::CreateTarget { .. } => {
                unreachable!("Target opening owns its prepared workflow")
            }
        }
    }

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
    context: &crate::conn::CommandDispatchContext,
) -> Option<TargetCommandTaskStep> {
    let action = cmd.parse_action::<TargetAction>();
    if let Some(action) = action
        && let Some(error) = target_handler_access_error(conn, cmd, action)
    {
        return Some(error);
    }
    match action {
        Some(TargetAction::GetTargets) => {
            Some(browser_context::start_get_targets_command(conn, cmd))
        }
        Some(TargetAction::GetBrowserContexts) => Some(TargetCommandTaskStep::Complete(
            browser_context::get_browser_contexts(conn),
        )),
        Some(TargetAction::CreateBrowserContext) => Some(TargetCommandTaskStep::Complete(
            browser_context::create_browser_context(conn, cmd),
        )),
        Some(TargetAction::CreateTarget) => Some(creation::start_create_target_command(conn, cmd)),
        Some(TargetAction::AttachToTarget) => {
            Some(attachment::start_attach_to_target_command(conn, cmd))
        }
        Some(TargetAction::AttachToBrowserTarget) => Some(TargetCommandTaskStep::Complete(
            attachment::attach_to_browser_target_command(conn, cmd),
        )),
        Some(TargetAction::GetTargetInfo) => Some(TargetCommandTaskStep::Complete(
            info::get_target_info(conn, cmd),
        )),
        Some(TargetAction::SetDiscoverTargets) => Some(TargetCommandTaskStep::Complete(
            set_discover_targets(conn, cmd),
        )),
        Some(TargetAction::ActivateTarget) => {
            Some(activation::start_activate_target_command(conn, cmd))
        }
        Some(TargetAction::SetAutoAttach) => {
            Some(auto_attach::start_set_auto_attach_command(conn, cmd))
        }
        Some(TargetAction::AutoAttachRelated) => Some(TargetCommandTaskStep::Complete(
            auto_attach::auto_attach_related(conn, cmd),
        )),
        Some(TargetAction::DetachFromTarget) => {
            Some(attachment::start_detach_from_target_command(conn, cmd))
        }
        Some(TargetAction::CloseTarget) => Some(closing::start_close_target_command(conn, cmd)),
        Some(TargetAction::DisposeBrowserContext) => Some(
            browser_context::start_dispose_browser_context_command(conn, cmd),
        ),
        Some(TargetAction::SendMessageToTarget) => Some(
            attachment::start_send_message_to_target_command(conn, cmd, context),
        ),
        None => Some(target_command_error(-32601, "UnknownMethod")),
    }
}

fn start_devtools_target_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: AutomationCommand,
) -> TargetCommandTaskStep {
    match command {
        AutomationCommand::CreateTarget(command) => creation::start_devtools_create_target_command(
            conn,
            command_id,
            command_session_id,
            command,
        ),
        AutomationCommand::CloseTarget(command) => closing::start_devtools_close_target_command(
            conn,
            command_id,
            command_session_id,
            command,
        ),
        AutomationCommand::ActivateTarget(command) => {
            pending_activate_target_command(conn, command_id, command_session_id, command)
        }
        AutomationCommand::GetTargets(command) => TargetCommandTaskStep::Complete(
            browser_context::start_devtools_get_targets_command(conn, command),
        ),
        AutomationCommand::GetServiceWorkerLogs(command) => TargetCommandTaskStep::Complete(
            match browser_context::execute_devtools_get_service_worker_logs_command(conn, &command)
            {
                Ok(result) => CommandOutputPlan::from_devtools_result(
                    AutomationResult::ServiceWorkerLogs(result),
                ),
                Err(error) => CommandOutputPlan::from_devtools_error(error),
            },
        ),
        AutomationCommand::GetClientWindows(command) => TargetCommandTaskStep::Complete(
            match browser_context::execute_devtools_get_client_windows_command(conn, &command) {
                Ok(result) => {
                    CommandOutputPlan::from_devtools_result(AutomationResult::ClientWindows(result))
                }
                Err(error) => CommandOutputPlan::from_devtools_error(error),
            },
        ),
        AutomationCommand::CreateBrowserContext(command) => {
            let plan = match browser_context::execute_devtools_create_browser_context_command(
                conn, command,
            ) {
                Ok(result) => CommandOutputPlan::from_devtools_result(
                    AutomationResult::CreateBrowserContext(result),
                ),
                Err(error) => CommandOutputPlan::from_devtools_error(error),
            };
            TargetCommandTaskStep::Complete(plan)
        }
        AutomationCommand::GetBrowserContexts(command) => TargetCommandTaskStep::Complete(
            CommandOutputPlan::from_devtools_result(AutomationResult::GetBrowserContexts(
                browser_context::devtools_get_browser_contexts_result(conn, &command),
            )),
        ),
        AutomationCommand::GetTargetInfo(command) => TargetCommandTaskStep::Complete(
            info::start_devtools_get_target_info_command(conn, command),
        ),
        _ => target_command_error(-32000, "UnsupportedDevToolsCommand"),
    }
}

pub(crate) fn execute_immediate_devtools_target_command_with_protocol_events(
    conn: &mut CdpConnection,
    command: AutomationCommand,
) -> (
    Result<AutomationResult, DevToolsError>,
    Vec<crate::conn::BackgroundProtocolEvent>,
) {
    match command {
        AutomationCommand::CreateTarget(command) => {
            let result = creation::execute_devtools_create_target_command(conn, command)
                .map(|execution| AutomationResult::CreateTarget(execution.result));
            (result, Vec::new())
        }
        AutomationCommand::GetTargets(command) => (
            browser_context::execute_devtools_get_targets_command(conn, &command)
                .map(AutomationResult::GetTargets),
            Vec::new(),
        ),
        AutomationCommand::GetServiceWorkerLogs(command) => (
            browser_context::execute_devtools_get_service_worker_logs_command(conn, &command)
                .map(AutomationResult::ServiceWorkerLogs),
            Vec::new(),
        ),
        AutomationCommand::GetClientWindows(command) => (
            browser_context::execute_devtools_get_client_windows_command(conn, &command)
                .map(AutomationResult::ClientWindows),
            Vec::new(),
        ),
        AutomationCommand::CreateBrowserContext(command) => (
            browser_context::execute_devtools_create_browser_context_command(conn, command)
                .map(AutomationResult::CreateBrowserContext),
            Vec::new(),
        ),
        AutomationCommand::GetBrowserContexts(command) => (
            Ok(AutomationResult::GetBrowserContexts(
                browser_context::devtools_get_browser_contexts_result(conn, &command),
            )),
            Vec::new(),
        ),
        AutomationCommand::GetTargetInfo(command) => (
            info::execute_devtools_get_target_info_command(conn, command)
                .map(AutomationResult::GetTargetInfo),
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

pub(crate) fn recover_failed_target_command(
    conn: &mut CdpConnection,
    context: &mut crate::conn::CommandDispatchContext,
    cleanup: TargetCommandFailureCleanup,
    command_id: Option<u64>,
    session_id: Option<&str>,
    response: CommandOutputPlan,
) -> PendingTargetCommandDispatch {
    let kind = match cleanup {
        TargetCommandFailureCleanup::Detachment(cleanup) => {
            PendingTargetCommandKind::RecoverDetachment(cleanup.prepare(conn, context, response))
        }
        TargetCommandFailureCleanup::PageClose(closing) => {
            PendingTargetCommandKind::ClosePageTarget {
                closing,
                response,
                access: conn.protocol_state_access(),
            }
        }
    };
    PendingTargetCommandDispatch {
        command_id,
        session_id: session_id.map(str::to_owned),
        kind: Box::new(kind),
    }
}

fn pending_activate_target_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    session_id: Option<&str>,
    command: DevToolsActivateTargetCommand,
) -> TargetCommandTaskStep {
    let activation = match activation::prepare_target_activation(conn, command) {
        Ok(Some(activation)) => activation,
        Ok(None) => {
            return TargetCommandTaskStep::Complete(CommandOutputPlan::result(serde_json::json!(
                {}
            )));
        }
        Err(error) => {
            return TargetCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(error));
        }
    };
    TargetCommandTaskStep::Pending(PendingTargetCommandDispatch {
        command_id,
        session_id: session_id.map(str::to_owned),
        kind: Box::new(PendingTargetCommandKind::ActivateTarget {
            activation,
            access: conn.protocol_state_access(),
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

#[derive(Debug, thiserror::Error)]
enum TargetSelectionError {
    #[error("BrowserContextNotLoaded")]
    BrowserContextNotLoaded,
    #[error("TargetNotLoaded")]
    TargetNotLoaded,
    #[error("UnknownTargetId")]
    UnknownTarget,
}

fn select_browser_context_for_target(conn: &mut CdpConnection, target_id: &str) -> Result<()> {
    if conn.browser_context.is_none() && conn.inactive_browser_contexts.is_empty() {
        anyhow::bail!(TargetSelectionError::BrowserContextNotLoaded);
    }
    if !conn.browser_contexts().any(|bc| {
        bc.active_target_id().is_some()
            || !bc.has_no_background_targets()
            || bc.has_any_shared_worker_targets()
            || bc.has_any_dedicated_worker_targets()
            || bc.has_any_service_worker_targets()
    }) {
        anyhow::bail!(TargetSelectionError::TargetNotLoaded);
    }
    if conn.activate_browser_context_for_target(target_id) {
        return Ok(());
    }
    anyhow::bail!(TargetSelectionError::UnknownTarget)
}

#[cfg(test)]
mod devtools_runtime_entry_tests {
    use crate::automation::{
        AutomationCommand, AutomationContext, AutomationEvent, DevToolsActivateTargetCommand,
        DevToolsCloseTargetCommand, DevToolsCreateTargetCommand, DevToolsGetClientWindowsCommand,
        DevToolsGetTargetInfoCommand, DevToolsGetTargetsCommand, DevToolsTargetId,
        DevToolsTargetKind, FrontendProtocol,
    };
    use crate::conn::RendererCommandDescriptor;
    use serde_json::{Value, json};

    use super::*;

    fn cdp_context() -> AutomationContext {
        AutomationContext {
            protocol: FrontendProtocol::Cdp,
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
        ctx: &mut crate::testing::TestContext,
        mut step: TargetCommandTaskStep,
        command_id: u64,
    ) -> Vec<serde_json::Value> {
        let mut command_context = crate::conn::CommandDispatchContext::default();
        loop {
            match step {
                TargetCommandTaskStep::Complete(plan) => {
                    let step = ctx
                        .state
                        .with_state(|state| {
                            state.finish_fixture_command_output_plan(
                                plan,
                                &mut command_context,
                                Some(command_id),
                                None,
                            )
                        })
                        .expect("live test owner");
                    let start = ctx.sent.len();
                    let (messages, scheduler_events) = ctx
                        .complete_command_task_step_with_context_for_test(
                            step,
                            &mut command_context,
                        )
                        .await;
                    let mut output: Vec<_> = ctx.sent.drain(start..).collect();
                    output.extend(messages);
                    ctx.route_direct_command_output_for_test(Vec::new(), scheduler_events)
                        .await;
                    output.extend(ctx.sent.drain(start..));
                    return output;
                }
                TargetCommandTaskStep::Pending(pending) => {
                    let completed = ctx.drive_owned_wait(pending.wait()).await;
                    step = match completed.try_into_opening_workflow(ctx.handle()) {
                        Ok(workflow) => TargetCommandTaskStep::Workflow(workflow),
                        Err(completed) => {
                            TargetCommandTaskStep::Complete(ctx.with_fixture(|conn| {
                                completed.finish_ready(conn, &mut command_context)
                            }))
                        }
                    };
                }
                TargetCommandTaskStep::Workflow(workflow) => {
                    let pending = workflow.start(&mut command_context, Some(command_id), None);
                    let completed = ctx.drive_owned_wait(pending.wait()).await;
                    step = TargetCommandTaskStep::Complete(
                        ctx.with_fixture(|conn| completed.finish(conn, &mut command_context)),
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn devtools_target_entry_routes_create_target_to_initial_document_lifecycle_work() {
        let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
        let step = ctx.with_fixture(|conn| {
            start_devtools_target_command(
                conn,
                Some(41),
                None,
                AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                    context: cdp_context(),
                    url: "about:blank".to_owned(),
                    browser_context_id: None,
                    activate: false,
                }),
            )
        });

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
            &mut ctx,
            TargetCommandTaskStep::Pending(pending),
            41,
        )
        .await;
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["id"], json!(41));
        assert!(out[0]["result"]["targetId"].as_str().is_some());
        ctx.with_fixture(|conn| {
            assert!({
                conn.browser_context
                    .as_ref()
                    .expect("browser context")
                    .active_page_target()
                    .runtime_slot
                    .has_loaded_page()
            })
        });
    }

    #[tokio::test]
    async fn attach_to_target_completion_plan_preserves_typed_attached_sidecar() {
        let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
        let mut browser = BrowserContext::new("BID-child".into());
        browser.set_active_target_id("TID-child");
        browser.attach_active_session("SID-child");
        ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
        ctx.with_fixture(|conn| {
            conn.register_session_route_for_test(
                "SID-parent",
                crate::conn::CdpSessionRoute::Browser,
            )
        });
        let handle = ctx.handle();
        let workflow = crate::conn::PreparedCommandWorkflow::new(move |mut context| async move {
            let plan = opening::complete_attachment(
                &mut crate::conn::ProtocolWorkflowDriver::new(handle),
                TargetAttachSessionCommit::direct(
                    "SID-child",
                    Some("SID-parent".to_owned()),
                    crate::conn::CdpSessionRoute::PageTarget {
                        browser_context_id: "BID-child".to_owned(),
                        target_id: "TID-child".to_owned(),
                        session_key: moli_page_types::DevToolsSessionKey::Primary,
                    },
                    false,
                ),
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
                &mut context,
            )
            .await
            .expect("live attachment owner");
            context.complete(plan)
        });
        let mut command_context = crate::conn::CommandDispatchContext::default();
        let pending = workflow.start(&mut command_context, None, Some("SID-parent".to_owned()));
        let completed = ctx.drive_owned_wait(pending.wait()).await;
        let plan = ctx.with_fixture(|conn| completed.finish(conn, &mut command_context));

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
    async fn devtools_target_close_drains_runtime_ready_events_without_serializing_them() {
        let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
        let mut browser_context = BrowserContext::new("BID-runtime-ready-close".to_owned());
        browser_context.set_active_target_id("TID-runtime-ready-close");
        browser_context.attach_active_session("SID-runtime-ready-close");
        assert!(browser_context.assign_attached_session_to_target(
            "TID-runtime-ready-close",
            "SID-runtime-ready-close-attached".to_owned(),
        ));
        ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser_context));
        let page = ctx
            .build_fixture_page("data:text/html,<p>runtime ready close</p>")
            .await
            .expect("page should load");
        ctx.with_fixture(|conn| {
            conn.browser_context
                .as_mut()
                .expect("browser context")
                .active_page_target_mut()
                .runtime_slot
                .set_loaded_page_for_test(page)
        });
        ctx.with_fixture(|conn| {
            conn.register_pending_inspector_await(7101, Some("SID-runtime-ready-close"))
        });
        ctx.with_fixture(|conn| {
            assert!(
                conn.claim_pending_inspector_await_for_scheduler_deferred_reply(
                    7101,
                    &crate::conn::CommandOwnerScope::for_session("SID-runtime-ready-close"),
                )
                .is_some(),
                "test must cover scheduler-deferred Runtime await owner cleanup"
            )
        });
        ctx.with_fixture(|conn| {
            conn.register_pending_inspector_await(7102, Some("SID-runtime-ready-close-attached"))
        });
        let attached_dispatch = ctx.with_fixture(|conn| {
            conn.try_register_renderer_call_for_session_owner(
                Some("SID-runtime-ready-close-attached"),
                7102,
                None,
                RendererCommandDescriptor::from_synthesized_payload(
                    json!({
                        "id": 7102,
                        "method": "Runtime.evaluate",
                        "params": { "expression": "new Promise(() => {})" },
                    })
                    .to_string(),
                )
                .expect("test Runtime command should parse"),
            )
            .expect("attached renderer command should register")
        });
        ctx.with_fixture(|conn| {
            assert!(
                conn.claim_pending_inspector_await_for_scheduler_deferred_reply(
                    7102,
                    &crate::conn::CommandOwnerScope::for_session(
                        "SID-runtime-ready-close-attached",
                    ),
                )
                .is_some(),
                "test must cover an attached scheduler-deferred Runtime await"
            )
        });

        let (result, scheduler, protocol_events, _predecessor) = ctx
            .execute_owned_automation_command_for_test(AutomationCommand::CloseTarget(
                DevToolsCloseTargetCommand {
                    context: cdp_context(),
                    target_id: DevToolsTargetId::from("TID-runtime-ready-close"),
                },
            ))
            .await
            .into_complete_parts();
        ctx.with_fixture(|conn| conn.extend_scheduler_events(scheduler));

        let AutomationResult::CloseTarget(close_result) =
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
                    && response.error().is_some_and(|error| matches!(
                        error.downcast_ref::<crate::conn::RuntimeInspectorError>(),
                        Some(crate::conn::RuntimeInspectorError::ResponseCanceled)
                    ))
                    && response
                        .clone()
                        .into_protocol_message_for_typed_runtime_route()["error"]["message"]
                        == "Target closed")),
            "pending Runtime await cancellation must remain a typed runtime-ready event"
        );
        assert!(
            protocol_events.iter().any(|event| event
                .as_runtime_inspector_response_ready()
                .is_some_and(|response| response.command_id() == 7102
                    && response.error().is_some_and(|error| matches!(
                        error.downcast_ref::<crate::conn::RuntimeInspectorError>(),
                        Some(crate::conn::RuntimeInspectorError::ResponseCanceled)
                    ))
                    && response
                        .clone()
                        .into_protocol_message_for_typed_runtime_route()["error"]["message"]
                        == "Target closed"
                    && response.has_bound_renderer_call_id())),
            "an attached claimed await must settle through its correlated typed response before the Page route retires"
        );
        drop(attached_dispatch);
        assert!(
            protocol_events.iter().all(|event| {
                event.protocol_message().is_none_or(|message| {
                    message.pointer("/error/message").and_then(Value::as_str)
                        != Some("InternalRuntimeInspectorResponseReadyNotRouted")
                })
            }),
            "Target executor must not serialize runtime-ready events as internal errors"
        );
    }

    #[test]
    fn immediate_create_target_staging_does_not_emit_target_created_before_initial_document() {
        let mut conn = CdpConnection::new();
        conn.set_root_target_discovery_enabled(true);
        let (result, protocol_events) =
            execute_immediate_devtools_target_command_with_protocol_events(
                &mut conn,
                AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                    context: cdp_context(),
                    url: "about:blank".to_owned(),
                    browser_context_id: None,
                    activate: false,
                }),
            );

        let AutomationResult::CreateTarget(result) =
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
    fn devtools_target_entry_rejects_missing_close_target_during_admission() {
        let owner = crate::conn::ProtocolStateOwner::new(CdpConnection::new());
        let step = owner
            .with_state(|state| {
                state.fixture(|conn| {
                    start_devtools_target_command(
                        conn,
                        Some(42),
                        Some("SID-1"),
                        AutomationCommand::CloseTarget(DevToolsCloseTargetCommand {
                            context: cdp_context(),
                            target_id: DevToolsTargetId::from("TARGET-1"),
                        }),
                    )
                })
            })
            .unwrap();
        let TargetCommandTaskStep::Complete(plan) = step else {
            panic!("a missing close target has no renderer work to wait for");
        };
        let messages: Vec<_> = plan
            .into_background_events(Some(42), Some("SID-1"))
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["id"], 42);
        assert_eq!(messages[0]["sessionId"], "SID-1");
        assert_eq!(messages[0]["error"]["code"], -31998);
    }

    #[test]
    fn devtools_target_entry_rejects_missing_activation_target_during_admission() {
        let mut conn = CdpConnection::new();
        let step = start_devtools_target_command(
            &mut conn,
            Some(43),
            Some("SID-2"),
            AutomationCommand::ActivateTarget(DevToolsActivateTargetCommand {
                context: cdp_context(),
                target_id: DevToolsTargetId::from("TARGET-2"),
            }),
        );

        let TargetCommandTaskStep::Complete(plan) = step else {
            panic!("a missing activation target has no renderer work to wait for");
        };
        let messages: Vec<_> = plan
            .into_background_events(Some(43), Some("SID-2"))
            .into_iter()
            .map(BackgroundProtocolEvent::into_protocol_message)
            .collect();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["id"], 43);
        assert_eq!(messages[0]["sessionId"], "SID-2");
        assert!(messages[0].get("error").is_some());
    }

    #[tokio::test]
    async fn devtools_target_entry_routes_get_targets_to_shared_result() {
        let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
        let create_step = ctx.with_fixture(|conn| {
            start_devtools_target_command(
                conn,
                Some(40),
                None,
                AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                    context: cdp_context(),
                    url: "about:blank".to_owned(),
                    browser_context_id: None,
                    activate: false,
                }),
            )
        });
        let _ = complete_messages_for_target_step_for_test(&mut ctx, create_step, 40).await;
        let step = ctx.with_fixture(|conn| {
            start_devtools_target_command(
                conn,
                Some(44),
                None,
                AutomationCommand::GetTargets(DevToolsGetTargetsCommand {
                    context: cdp_context(),
                    root: None,
                    max_depth: None,
                    filter: None,
                }),
            )
        });

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
        let mut ctx = crate::testing::TestContext::from_conn(CdpConnection::new());
        let first_create_step = ctx.with_fixture(|conn| {
            start_devtools_target_command(
                conn,
                Some(46),
                None,
                AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                    context: cdp_context(),
                    url: "about:blank".to_owned(),
                    browser_context_id: None,
                    activate: true,
                }),
            )
        });
        let _ = complete_messages_for_target_step_for_test(&mut ctx, first_create_step, 46).await;
        let second_create_step = ctx.with_fixture(|conn| {
            start_devtools_target_command(
                conn,
                Some(47),
                None,
                AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                    context: cdp_context(),
                    url: "about:blank".to_owned(),
                    browser_context_id: None,
                    activate: false,
                }),
            )
        });
        let _ = complete_messages_for_target_step_for_test(&mut ctx, second_create_step, 47).await;

        let step = ctx.with_fixture(|conn| {
            start_devtools_target_command(
                conn,
                Some(45),
                None,
                AutomationCommand::GetClientWindows(DevToolsGetClientWindowsCommand {
                    context: cdp_context(),
                }),
            )
        });

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
            AutomationCommand::GetTargetInfo(DevToolsGetTargetInfoCommand {
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
