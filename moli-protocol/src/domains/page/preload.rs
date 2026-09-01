use crate::devtools_runtime::{
    DevToolsAddPreloadScriptCommand, DevToolsAddPreloadScriptResult, DevToolsCommand,
    DevToolsCommandResult, DevToolsError, DevToolsErrorKind, DevToolsPreloadScriptId,
    DevToolsPreloadScriptSource, DevToolsProtocol, DevToolsRemovePreloadScriptCommand,
    DevToolsTargetId, DevToolsTargetKind,
};
use chromiumoxide_cdp::cdp::browser_protocol::page::{
    AddScriptToEvaluateOnNewDocumentParams, RemoveScriptToEvaluateOnNewDocumentParams,
};
use serde::Deserialize;
use serde_json::json;

use crate::conn::{
    BackgroundProtocolEvent, BrowserContext, CdpConnection, CdpSessionRoute, Cmd,
    CommandDispatchContext, CommandOwnerScope, DocumentStartScript,
};
use crate::domains::command_output::CommandOutputPlan;
use crate::domains::runtime::bidi_preload_function_declaration_source;

use super::{PageCommandTaskStep, PendingPageCommandDispatch, PendingPageCommandKind};
use moli_core::page::{CompletedPageCommand, PendingPageCommand, RendererAgentAttachmentId};
use moli_page_types::DevToolsSessionKey;

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct CreateIsolatedWorldParams {
    frame_id: String,
    world_name: String,
    // Chromium's CDP schema contains this historical misspelling.
    #[serde(
        default,
        rename = "grantUniveralAccess",
        alias = "grantUniversalAccess"
    )]
    grant_universal_access: bool,
}

pub(super) struct PendingCreateIsolatedWorldCommand {
    task: CreateIsolatedWorldCommandTask,
    pending: PendingCreateIsolatedWorldPhase,
}

pub(super) struct CompletedCreateIsolatedWorldCommand {
    task: CreateIsolatedWorldCommandTask,
    completed: CompletedCreateIsolatedWorldPhase,
}

pub(super) struct PendingAddScriptToEvaluateOnNewDocumentCommand {
    command: DevToolsAddPreloadScriptCommand,
}

pub(super) struct CompletedAddScriptToEvaluateOnNewDocumentCommand {
    command: DevToolsAddPreloadScriptCommand,
}

struct RecordedDocumentStartScript {
    identifier: String,
    script: DocumentStartScript,
    inserted: bool,
}

enum PendingCreateIsolatedWorldPhase {
    InitialDocumentNavigation(Box<super::navigation::PendingNavigateLoadCommand>),
    InitialDocumentNavigationContinue(
        Box<super::navigation::PendingContinueNavigationWithoutRequestPauseCommand>,
    ),
    RendererPageCommand(PendingPageCommand),
}

enum CompletedCreateIsolatedWorldPhase {
    InitialDocumentNavigation(Box<super::navigation::CompletedNavigateLoadCommand>),
    InitialDocumentNavigationContinue(
        Box<super::navigation::CompletedContinueNavigationWithoutRequestPauseCommand>,
    ),
    RendererPageCommand(Box<Result<CompletedPageCommand, String>>),
}

struct CreateIsolatedWorldCommandTask {
    target_id: String,
    params: CreateIsolatedWorldParams,
    has_bidi_channel_argument: bool,
    prefix_output: CommandOutputPlan,
    pending_renderer_agent_attachment_id: Option<RendererAgentAttachmentId>,
    phase: CreateIsolatedWorldPhase,
}

#[derive(Clone)]
enum CreateIsolatedWorldPhase {
    InitialDocumentNavigation,
    RuntimeActivity,
}

impl PendingCreateIsolatedWorldCommand {
    pub(super) async fn wait(self) -> CompletedCreateIsolatedWorldCommand {
        let completed = match self.pending {
            PendingCreateIsolatedWorldPhase::InitialDocumentNavigation(pending) => {
                CompletedCreateIsolatedWorldPhase::InitialDocumentNavigation(Box::new(
                    pending.wait().await,
                ))
            }
            PendingCreateIsolatedWorldPhase::InitialDocumentNavigationContinue(pending) => {
                CompletedCreateIsolatedWorldPhase::InitialDocumentNavigationContinue(Box::new(
                    pending.wait().await,
                ))
            }
            PendingCreateIsolatedWorldPhase::RendererPageCommand(pending) => {
                CompletedCreateIsolatedWorldPhase::RendererPageCommand(Box::new(
                    pending.wait().await.map_err(|error| error.to_string()),
                ))
            }
        };
        CompletedCreateIsolatedWorldCommand {
            task: self.task,
            completed,
        }
    }
}

impl PendingAddScriptToEvaluateOnNewDocumentCommand {
    pub(super) async fn wait(self) -> CompletedAddScriptToEvaluateOnNewDocumentCommand {
        CompletedAddScriptToEvaluateOnNewDocumentCommand {
            command: self.command,
        }
    }
}

async fn append_loaded_page_document_start_script_for_owner_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    script: &DocumentStartScript,
) -> Result<(), String> {
    let renderer_runtime_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let slot = conn.runtime_session_owner_slot_mut_for_route(
        owner.session_id(),
        owner.session_owner_route(),
    )?;
    if let Some(page) = slot.loaded_page_mut() {
        page.add_document_start_script_runtime_activity_async(
            renderer_runtime_inspector_session_id.as_deref(),
            script,
            false,
        )
        .await
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn remove_loaded_page_document_start_script_for_owner_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    registry_key: &str,
) -> Result<(), String> {
    let slot = conn.runtime_session_owner_slot_mut_for_route(
        owner.session_id(),
        owner.session_owner_route(),
    )?;
    if let Some(page) = slot.loaded_page_mut() {
        page.remove_document_start_script_by_registry_key_async(registry_key)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn start_default_document_start_script_append(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    identifier: String,
    script: &DocumentStartScript,
) -> PageCommandTaskStep {
    let renderer_runtime_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_session(command_session_id);
    let Some(page) = conn
        .runtime_session_owner_slot_mut(command_session_id)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return PageCommandTaskStep::Complete(add_preload_script_result_plan(identifier));
    };
    match page.start_add_document_start_script_runtime_activity(
        renderer_runtime_inspector_session_id.as_deref(),
        script,
        false,
    ) {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id,
            owner_scope: crate::conn::CommandOwnerScope::capture(conn, command_session_id),
            kind: Box::new(PendingPageCommandKind::AppendDefaultDocumentStartScript {
                identifier,
                pending,
            }),
        }),
        Err(error) => {
            PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

fn start_document_start_script_remove(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    registry_key: Option<String>,
) -> PageCommandTaskStep {
    let Some(registry_key) = registry_key else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::success());
    };
    let Some(page) = conn
        .runtime_session_owner_slot_mut(command_session_id)
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::success());
    };
    match page.start_remove_document_start_script_by_registry_key(&registry_key) {
        Ok(pending) => PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id,
            owner_scope: crate::conn::CommandOwnerScope::capture(conn, command_session_id),
            kind: Box::new(PendingPageCommandKind::RemoveDocumentStartScript { pending }),
        }),
        Err(error) => {
            PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

pub(super) fn add_preload_script_result_plan(identifier: String) -> CommandOutputPlan {
    CommandOutputPlan::from_devtools_result(DevToolsCommandResult::AddPreloadScript(
        DevToolsAddPreloadScriptResult {
            script_id: DevToolsPreloadScriptId::from(identifier),
        },
    ))
}

fn push_background_events(
    plan: &mut CommandOutputPlan,
    events: impl IntoIterator<Item = BackgroundProtocolEvent>,
) {
    for event in events {
        plan.push_background_event(event);
    }
}

fn build_cdp_add_preload_script_command(
    cmd: &Cmd<'_>,
    target_id: Option<&str>,
    browser_context_id: Option<&str>,
    params: AddScriptToEvaluateOnNewDocumentParams,
) -> DevToolsAddPreloadScriptCommand {
    DevToolsAddPreloadScriptCommand {
        context: cmd.devtools_command_context(target_id, browser_context_id),
        source: DevToolsPreloadScriptSource::RawScript(params.source),
        world_name: params.world_name,
        target_ids: target_id.map(|target_id| vec![target_id.into()]),
        browser_context_ids: browser_context_id
            .map(|browser_context_id| vec![browser_context_id.into()])
            .unwrap_or_default(),
        run_immediately: params.run_immediately.unwrap_or(false),
        include_command_line_api: params.include_command_line_api.unwrap_or(false),
    }
}

fn build_cdp_remove_preload_script_command(
    cmd: &Cmd<'_>,
    target_id: Option<&str>,
    browser_context_id: Option<&str>,
    params: RemoveScriptToEvaluateOnNewDocumentParams,
) -> DevToolsRemovePreloadScriptCommand {
    DevToolsRemovePreloadScriptCommand {
        context: cmd.devtools_command_context(target_id, browser_context_id),
        script_id: DevToolsPreloadScriptId::from(params.identifier.as_ref()),
    }
}

fn document_start_script_from_add_preload_command(
    command: &DevToolsAddPreloadScriptCommand,
) -> Result<DocumentStartScript, DevToolsError> {
    match &command.source {
        DevToolsPreloadScriptSource::RawScript(source) => Ok(DocumentStartScript {
            registry_key: None,
            devtools_session: None,
            source: source.clone(),
            world_name: command.world_name.clone(),
            has_bidi_channel_argument: false,
            bidi_channel_handoffs: Vec::new(),
        }),
        DevToolsPreloadScriptSource::FunctionDeclaration {
            function_declaration,
            arguments,
        } => {
            let source = if arguments.is_empty() {
                crate::domains::runtime::BidiPreloadFunctionDeclaration {
                    source: format!("({function_declaration})();"),
                    channel_handoffs: Vec::new(),
                }
            } else {
                bidi_preload_function_declaration_source(function_declaration, arguments)
                    .map_err(devtools_preload_internal_error)?
                    .ok_or_else(|| {
                        devtools_preload_internal_error("UnsupportedPreloadScriptArguments")
                    })?
            };
            let has_bidi_channel_argument = !source.channel_handoffs.is_empty();
            Ok(DocumentStartScript {
                registry_key: None,
                devtools_session: None,
                source: source.source,
                world_name: command.world_name.clone(),
                has_bidi_channel_argument,
                bidi_channel_handoffs: source.channel_handoffs,
            })
        }
    }
}
fn start_devtools_preload_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsCommand,
) -> PageCommandTaskStep {
    match command {
        DevToolsCommand::AddPreloadScript(command) => {
            start_devtools_add_preload_script_command(conn, command_id, command_session_id, command)
        }
        DevToolsCommand::RemovePreloadScript(command) => {
            start_devtools_remove_preload_script_command(
                conn,
                command_id,
                command_session_id,
                command,
            )
        }
        _ => PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -32000,
            "UnsupportedDevToolsCommand",
        )),
    }
}

pub(crate) async fn execute_devtools_preload_command_async(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
) -> (
    Result<DevToolsCommandResult, DevToolsError>,
    Vec<BackgroundProtocolEvent>,
    Option<moli_core::RendererOutputFence>,
) {
    match command {
        DevToolsCommand::AddPreloadScript(command) => {
            if !command.browser_context_ids.is_empty() {
                return (
                    execute_devtools_browser_context_add_preload_script_command(conn, command)
                        .await,
                    Vec::new(),
                    None,
                );
            }
            execute_devtools_single_route_preload_command_async(
                conn,
                DevToolsCommand::AddPreloadScript(command),
            )
            .await
        }
        DevToolsCommand::RemovePreloadScript(command)
            if command.context.protocol == DevToolsProtocol::WebDriverBidi
                && command.context.target_id.is_none()
                && split_bidi_preload_script_id(&command.script_id).is_none() =>
        {
            (
                execute_devtools_bidi_browser_context_remove_preload_script_command(conn, command)
                    .await,
                Vec::new(),
                None,
            )
        }
        command => execute_devtools_single_route_preload_command_async(conn, command).await,
    }
}

async fn execute_devtools_single_route_preload_command_async(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
) -> (
    Result<DevToolsCommandResult, DevToolsError>,
    Vec<BackgroundProtocolEvent>,
    Option<moli_core::RendererOutputFence>,
) {
    let result_kind = match DevToolsPreloadResultKind::from_command(&command) {
        Ok(result_kind) => result_kind,
        Err(error) => return (Err(error), Vec::new(), None),
    };
    let (route, command) = match devtools_preload_command_target_route(conn, command) {
        Ok(resolved) => resolved,
        Err(error) => return (Err(error), Vec::new(), None),
    };
    let owner = CommandOwnerScope::for_route(route);
    let mut events = Vec::new();
    let mut command_context = CommandDispatchContext::default();
    let result = match command {
        DevToolsCommand::AddPreloadScript(command) => {
            execute_devtools_single_route_add_preload_script_command(
                conn,
                &owner,
                command,
                result_kind,
                &mut events,
                &mut command_context,
            )
            .await
        }
        DevToolsCommand::RemovePreloadScript(command) => {
            match execute_devtools_single_route_remove_preload_script_command(conn, &owner, command)
                .await
            {
                Ok(()) => Ok(DevToolsCommandResult::Empty),
                Err(error) => Err(error),
            }
        }
        _ => Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "UnsupportedDevToolsCommand",
        )),
    };
    let mut ordered_events = command_context.take_protocol_events();
    ordered_events.extend(events);
    ordered_events.extend(command_context.take_post_response_events());
    (
        result,
        ordered_events,
        command_context.take_renderer_output_predecessor(),
    )
}

async fn execute_devtools_browser_context_add_preload_script_command(
    conn: &mut CdpConnection,
    command: DevToolsAddPreloadScriptCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let script = document_start_script_from_add_preload_command(&command)?;
    let browser_context_ids =
        resolve_bidi_preload_browser_context_ids(conn, &command.browser_context_ids)?;
    let Some(first_browser_context_id) = browser_context_ids.first() else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "PreloadScriptUserContextsMustNotBeEmpty",
        ));
    };
    let identifier = conn
        .browser_context_by_id_mut(first_browser_context_id)
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?
        .reserve_default_document_start_script_id();
    let renderer_script = script.with_registry_key(
        BrowserContext::default_document_start_script_registry_key(&identifier),
    );
    for browser_context_id in &browser_context_ids {
        let Some(browser_context) = conn.browser_context_by_id_mut(browser_context_id) else {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "UnknownBrowserContextId",
            ));
        };
        browser_context
            .record_default_document_start_script_with_identifier(identifier.clone(), &script);
    }
    for browser_context_id in &browser_context_ids {
        let active_target_id = conn
            .browser_context_by_id(browser_context_id)
            .and_then(BrowserContext::active_target_id)
            .map(str::to_owned);
        let Some(active_target_id) = active_target_id else {
            continue;
        };
        let route = CdpSessionRoute::PageTarget {
            browser_context_id: browser_context_id.clone(),
            target_id: active_target_id,
            session_key: DevToolsSessionKey::Primary,
        };
        let owner = CommandOwnerScope::for_route(route);
        append_loaded_page_document_start_script_for_owner_async(conn, &owner, &renderer_script)
            .await
            .map_err(|message| devtools_preload_owner_error(&message))?;
    }
    Ok(DevToolsCommandResult::AddPreloadScript(
        DevToolsAddPreloadScriptResult {
            script_id: DevToolsPreloadScriptId::from(identifier),
        },
    ))
}

async fn execute_devtools_single_route_add_preload_script_command(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    command: DevToolsAddPreloadScriptCommand,
    result_kind: DevToolsPreloadResultKind,
    events: &mut Vec<BackgroundProtocolEvent>,
    command_context: &mut CommandDispatchContext,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let identifier = if is_bidi_default_preload_command(&command) {
        execute_devtools_default_add_preload_script_command(conn, owner, command).await?
    } else {
        if conn
            .target_owner_identity_for_route(owner.session_id(), owner.session_owner_route())
            .is_none()
        {
            return Err(preload_missing_owner_error(conn));
        }
        let mut side_effects = CommandOutputPlan::default();
        let identifier = add_script_to_evaluate_on_new_document_direct_async(
            conn,
            owner,
            command,
            &mut side_effects,
            command_context,
        )
        .await?;
        events.extend(side_effects.into_background_events(None, None));
        identifier
    };
    result_kind.add_result(identifier)
}

async fn execute_devtools_default_add_preload_script_command(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    command: DevToolsAddPreloadScriptCommand,
) -> Result<String, DevToolsError> {
    let script = document_start_script_from_add_preload_command(&command)?;
    let Some(browser_context_id) = conn
        .target_owner_identity_for_route(owner.session_id(), owner.session_owner_route())
        .map(|(browser_context_id, _)| browser_context_id)
    else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "BrowserContextNotLoaded",
        ));
    };
    let Some(browser_context) = conn.browser_context_by_id_mut(&browser_context_id) else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchTarget,
            "BrowserContextNotLoaded",
        ));
    };
    let identifier = browser_context.record_default_document_start_script(&script);
    let script = script.with_registry_key(
        BrowserContext::default_document_start_script_registry_key(&identifier),
    );
    append_default_document_start_script_direct_async(conn, owner, identifier, &script).await
}

async fn append_default_document_start_script_direct_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    identifier: String,
    script: &DocumentStartScript,
) -> Result<String, DevToolsError> {
    let renderer_runtime_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let Some(page) = conn
        .runtime_session_owner_slot_mut_for_route(owner.session_id(), owner.session_owner_route())
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return Ok(identifier);
    };
    let pending = page
        .start_add_document_start_script_runtime_activity(
            renderer_runtime_inspector_session_id.as_deref(),
            script,
            false,
        )
        .map_err(|error| devtools_preload_internal_error(error.to_string()))?;
    let completion = pending
        .wait()
        .await
        .map_err(|error| devtools_preload_internal_error(error.to_string()))?;
    let Some(page) = conn
        .runtime_session_owner_slot_mut_for_route(owner.session_id(), owner.session_owner_route())
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return Err(devtools_preload_internal_error("NoDocumentLoaded"));
    };
    page.finish_document_start_script_result(completion)
        .map_err(|error| devtools_preload_internal_error(error.to_string()))?;
    Ok(identifier)
}

async fn execute_devtools_single_route_remove_preload_script_command(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    command: DevToolsRemovePreloadScriptCommand,
) -> Result<(), DevToolsError> {
    let protocol = command.context.protocol;
    let script_id = command.script_id.into_string();
    let owner_identity =
        conn.target_owner_identity_for_route(owner.session_id(), owner.session_owner_route());
    let target_registry_key = owner_identity.as_ref().map(|(_, target_id)| {
        BrowserContext::target_document_start_script_registry_key(target_id.as_deref(), &script_id)
    });
    let remove_result = conn.with_target_owner_state_for_route_mut(
        owner.session_id(),
        owner.session_owner_route(),
        |owner_state| {
            remove_stored_document_start_script_registry_key(
                owner_state,
                None,
                &script_id,
                target_registry_key.clone(),
            )
        },
    );
    let Some((mut removed, mut registry_key)) = remove_result else {
        return Err(preload_missing_owner_error(conn));
    };
    if !removed
        && protocol == DevToolsProtocol::WebDriverBidi
        && let Some((browser_context_id, _)) = owner_identity.as_ref()
        && let Some(browser_context) = conn.browser_context_by_id_mut(browser_context_id)
        && let Some(default_registry_key) =
            browser_context.remove_default_document_start_script(&script_id)
    {
        removed = true;
        registry_key = Some(default_registry_key);
    }
    if !removed && protocol == DevToolsProtocol::WebDriverBidi {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchScript,
            "NoSuchScript",
        ));
    }
    remove_document_start_script_direct_async(conn, owner, registry_key).await
}

async fn remove_document_start_script_direct_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    registry_key: Option<String>,
) -> Result<(), DevToolsError> {
    let Some(registry_key) = registry_key else {
        return Ok(());
    };
    let Some(page) = conn
        .runtime_session_owner_slot_mut_for_route(owner.session_id(), owner.session_owner_route())
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return Ok(());
    };
    let pending = page
        .start_remove_document_start_script_by_registry_key(&registry_key)
        .map_err(|error| devtools_preload_internal_error(error.to_string()))?;
    let completion = pending
        .wait()
        .await
        .map_err(|error| devtools_preload_internal_error(error.to_string()))?;
    let Some(page) = conn
        .runtime_session_owner_slot_mut_for_route(owner.session_id(), owner.session_owner_route())
        .ok()
        .and_then(|slot| slot.loaded_page_mut())
    else {
        return Err(devtools_preload_internal_error("NoDocumentLoaded"));
    };
    page.finish_unit_runtime_page_command(completion, "remove document-start script")
        .map_err(|error| devtools_preload_internal_error(error.to_string()))
}

async fn execute_devtools_bidi_browser_context_remove_preload_script_command(
    conn: &mut CdpConnection,
    command: DevToolsRemovePreloadScriptCommand,
) -> Result<DevToolsCommandResult, DevToolsError> {
    let script_id = command.script_id.as_str().to_owned();
    let browser_context_ids = conn
        .browser_contexts()
        .filter(|browser_context| browser_context.has_default_document_start_script(&script_id))
        .map(|browser_context| browser_context.id.clone())
        .collect::<Vec<_>>();
    if browser_context_ids.is_empty() {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchScript,
            "NoSuchScript",
        ));
    }
    let mut removed_contexts = Vec::new();
    for browser_context_id in &browser_context_ids {
        if let Some(browser_context) = conn.browser_context_by_id_mut(browser_context_id)
            && let Some(registry_key) =
                browser_context.remove_default_document_start_script(&script_id)
        {
            removed_contexts.push((browser_context_id.clone(), registry_key));
        }
    }
    for (browser_context_id, registry_key) in &removed_contexts {
        let active_target_id = conn
            .browser_context_by_id(browser_context_id)
            .and_then(BrowserContext::active_target_id)
            .map(str::to_owned);
        let Some(active_target_id) = active_target_id else {
            continue;
        };
        let route = CdpSessionRoute::PageTarget {
            browser_context_id: browser_context_id.clone(),
            target_id: active_target_id,
            session_key: DevToolsSessionKey::Primary,
        };
        let owner = CommandOwnerScope::for_route(route);
        remove_loaded_page_document_start_script_for_owner_async(conn, &owner, registry_key)
            .await
            .map_err(|message| devtools_preload_owner_error(&message))?;
    }
    Ok(DevToolsCommandResult::Empty)
}

fn resolve_bidi_preload_browser_context_ids(
    conn: &mut CdpConnection,
    browser_context_ids: &[crate::devtools_runtime::DevToolsBrowserContextId],
) -> Result<Vec<String>, DevToolsError> {
    let mut resolved = Vec::new();
    for browser_context_id in browser_context_ids {
        let browser_context_id = browser_context_id.as_str();
        if browser_context_id == "default" {
            let mut default_context_ids = conn
                .browser_contexts()
                .filter(|context| is_moli_internal_default_user_context(&context.id))
                .map(|context| context.id.clone())
                .collect::<Vec<_>>();
            if default_context_ids.is_empty() {
                let id = conn.default_browser_context_id().to_owned();
                conn.insert_browser_context(conn.new_browser_context(id.clone()));
                default_context_ids.push(id);
            }
            resolved.extend(default_context_ids);
            continue;
        }
        if !conn.has_browser_context_id(browser_context_id) {
            return Err(DevToolsError::new(
                DevToolsErrorKind::NoSuchTarget,
                "UnknownBrowserContextId",
            ));
        }
        resolved.push(browser_context_id.to_owned());
    }
    resolved.sort();
    resolved.dedup();
    Ok(resolved)
}

fn is_moli_internal_default_user_context(browser_context_id: &str) -> bool {
    browser_context_id == "BID-default"
        || browser_context_id
            .strip_prefix("BID-")
            .is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
            })
}

fn devtools_preload_owner_error(message: &str) -> DevToolsError {
    if message == "BrowserContextNotLoaded" || message == "TargetNotLoaded" {
        DevToolsError::new(DevToolsErrorKind::NoSuchTarget, message)
    } else {
        DevToolsError::new(DevToolsErrorKind::Internal, message)
    }
}

fn preload_missing_owner_error(conn: &CdpConnection) -> DevToolsError {
    let message = if conn.browser_context.is_none() {
        "BrowserContextNotLoaded"
    } else {
        "TargetNotLoaded"
    };
    DevToolsError::new(DevToolsErrorKind::NoSuchTarget, message)
}

fn devtools_preload_internal_error(message: impl Into<String>) -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::Internal, message)
}

#[derive(Clone)]
enum DevToolsPreloadResultKind {
    Add {
        protocol: DevToolsProtocol,
        target_id: Option<DevToolsTargetId>,
    },
    Remove,
}

impl DevToolsPreloadResultKind {
    fn from_command(command: &DevToolsCommand) -> Result<Self, DevToolsError> {
        match command {
            DevToolsCommand::AddPreloadScript(command) => {
                let target_id = add_preload_command_target_id(command)?.cloned();
                Ok(Self::Add {
                    protocol: command.context.protocol,
                    target_id,
                })
            }
            DevToolsCommand::RemovePreloadScript(_) => Ok(Self::Remove),
            _ => Err(DevToolsError::new(
                DevToolsErrorKind::Unsupported,
                "UnsupportedDevToolsCommand",
            )),
        }
    }

    fn add_result(self, identifier: String) -> Result<DevToolsCommandResult, DevToolsError> {
        let Self::Add {
            protocol,
            target_id,
        } = self
        else {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Internal,
                "UnexpectedPreloadResultKind",
            ));
        };
        let script_id = if protocol == DevToolsProtocol::WebDriverBidi
            && let Some(target_id) = target_id
        {
            format!("{}:{identifier}", target_id.as_str())
        } else {
            identifier
        };
        Ok(DevToolsCommandResult::AddPreloadScript(
            DevToolsAddPreloadScriptResult {
                script_id: DevToolsPreloadScriptId::from(script_id),
            },
        ))
    }
}

fn devtools_preload_command_target_route(
    conn: &mut CdpConnection,
    command: DevToolsCommand,
) -> Result<(CdpSessionRoute, DevToolsCommand), DevToolsError> {
    match command {
        DevToolsCommand::AddPreloadScript(command) => {
            let route = if let Some(target_id) = add_preload_command_target_id(&command)? {
                if conn
                    .target_session_route_for_child_frame_id(target_id.as_str())
                    .is_some()
                {
                    return Err(DevToolsError::new(
                        DevToolsErrorKind::InvalidArgument,
                        "PreloadScriptContextMustBeTopLevel",
                    ));
                }
                conn.target_session_route_for_target_id(target_id.as_str())
                    .ok_or_else(|| {
                        DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget")
                    })?
            } else {
                default_add_preload_command_route(conn)?
            };
            Ok((route, DevToolsCommand::AddPreloadScript(command)))
        }
        DevToolsCommand::RemovePreloadScript(command) => {
            let (route, command) = remove_preload_command_target_route(conn, command)?;
            Ok((route, DevToolsCommand::RemovePreloadScript(command)))
        }
        _ => Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "UnsupportedDevToolsCommand",
        )),
    }
}

fn add_preload_command_target_id(
    command: &DevToolsAddPreloadScriptCommand,
) -> Result<Option<&DevToolsTargetId>, DevToolsError> {
    if !command.browser_context_ids.is_empty() {
        return Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "UnsupportedPreloadScriptUserContexts",
        ));
    }
    match command.target_ids.as_deref() {
        Some([target_id]) => Ok(Some(target_id)),
        Some([]) => Err(DevToolsError::new(
            DevToolsErrorKind::InvalidArgument,
            "PreloadScriptContextsMustNotBeEmpty",
        )),
        Some(_) => Err(DevToolsError::new(
            DevToolsErrorKind::Unsupported,
            "MultiplePreloadScriptContextsUnsupported",
        )),
        None => Ok(command.context.target_id.as_ref()),
    }
}

fn is_bidi_default_preload_command(command: &DevToolsAddPreloadScriptCommand) -> bool {
    command.context.protocol == DevToolsProtocol::WebDriverBidi
        && command.target_ids.is_none()
        && command.context.target_id.is_none()
        && command.browser_context_ids.is_empty()
}

fn default_add_preload_command_route(
    conn: &mut CdpConnection,
) -> Result<CdpSessionRoute, DevToolsError> {
    if conn.browser_context.is_none() {
        let browser_context =
            conn.new_browser_context(conn.default_browser_context_id().to_owned());
        conn.insert_browser_context(browser_context);
    }
    let browser_context = conn
        .browser_context
        .as_ref()
        .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
    Ok(CdpSessionRoute::BrowserContext {
        browser_context_id: browser_context.id.clone(),
    })
}

fn remove_preload_command_target_route(
    conn: &mut CdpConnection,
    mut command: DevToolsRemovePreloadScriptCommand,
) -> Result<(CdpSessionRoute, DevToolsRemovePreloadScriptCommand), DevToolsError> {
    if let Some(target_id) = command.context.target_id.as_ref() {
        let route = conn
            .target_session_route_for_target_id(target_id.as_str())
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchTarget, "NoSuchTarget"))?;
        return Ok((route, command));
    }
    if command.context.protocol == DevToolsProtocol::WebDriverBidi
        && let Some((target_id, local_script_id)) = split_bidi_preload_script_id(&command.script_id)
    {
        let route = conn
            .target_session_route_for_target_id(target_id.as_str())
            .ok_or_else(|| DevToolsError::new(DevToolsErrorKind::NoSuchScript, "NoSuchScript"))?;
        command.context.target_id = Some(target_id);
        command.script_id = local_script_id;
        return Ok((route, command));
    }
    let script_id = command.script_id.as_str().to_owned();
    if command.context.protocol == DevToolsProtocol::WebDriverBidi
        && let Some(route) = find_default_preload_script_route(conn, &script_id)
    {
        return Ok((route, command));
    }
    let Some(route) = find_preload_script_route(conn, &script_id) else {
        return Err(DevToolsError::new(
            DevToolsErrorKind::NoSuchScript,
            "NoSuchScript",
        ));
    };
    Ok((route, command))
}

fn split_bidi_preload_script_id(
    script_id: &DevToolsPreloadScriptId,
) -> Option<(DevToolsTargetId, DevToolsPreloadScriptId)> {
    let (target_id, local_script_id) = script_id.as_str().split_once(':')?;
    if target_id.is_empty() || local_script_id.is_empty() {
        return None;
    }
    Some((
        DevToolsTargetId::from(target_id),
        DevToolsPreloadScriptId::from(local_script_id),
    ))
}

fn find_default_preload_script_route(
    conn: &mut CdpConnection,
    script_id: &str,
) -> Option<CdpSessionRoute> {
    conn.browser_contexts()
        .find(|browser_context| browser_context.has_default_document_start_script(script_id))
        .map(|browser_context| CdpSessionRoute::BrowserContext {
            browser_context_id: browser_context.id.clone(),
        })
}

fn find_preload_script_route(conn: &mut CdpConnection, script_id: &str) -> Option<CdpSessionRoute> {
    let target_ids = conn
        .browser_contexts()
        .flat_map(|browser_context| browser_context.devtools_target_infos())
        .filter_map(|info| {
            if !matches!(
                info.kind,
                DevToolsTargetKind::Page | DevToolsTargetKind::Frame
            ) {
                return None;
            }
            info.target_id
                .map(|target_id| target_id.as_str().to_owned())
        })
        .collect::<Vec<_>>();
    for target_id in target_ids {
        let Some(route) = conn.target_session_route_for_target_id(&target_id) else {
            continue;
        };
        let has_script = conn
            .target_owner_state_for_route(None, Some(&route))
            .is_some_and(|owner_state| {
                owner_state
                    .document_start_scripts
                    .iter()
                    .any(|(identifier, _)| identifier == script_id)
            })
            || route
                .browser_context_id()
                .and_then(|browser_context_id| conn.browser_context_by_id(browser_context_id))
                .is_some_and(|browser_context| {
                    browser_context.has_default_document_start_script(script_id)
                });
        if has_script {
            return Some(route);
        }
    }
    None
}

fn start_devtools_add_preload_script_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsAddPreloadScriptCommand,
) -> PageCommandTaskStep {
    let script = match document_start_script_from_add_preload_command(&command) {
        Ok(script) => script,
        Err(error) => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::from_devtools_error(error));
        }
    };
    if !is_bidi_default_preload_command(&command) {
        if conn
            .target_owner_identity_for_session(command_session_id)
            .is_none()
        {
            if conn.browser_context.is_none() {
                return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                    -31998,
                    "BrowserContextNotLoaded",
                ));
            }
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -31998,
                "TargetNotLoaded",
            ));
        }
        return PageCommandTaskStep::Pending(PendingPageCommandDispatch {
            command_id,
            owner_scope: crate::conn::CommandOwnerScope::capture(conn, command_session_id),
            kind: Box::new(PendingPageCommandKind::AddScriptToEvaluateOnNewDocument(
                PendingAddScriptToEvaluateOnNewDocumentCommand { command },
            )),
        });
    }

    let Some(browser_context) = conn.browser_context.as_mut() else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(
            -31998,
            "BrowserContextNotLoaded",
        ));
    };
    let identifier = browser_context.record_default_document_start_script(&script);
    let script = script.with_registry_key(
        BrowserContext::default_document_start_script_registry_key(&identifier),
    );
    start_default_document_start_script_append(
        conn,
        command_id,
        command_session_id,
        identifier,
        &script,
    )
}

fn start_devtools_remove_preload_script_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    command_session_id: Option<&str>,
    command: DevToolsRemovePreloadScriptCommand,
) -> PageCommandTaskStep {
    let protocol = command.context.protocol;
    let script_id = command.script_id.into_string();
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_session(command_session_id);
    let script_owner_session = (protocol == DevToolsProtocol::Cdp).then(|| {
        DevToolsSessionKey::from_wire_session_id(renderer_inspector_session_id.as_deref())
    });
    let owner_identity = conn.target_owner_identity_for_session(command_session_id);
    let target_registry_key = owner_identity.as_ref().map(|(_, target_id)| {
        target_document_start_script_registry_key_for_owner(
            target_id.as_deref(),
            script_owner_session.as_ref(),
            &script_id,
        )
    });
    let remove_result =
        conn.with_target_owner_state_for_session_mut(command_session_id, |owner_state| {
            remove_stored_document_start_script_registry_key(
                owner_state,
                script_owner_session.as_ref(),
                &script_id,
                target_registry_key.clone(),
            )
        });
    let Some((mut removed, mut registry_key)) = remove_result else {
        if conn.browser_context.is_none() {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -31998,
                "BrowserContextNotLoaded",
            ));
        }
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-31998, "TargetNotLoaded"));
    };
    if !removed
        && let Some((browser_context_id, _)) = owner_identity.as_ref()
        && let Some(browser_context) = conn.browser_context_by_id_mut(browser_context_id)
        && let Some(default_registry_key) =
            browser_context.remove_default_document_start_script(&script_id)
    {
        removed = true;
        registry_key = Some(default_registry_key);
    }
    if !removed {
        let message = match protocol {
            DevToolsProtocol::Cdp => "Script not found",
            DevToolsProtocol::WebDriverClassic | DevToolsProtocol::WebDriverBidi => "NoSuchScript",
        };
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, message));
    }
    start_document_start_script_remove(conn, command_id, command_session_id, registry_key)
}

fn remove_stored_document_start_script_registry_key(
    owner_state: &mut crate::conn::TargetOwnerState,
    owner_session: Option<&DevToolsSessionKey>,
    script_id: &str,
    fallback_registry_key: Option<String>,
) -> (bool, Option<String>) {
    let Some(index) = owner_state
        .document_start_scripts
        .iter()
        .position(|(identifier, script)| {
            identifier == script_id && script.devtools_session.as_ref() == owner_session
        })
    else {
        return (false, None);
    };
    let (_, script) = owner_state.document_start_scripts.remove(index);
    (true, script.registry_key.or(fallback_registry_key))
}

pub(super) fn try_start_add_script_to_evaluate_on_new_document_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<PageCommandTaskStep> {
    let params: AddScriptToEvaluateOnNewDocumentParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return Some(PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            )));
        }
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .unwrap_or((String::new(), None));
    let browser_context_id = (!browser_context_id.is_empty()).then_some(browser_context_id);
    let command = build_cdp_add_preload_script_command(
        cmd,
        target_id.as_deref(),
        browser_context_id.as_deref(),
        params,
    );
    Some(start_devtools_preload_command(
        conn,
        cmd.id,
        cmd.session_id,
        DevToolsCommand::AddPreloadScript(command),
    ))
}

pub(super) fn try_start_remove_script_to_evaluate_on_new_document_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<PageCommandTaskStep> {
    let params: RemoveScriptToEvaluateOnNewDocumentParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return Some(PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            )));
        }
    };
    let (browser_context_id, target_id) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .unwrap_or((String::new(), None));
    let browser_context_id = (!browser_context_id.is_empty()).then_some(browser_context_id);
    let command = build_cdp_remove_preload_script_command(
        cmd,
        target_id.as_deref(),
        browser_context_id.as_deref(),
        params,
    );
    Some(start_devtools_preload_command(
        conn,
        cmd.id,
        cmd.session_id,
        DevToolsCommand::RemovePreloadScript(command),
    ))
}

pub(super) fn try_start_create_isolated_world_command(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> PageCommandTaskStep {
    let params: CreateIsolatedWorldParams = match cmd.get_params() {
        Ok(Some(params)) => params,
        _ => {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32602,
                "InvalidParams",
            ));
        }
    };
    let Some((_, Some(target_id))) = conn.target_owner_identity_for_session(cmd.session_id) else {
        if conn.browser_context.is_none() {
            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -31998,
                "BrowserContextNotLoaded",
            ));
        }
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-31998, "TargetNotLoaded"));
    };
    let Some(task) = prepare_create_isolated_world_task(conn, cmd.session_id, target_id, params)
    else {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-31998, "TargetNotLoaded"));
    };

    if conn
        .runtime_session_owner_slot(cmd.session_id)
        .is_ok_and(|slot| !slot.has_loaded_page())
    {
        return PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, "NoDocumentLoaded"));
    }

    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    start_create_isolated_world_initial_navigation_or_renderer_phase(conn, cmd.id, owner, task)
}

fn prepare_create_isolated_world_task(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    target_id: String,
    params: CreateIsolatedWorldParams,
) -> Option<CreateIsolatedWorldCommandTask> {
    let has_bidi_channel_argument =
        conn.with_target_owner_state_for_session_mut(session_id, |owner_state| {
            owner_state
                .document_start_scripts
                .iter()
                .any(|(_, script)| {
                    script.world_name.as_deref() == Some(params.world_name.as_str())
                        && script.has_bidi_channel_argument
                })
        })?;
    Some(CreateIsolatedWorldCommandTask {
        target_id,
        params,
        has_bidi_channel_argument,
        prefix_output: CommandOutputPlan::default(),
        pending_renderer_agent_attachment_id: None,
        phase: CreateIsolatedWorldPhase::RuntimeActivity,
    })
}

fn pending_create_isolated_world_command_for_session(
    command_id: Option<u64>,
    owner_scope: crate::conn::CommandOwnerScope,
    task: CreateIsolatedWorldCommandTask,
    pending: PendingCreateIsolatedWorldPhase,
) -> PageCommandTaskStep {
    PageCommandTaskStep::Pending(PendingPageCommandDispatch {
        command_id,
        owner_scope,
        kind: Box::new(PendingPageCommandKind::CreateIsolatedWorld(
            PendingCreateIsolatedWorldCommand { task, pending },
        )),
    })
}

fn start_create_isolated_world_initial_navigation_or_renderer_phase(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: CommandOwnerScope,
    mut task: CreateIsolatedWorldCommandTask,
) -> PageCommandTaskStep {
    let should_start_target_url_navigation =
        conn.runtime_session_owner_can_start_initial_document_navigation(owner.session_id());
    if !should_start_target_url_navigation {
        return start_create_isolated_world_frame_or_world_phase(conn, command_id, owner, task);
    }

    let start = match super::navigation::start_initial_document_navigation_for_session_owner(
        conn,
        None,
        &owner,
        json!({}),
    ) {
        Ok(start) => start,
        Err(plan) => return PageCommandTaskStep::Complete(plan),
    };
    match start {
        super::navigation::NavigateCommandStart::CompleteImmediate(plan) => {
            if let Err(plan) = append_page_command_step_output(
                &mut task.prefix_output,
                PageCommandTaskStep::Complete(plan),
            ) {
                return PageCommandTaskStep::Complete(plan);
            }
            start_create_isolated_world_frame_or_world_phase(conn, command_id, owner, task)
        }
        super::navigation::NavigateCommandStart::CompletePlan(plan) => {
            PageCommandTaskStep::Complete(plan)
        }
        super::navigation::NavigateCommandStart::PendingLoad(pending) => {
            task.phase = CreateIsolatedWorldPhase::InitialDocumentNavigation;
            pending_create_isolated_world_command_for_session(
                command_id,
                owner.clone(),
                task,
                PendingCreateIsolatedWorldPhase::InitialDocumentNavigation(pending),
            )
        }
        super::navigation::NavigateCommandStart::PendingChildFrame(_) => {
            PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32000,
                "UnexpectedChildFrameNavigation",
            ))
        }
        super::navigation::NavigateCommandStart::PendingSameDocument(_) => {
            PageCommandTaskStep::Complete(CommandOutputPlan::error(
                -32000,
                "UnexpectedSameDocumentNavigation",
            ))
        }
        super::navigation::NavigateCommandStart::PendingContinueWithoutRequestPause(pending) => {
            task.phase = CreateIsolatedWorldPhase::InitialDocumentNavigation;
            pending_create_isolated_world_command_for_session(
                command_id,
                owner,
                task,
                PendingCreateIsolatedWorldPhase::InitialDocumentNavigationContinue(pending),
            )
        }
    }
}

fn start_create_isolated_world_frame_or_world_phase(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: CommandOwnerScope,
    mut task: CreateIsolatedWorldCommandTask,
) -> PageCommandTaskStep {
    task.phase = CreateIsolatedWorldPhase::RuntimeActivity;
    let frame_id = (task.params.frame_id != task.target_id).then(|| task.params.frame_id.clone());
    let world_name = task.params.world_name.clone();
    let grant_universal_access = task.params.grant_universal_access;
    let renderer_runtime_inspector_session_id = conn
        .target_renderer_runtime_inspector_session_id_for_route(
            owner.session_id(),
            owner.session_owner_route(),
        );
    let page =
        match loaded_page_mut_for_create_isolated_world_renderer_command(conn, &owner, &mut task) {
            Ok(page) => page,
            Err(plan) => return PageCommandTaskStep::Complete(plan),
        };
    match page.start_create_isolated_world_runtime_activity_capturing_runtime_inspector_messages(
        renderer_runtime_inspector_session_id.as_deref(),
        frame_id.as_deref(),
        &world_name,
        grant_universal_access,
    ) {
        Ok(pending) => pending_create_isolated_world_command_for_session(
            command_id,
            owner,
            task,
            PendingCreateIsolatedWorldPhase::RendererPageCommand(pending),
        ),
        Err(error) => {
            PageCommandTaskStep::Complete(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

fn loaded_page_mut_for_create_isolated_world_renderer_command<'a>(
    conn: &'a mut CdpConnection,
    owner: &CommandOwnerScope,
    task: &mut CreateIsolatedWorldCommandTask,
) -> Result<&'a mut moli_core::page::Page, CommandOutputPlan> {
    let slot = conn
        .runtime_session_owner_slot_mut_for_route(owner.session_id(), owner.session_owner_route())
        .map_err(|error| CommandOutputPlan::error(-32000, error))?;
    task.pending_renderer_agent_attachment_id = Some(
        slot.current_renderer_attachment()
            .ok_or_else(|| CommandOutputPlan::error(-32000, "NoDocumentLoaded"))?
            .id(),
    );
    slot.loaded_page_mut()
        .ok_or_else(|| CommandOutputPlan::error(-32000, "NoDocumentLoaded"))
}

fn create_isolated_world_renderer_completion_is_stale(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    task: &CreateIsolatedWorldCommandTask,
) -> bool {
    let Some(expected_attachment_id) = task.pending_renderer_agent_attachment_id else {
        return false;
    };
    conn.runtime_session_owner_slot_for_route(owner.session_id(), owner.session_owner_route())
        .map(|slot| {
            slot.current_renderer_attachment()
                .map(|attachment| attachment.id())
                != Some(expected_attachment_id)
                || !slot.has_loaded_page()
        })
        .unwrap_or(true)
}

fn restart_create_isolated_world_after_stale_renderer_completion(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: CommandOwnerScope,
    mut task: CreateIsolatedWorldCommandTask,
) -> PageCommandTaskStep {
    task.pending_renderer_agent_attachment_id = None;
    start_create_isolated_world_frame_or_world_phase(conn, command_id, owner, task)
}

fn record_document_start_script(
    owner_state: &mut crate::conn::TargetOwnerState,
    target_id: Option<&str>,
    owner_session: Option<&DevToolsSessionKey>,
    script: &DocumentStartScript,
) -> RecordedDocumentStartScript {
    if let Some(owner_session) = owner_session {
        let identifier = owner_state
            .document_start_scripts
            .iter()
            .filter(|(_, script)| script.devtools_session.as_ref() == Some(owner_session))
            .filter_map(|(identifier, _)| identifier.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            .wrapping_add(1)
            .to_string();
        let script = script
            .with_devtools_session(owner_session.clone())
            .with_registry_key(target_document_start_script_registry_key_for_owner(
                target_id,
                Some(owner_session),
                &identifier,
            ));
        owner_state
            .document_start_scripts
            .push((identifier.clone(), script.clone()));
        return RecordedDocumentStartScript {
            identifier,
            script,
            inserted: true,
        };
    }
    if owner_state.next_document_start_script_id == 0
        && !owner_state.document_start_scripts.is_empty()
    {
        let max_existing_id = owner_state
            .document_start_scripts
            .iter()
            .filter(|(_, script)| script.devtools_session.is_none())
            .filter_map(|(identifier, _)| identifier.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        owner_state.next_document_start_script_id = max_existing_id;
        if let Some((identifier, existing)) =
            owner_state
                .document_start_scripts
                .iter_mut()
                .find(|(_, existing)| {
                    existing.devtools_session.is_none()
                        && existing.source == script.source
                        && existing.world_name == script.world_name
                })
        {
            let registry_key = existing.registry_key.clone().unwrap_or_else(|| {
                BrowserContext::target_document_start_script_registry_key(target_id, identifier)
            });
            if existing.registry_key.is_none() {
                existing.registry_key = Some(registry_key.clone());
            }
            let script = existing.with_registry_key(registry_key);
            return RecordedDocumentStartScript {
                identifier: identifier.clone(),
                script,
                inserted: false,
            };
        }
    }
    owner_state.next_document_start_script_id =
        owner_state.next_document_start_script_id.wrapping_add(1);
    let identifier = owner_state.next_document_start_script_id.to_string();
    let script = owner_session
        .map(|owner| script.with_devtools_session(owner.clone()))
        .unwrap_or_else(|| script.clone())
        .with_registry_key(target_document_start_script_registry_key_for_owner(
            target_id,
            owner_session,
            &identifier,
        ));
    owner_state
        .document_start_scripts
        .push((identifier.clone(), script.clone()));
    RecordedDocumentStartScript {
        identifier,
        script,
        inserted: true,
    }
}

fn target_document_start_script_registry_key_for_owner(
    target_id: Option<&str>,
    owner_session: Option<&DevToolsSessionKey>,
    identifier: &str,
) -> String {
    match owner_session.and_then(DevToolsSessionKey::wire_session_id) {
        Some(owner_session_id) => {
            BrowserContext::target_session_document_start_script_registry_key(
                target_id,
                owner_session_id,
                identifier,
            )
        }
        None => BrowserContext::target_document_start_script_registry_key(target_id, identifier),
    }
}

pub(super) async fn complete_pending_create_isolated_world_command(
    conn: &mut CdpConnection,
    command_id: Option<u64>,
    owner: &CommandOwnerScope,
    mut completed: CompletedCreateIsolatedWorldCommand,
    command_context: &mut CommandDispatchContext,
) -> PageCommandTaskStep {
    match completed.task.phase.clone() {
        CreateIsolatedWorldPhase::InitialDocumentNavigation => {
            let navigation_step = match completed.completed {
                CompletedCreateIsolatedWorldPhase::InitialDocumentNavigation(completed) => {
                    super::navigation::complete_pending_navigate_load_command(
                        conn,
                        *completed,
                        command_context,
                    )
                    .await
                }
                CompletedCreateIsolatedWorldPhase::InitialDocumentNavigationContinue(completed) => {
                    super::navigation::complete_pending_continue_navigation_without_request_pause_command(
                        conn,
                        *completed,
                    )
                    .await
                }
                CompletedCreateIsolatedWorldPhase::RendererPageCommand(_) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        "Invalid createIsolatedWorld initial navigation completion",
                    ));
                }
            };
            if let Err(plan) =
                append_page_command_step_output(&mut completed.task.prefix_output, navigation_step)
            {
                return PageCommandTaskStep::Complete(plan);
            }
            start_create_isolated_world_frame_or_world_phase(
                conn,
                command_id,
                owner.clone(),
                completed.task,
            )
        }
        CreateIsolatedWorldPhase::RuntimeActivity => {
            if create_isolated_world_renderer_completion_is_stale(conn, owner, &completed.task) {
                return restart_create_isolated_world_after_stale_renderer_completion(
                    conn,
                    command_id,
                    owner.clone(),
                    completed.task,
                );
            }
            let is_child_frame_request = completed.task.params.frame_id != completed.task.target_id;
            let completion = match completed.completed {
                CompletedCreateIsolatedWorldPhase::RendererPageCommand(renderer_completed) => {
                    match *renderer_completed {
                        Ok(completion) => completion,
                        Err(message) => {
                            let error_message = if is_child_frame_request {
                                "NoFrameForGivenId".to_owned()
                            } else {
                                message
                            };
                            return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                                -32000,
                                error_message,
                            ));
                        }
                    }
                }
                CompletedCreateIsolatedWorldPhase::InitialDocumentNavigation(_)
                | CompletedCreateIsolatedWorldPhase::InitialDocumentNavigationContinue(_) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        "Invalid createIsolatedWorld runtime-activity completion",
                    ));
                }
            };
            let completed_world = {
                let Some(page) = conn
                    .runtime_session_owner_slot_mut_for_route(
                        owner.session_id(),
                        owner.session_owner_route(),
                    )
                    .ok()
                    .and_then(|slot| slot.loaded_page_mut())
                else {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        "NoDocumentLoaded",
                    ));
                };
                page.finish_create_isolated_world_command_turn(completion)
            };
            let (execution_context_id, output) = match completed_world {
                Ok(completed_world) => completed_world,
                Err(error) => {
                    return PageCommandTaskStep::Complete(CommandOutputPlan::error(
                        -32000,
                        error.to_string(),
                    ));
                }
            };
            command_context.consume_renderer_command_turn_output(output);
            complete_create_isolated_world_task(conn, owner, completed.task, execution_context_id)
                .await
        }
    }
}

fn append_page_command_step_output(
    prefix: &mut CommandOutputPlan,
    step: PageCommandTaskStep,
) -> Result<(), CommandOutputPlan> {
    match step {
        PageCommandTaskStep::Complete(plan) => {
            if plan.command_status().is_some_and(|status| status.is_err()) {
                return Err(plan);
            }
            prefix.extend(plan.into_composite_command_prefix());
            Ok(())
        }
        PageCommandTaskStep::Pending(_) => Err(CommandOutputPlan::error(
            -32000,
            "Invalid createIsolatedWorld nested navigation continuation",
        )),
    }
}

async fn complete_create_isolated_world_task(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    task: CreateIsolatedWorldCommandTask,
    execution_context_id: i64,
) -> PageCommandTaskStep {
    let CreateIsolatedWorldCommandTask {
        target_id: _,
        params: _,
        mut prefix_output,
        has_bidi_channel_argument,
        ..
    } = task;
    let mut preload_channel_listener_events = Vec::new();
    if has_bidi_channel_argument {
        Box::pin(
            crate::domains::runtime::start_bidi_preload_channel_listeners_for_execution_context_background_events_async(
                conn,
                owner,
                execution_context_id,
                &mut preload_channel_listener_events,
            ),
        )
        .await;
    }
    push_background_events(&mut prefix_output, preload_channel_listener_events);
    prefix_output.push_result(json!({ "executionContextId": execution_context_id }));
    PageCommandTaskStep::Complete(prefix_output)
}

pub(super) async fn complete_pending_add_script_to_evaluate_on_new_document_command(
    conn: &mut CdpConnection,
    _command_id: Option<u64>,
    owner: &CommandOwnerScope,
    completed: CompletedAddScriptToEvaluateOnNewDocumentCommand,
    command_context: &mut CommandDispatchContext,
) -> PageCommandTaskStep {
    let mut plan = CommandOutputPlan::default();
    match add_script_to_evaluate_on_new_document_direct_async(
        conn,
        owner,
        completed.command,
        &mut plan,
        command_context,
    )
    .await
    {
        Ok(identifier) => {
            plan.extend(add_preload_script_result_plan(identifier));
            PageCommandTaskStep::Complete(plan)
        }
        Err(error) => {
            plan.extend(CommandOutputPlan::from_devtools_error(error));
            PageCommandTaskStep::Complete(plan)
        }
    }
}

async fn add_script_to_evaluate_on_new_document_direct_async(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    command: DevToolsAddPreloadScriptCommand,
    side_effects: &mut CommandOutputPlan,
    command_context: &mut CommandDispatchContext,
) -> Result<String, DevToolsError> {
    let script = document_start_script_from_add_preload_command(&command)?;
    let session_id = owner.session_id();
    let owner_route = owner.session_owner_route();
    let renderer_inspector_session_id =
        conn.target_renderer_runtime_inspector_session_id_for_route(session_id, owner_route);
    let script_owner_session = (command.context.protocol == DevToolsProtocol::Cdp).then(|| {
        DevToolsSessionKey::from_wire_session_id(renderer_inspector_session_id.as_deref())
    });
    let target_id = conn
        .target_owner_identity_for_route(session_id, owner_route)
        .and_then(|(_, target_id)| target_id);
    let Some(recorded) =
        conn.with_target_owner_state_for_route_mut(session_id, owner_route, |owner_state| {
            record_document_start_script(
                owner_state,
                target_id.as_deref(),
                script_owner_session.as_ref(),
                &script,
            )
        })
    else {
        return Err(preload_missing_owner_error(conn));
    };
    let identifier = recorded.identifier.clone();
    let script = recorded.script;
    if !recorded.inserted {
        return Ok(identifier);
    }
    let pending_run_immediately = {
        let renderer_runtime_inspector_session_id = renderer_inspector_session_id;
        let slot = match conn.runtime_session_owner_slot_mut_for_route(session_id, owner_route) {
            Ok(slot) => slot,
            Err(error) => return Err(devtools_preload_internal_error(error)),
        };
        if let Some(page) = slot.loaded_page_mut() {
            Some(
                page.start_add_document_start_script_runtime_activity(
                    renderer_runtime_inspector_session_id.as_deref(),
                    &script,
                    command.run_immediately,
                )
                .map_err(|error| devtools_preload_internal_error(error.to_string()))?,
            )
        } else {
            None
        }
    };
    let run_immediately_result = match pending_run_immediately {
        Some(pending) => {
            // The pending command owns the renderer turn. Reacquire the
            // session-owned Page after the wait so no mutable protocol owner
            // is retained across this asynchronous boundary.
            let completion = pending
                .wait()
                .await
                .map_err(|error| devtools_preload_internal_error(error.to_string()))?;
            let (result, output) = {
                let slot = conn
                    .runtime_session_owner_slot_mut_for_route(session_id, owner_route)
                    .map_err(devtools_preload_internal_error)?;
                let page = slot.loaded_page_mut().ok_or_else(|| {
                    devtools_preload_internal_error("NoDocumentLoaded".to_owned())
                })?;
                page.finish_document_start_script_result_command_turn(completion)
                    .map_err(|error| devtools_preload_internal_error(error.to_string()))?
            };
            command_context.consume_renderer_command_turn_output(output);
            result
        }
        None => None,
    };
    if script.has_bidi_channel_argument
        && let Some((execution_context_id, _)) = run_immediately_result
    {
        let mut preload_channel_listener_events = Vec::new();
        Box::pin(
            crate::domains::runtime::start_bidi_preload_channel_listeners_for_execution_context_background_events_async(
                conn,
                owner,
                execution_context_id,
                &mut preload_channel_listener_events,
            ),
        )
        .await;
        push_background_events(side_effects, preload_channel_listener_events);
    }
    Ok(identifier)
}

#[cfg(test)]
mod protocol_neutral_tests {
    use crate::devtools_runtime::{
        DevToolsAddPreloadScriptCommand, DevToolsCommand, DevToolsCommandContext,
        DevToolsPreloadScriptSource, DevToolsProtocol,
    };
    use serde_json::json;

    use crate::conn::{CdpConnection, Cmd, DocumentStartScript, TargetOwnerState};

    use super::{
        PageCommandTaskStep, build_cdp_add_preload_script_command,
        build_cdp_remove_preload_script_command, document_start_script_from_add_preload_command,
        start_devtools_preload_command,
    };

    #[test]
    fn cdp_add_preload_script_builds_protocol_neutral_command() {
        let params = json!({
            "source": "globalThis.ready = true;",
            "worldName": "utility",
            "runImmediately": true,
            "includeCommandLineAPI": true
        });
        let cmd = Cmd::for_test(
            Some(41),
            "Page.addScriptToEvaluateOnNewDocument",
            &params,
            Some("SID-preload"),
            r#"{"id":41,"method":"Page.addScriptToEvaluateOnNewDocument"}"#,
        );
        let params = cmd
            .get_params()
            .expect("add preload params should parse")
            .expect("add preload params should be present");

        let command =
            build_cdp_add_preload_script_command(&cmd, Some("TID-preload"), Some("BID-1"), params);

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(
            command.context.session_id.as_ref().map(|id| id.as_str()),
            Some("SID-preload")
        );
        assert_eq!(
            command.context.target_id.as_ref().map(|id| id.as_str()),
            Some("TID-preload")
        );
        assert_eq!(
            command
                .context
                .browser_context_id
                .as_ref()
                .map(|id| id.as_str()),
            Some("BID-1")
        );
        assert_eq!(
            command.target_ids.as_ref().map(|ids| {
                ids.iter()
                    .map(crate::devtools_runtime::DevToolsTargetId::as_str)
                    .collect::<Vec<_>>()
            }),
            Some(vec!["TID-preload"])
        );
        assert_eq!(
            command
                .browser_context_ids
                .iter()
                .map(crate::devtools_runtime::DevToolsBrowserContextId::as_str)
                .collect::<Vec<_>>(),
            vec!["BID-1"]
        );
        assert_eq!(command.world_name.as_deref(), Some("utility"));
        assert!(command.run_immediately);
        assert!(command.include_command_line_api);
        assert_eq!(
            command.source,
            DevToolsPreloadScriptSource::RawScript("globalThis.ready = true;".to_owned())
        );
    }

    #[test]
    fn cdp_remove_preload_script_builds_protocol_neutral_command() {
        let params = json!({"identifier": "SCRIPT-1"});
        let cmd = Cmd::for_test(
            Some(42),
            "Page.removeScriptToEvaluateOnNewDocument",
            &params,
            Some("SID-preload"),
            r#"{"id":42,"method":"Page.removeScriptToEvaluateOnNewDocument"}"#,
        );
        let params = cmd
            .get_params()
            .expect("remove preload params should parse")
            .expect("remove preload params should be present");

        let command = build_cdp_remove_preload_script_command(
            &cmd,
            Some("TID-preload"),
            Some("BID-1"),
            params,
        );

        assert_eq!(command.context.protocol, DevToolsProtocol::Cdp);
        assert_eq!(command.script_id.as_str(), "SCRIPT-1");
        assert_eq!(
            command.context.target_id.as_ref().map(|id| id.as_str()),
            Some("TID-preload")
        );
    }

    #[test]
    fn target_preload_remove_prefers_stored_registry_key() {
        let mut owner_state = TargetOwnerState {
            document_start_scripts: vec![(
                "1".to_owned(),
                DocumentStartScript {
                    registry_key: Some("target:legacy-active:1".to_owned()),
                    devtools_session: None,
                    source: "globalThis.ready = true;".to_owned(),
                    world_name: None,
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
            )],
            ..Default::default()
        };

        let (removed, registry_key) = super::remove_stored_document_start_script_registry_key(
            &mut owner_state,
            None,
            "1",
            Some("target:TID-current:1".to_owned()),
        );

        assert!(removed);
        assert_eq!(registry_key.as_deref(), Some("target:legacy-active:1"));
        assert!(owner_state.document_start_scripts.is_empty());
    }

    #[test]
    fn target_preload_remove_falls_back_for_legacy_unkeyed_script() {
        let mut owner_state = TargetOwnerState {
            document_start_scripts: vec![(
                "1".to_owned(),
                DocumentStartScript {
                    registry_key: None,
                    devtools_session: None,
                    source: "globalThis.ready = true;".to_owned(),
                    world_name: None,
                    has_bidi_channel_argument: false,
                    bidi_channel_handoffs: Vec::new(),
                },
            )],
            ..Default::default()
        };

        let (removed, registry_key) = super::remove_stored_document_start_script_registry_key(
            &mut owner_state,
            None,
            "1",
            Some("target:TID-current:1".to_owned()),
        );

        assert!(removed);
        assert_eq!(registry_key.as_deref(), Some("target:TID-current:1"));
        assert!(owner_state.document_start_scripts.is_empty());
    }

    #[test]
    fn target_preload_duplicate_records_key_for_legacy_unkeyed_script() {
        let mut owner_state = TargetOwnerState::default();
        owner_state.document_start_scripts.push((
            "1".to_owned(),
            DocumentStartScript {
                registry_key: None,
                devtools_session: None,
                source: "globalThis.ready = true;".to_owned(),
                world_name: Some("utility".to_owned()),
                has_bidi_channel_argument: false,
                bidi_channel_handoffs: Vec::new(),
            },
        ));
        let script = DocumentStartScript {
            registry_key: None,
            devtools_session: None,
            source: "globalThis.ready = true;".to_owned(),
            world_name: Some("utility".to_owned()),
            has_bidi_channel_argument: false,
            bidi_channel_handoffs: Vec::new(),
        };

        let recorded = super::record_document_start_script(
            &mut owner_state,
            Some("TID-current"),
            None,
            &script,
        );

        assert!(!recorded.inserted);
        assert_eq!(recorded.identifier, "1");
        assert_eq!(
            recorded.script.registry_key.as_deref(),
            Some("target:TID-current:1")
        );
        assert_eq!(
            owner_state.document_start_scripts[0]
                .1
                .registry_key
                .as_deref(),
            Some("target:TID-current:1")
        );
    }

    #[test]
    fn target_preload_duplicate_preserves_existing_stored_key() {
        let mut owner_state = TargetOwnerState::default();
        owner_state.document_start_scripts.push((
            "1".to_owned(),
            DocumentStartScript {
                registry_key: Some("target:legacy-active:1".to_owned()),
                devtools_session: None,
                source: "globalThis.ready = true;".to_owned(),
                world_name: None,
                has_bidi_channel_argument: false,
                bidi_channel_handoffs: Vec::new(),
            },
        ));
        let script = DocumentStartScript {
            registry_key: None,
            devtools_session: None,
            source: "globalThis.ready = true;".to_owned(),
            world_name: None,
            has_bidi_channel_argument: false,
            bidi_channel_handoffs: Vec::new(),
        };

        let recorded = super::record_document_start_script(
            &mut owner_state,
            Some("TID-current"),
            None,
            &script,
        );

        assert!(!recorded.inserted);
        assert_eq!(
            recorded.script.registry_key.as_deref(),
            Some("target:legacy-active:1")
        );
        assert_eq!(
            owner_state.document_start_scripts[0]
                .1
                .registry_key
                .as_deref(),
            Some("target:legacy-active:1")
        );
    }

    #[test]
    fn bidi_add_preload_script_marks_channel_arguments() {
        let command = DevToolsAddPreloadScriptCommand {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: None,
                target_id: None,
                browser_context_id: None,
            },
            source: DevToolsPreloadScriptSource::FunctionDeclaration {
                function_declaration: "(channel) => channel('ready')".to_owned(),
                arguments: vec![json!({
                    "type": "channel",
                    "value": {
                        "channel": "channel_name"
                    }
                })],
            },
            world_name: None,
            target_ids: None,
            browser_context_ids: Vec::new(),
            run_immediately: false,
            include_command_line_api: false,
        };

        let script = document_start_script_from_add_preload_command(&command)
            .expect("BiDi channel preload should lower to a document-start script");

        assert!(script.has_bidi_channel_argument);
        assert_eq!(script.bidi_channel_handoffs.len(), 1);
        assert_eq!(script.bidi_channel_handoffs[0].channel, "channel_name");
        assert!(
            script.bidi_channel_handoffs[0]
                .handoff_id
                .starts_with("__lmBidiPreloadChannel_")
        );
        assert!(!script.bidi_channel_handoffs[0].token.is_empty());
        assert!(script.source.contains("__moliCreateBidiChannelDelegate"));
        assert!(script.source.contains("__moliPutBidiPreloadChannelProxy"));
        assert!(!script.source.contains("__moliBidiPreloadChannelRegistry"));
        assert!(
            !script
                .source
                .contains("__moliEnsureBidiPreloadChannelHandoff")
        );
        assert!(!script.source.contains("Object.create(null)"));
    }

    #[test]
    fn devtools_preload_entry_routes_add_command_to_owner_error() {
        let mut conn = CdpConnection::new();
        let params = json!({"source": "globalThis.ready = true;"});
        let cmd = Cmd::for_test(
            Some(43),
            "Page.addScriptToEvaluateOnNewDocument",
            &params,
            Some("SID-missing"),
            r#"{"id":43,"method":"Page.addScriptToEvaluateOnNewDocument"}"#,
        );
        let params = cmd
            .get_params()
            .expect("add preload params should parse")
            .expect("add preload params should be present");
        let command = build_cdp_add_preload_script_command(&cmd, None, None, params);

        let step = start_devtools_preload_command(
            &mut conn,
            cmd.id,
            cmd.session_id,
            DevToolsCommand::AddPreloadScript(command),
        );

        let PageCommandTaskStep::Complete(plan) = step else {
            panic!("missing preload target should complete through the unified preload entry");
        };
        let mut out = Vec::new();
        plan.emit_into(&mut out, cmd.id, cmd.session_id);
        assert_eq!(out[0]["id"], json!(43));
        assert_eq!(out[0]["error"]["message"], json!("BrowserContextNotLoaded"));
    }

    #[test]
    fn primary_cdp_preload_retains_typed_session_ownership() {
        let mut owner_state = TargetOwnerState::default();
        let script = DocumentStartScript {
            registry_key: None,
            devtools_session: None,
            source: "globalThis.primaryReady = true;".to_owned(),
            world_name: Some("utility".to_owned()),
            has_bidi_channel_argument: false,
            bidi_channel_handoffs: Vec::new(),
        };
        let primary = moli_page_types::DevToolsSessionKey::Primary;

        let recorded = super::record_document_start_script(
            &mut owner_state,
            Some("TID-primary"),
            Some(&primary),
            &script,
        );

        assert!(recorded.inserted);
        assert_eq!(recorded.script.devtools_session.as_ref(), Some(&primary));
        assert!(
            owner_state
                .document_start_scripts
                .iter()
                .any(|(identifier, script)| {
                    identifier == "1" && script.devtools_session.as_ref() == Some(&primary)
                })
        );
    }
}
