//! CDP world/preload handlers finish at the renderer. Browser keeps only the
//! desired preload registry for future documents. BiDi channel installation
//! and initial navigation remain explicitly Browser-owned composite tasks.
use super::*;
use crate::domains::native::{self, NativeCommandStep};
use moli_core::{
    RendererNativeOperation as Operation, RendererNativeProtocolResponse as Response,
    RendererPageCommand as Command, RendererPageReply as Reply,
};

pub(in crate::domains::page) fn try_start(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    action: crate::domains::actions::PageAction,
) -> Option<NativeCommandStep> {
    use crate::domains::actions::PageAction;
    let operation = match action {
        PageAction::CreateIsolatedWorld => {
            let params: CreateIsolatedWorldParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => {
                    return Some(NativeCommandStep::Complete(CommandOutputPlan::error(
                        -32602,
                        "InvalidParams",
                    )));
                }
            };
            let owner = CommandOwnerScope::capture(conn, cmd.session_id);
            let (_, Some(target_id)) = conn.target_owner_identity_for_owner(&owner)? else {
                return None;
            };
            let task = prepare_create_isolated_world_task(conn, cmd.session_id, target_id, params)?;
            if task.has_bidi_channel_argument
                || conn
                    .runtime_session_owner_can_start_initial_document_navigation_for_owner(&owner)
            {
                return None;
            }
            let child = task.params.frame_id != task.target_id;
            let session = conn.target_renderer_runtime_inspector_session_id_for_owner(&owner);
            Ok(Operation::new(
                Command::CreateIsolatedWorldRuntimeActivity {
                    inspector_session_id: session,
                    frame_id: child.then_some(task.params.frame_id),
                    name: task.params.world_name,
                    grant_universal_access: task.params.grant_universal_access,
                },
                move |reply| match reply {
                    Ok(Reply::ExecutionContextId(id)) => {
                        Response::success(json!({"executionContextId": id}))
                    }
                    Err(error) => Response::error(
                        -32000,
                        if child {
                            "NoFrameForGivenId".to_owned()
                        } else {
                            error.to_string()
                        },
                    ),
                    _ => unreachable!("Page isolated world reply"),
                },
            ))
        }
        PageAction::AddScriptToEvaluateOnNewDocument => add(conn, cmd),
        PageAction::RemoveScriptToEvaluateOnNewDocument => remove(conn, cmd),
        _ => return None,
    };
    Some(match operation {
        // World creation and immediate preloads acquire the document isolate.
        Ok(operation) => native::start_operation(conn, cmd, operation),
        Err(plan) => NativeCommandStep::Complete(plan),
    })
}

fn add(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> Result<Operation, CommandOutputPlan> {
    let params: AddScriptToEvaluateOnNewDocumentParams = cmd
        .get_params()
        .ok()
        .flatten()
        .ok_or_else(|| CommandOutputPlan::error(-32602, "InvalidParams"))?;
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let (context, target) = conn
        .target_owner_identity_for_owner(&owner)
        .ok_or_else(|| CommandOutputPlan::error(-31998, "TargetNotLoaded"))?;
    let command =
        build_cdp_add_preload_script_command(cmd, target.as_deref(), Some(&context), params);
    let script = document_start_script_from_add_preload_command(&command)
        .map_err(CommandOutputPlan::from_devtools_error)?;
    let session = conn.target_renderer_runtime_inspector_session_id_for_owner(&owner);
    let script_owner = DevToolsSessionKey::from_wire_session_id(session.as_deref());
    let recorded = conn
        .with_target_owner_state_for_owner_mut(&owner, |state| {
            record_document_start_script(state, target.as_deref(), Some(&script_owner), &script)
        })
        .ok_or_else(|| CommandOutputPlan::error(-31998, "TargetNotLoaded"))?;
    let identifier = recorded.identifier;
    if !recorded.inserted {
        return Err(add_preload_script_result_plan(identifier));
    }
    Ok(Operation::new(
        Command::AddDocumentStartScriptRuntimeActivity {
            inspector_session_id: session,
            script: recorded.script,
            run_immediately: command.run_immediately,
        },
        move |reply| match reply {
            Ok(Reply::DocumentStartScriptResult(_)) => {
                Response::success(add_preload_script_result(identifier))
            }
            Err(error) => Response::error(-32000, error.to_string()),
            _ => unreachable!("Page preload append reply"),
        },
    ))
}

fn remove(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> Result<Operation, CommandOutputPlan> {
    let params: RemoveScriptToEvaluateOnNewDocumentParams = cmd
        .get_params()
        .ok()
        .flatten()
        .ok_or_else(|| CommandOutputPlan::error(-32602, "InvalidParams"))?;
    let (context, target) = conn
        .target_owner_identity_for_session(cmd.session_id)
        .ok_or_else(|| CommandOutputPlan::error(-31998, "TargetNotLoaded"))?;
    let command =
        build_cdp_remove_preload_script_command(cmd, target.as_deref(), Some(&context), params);
    let registry_key = remove_preload_script_registration(conn, cmd.session_id, command)?
        .ok_or_else(CommandOutputPlan::success)?;
    Ok(Operation::new(
        Command::RemoveDocumentStartScriptByRegistryKey(registry_key),
        |reply| match reply {
            Ok(Reply::Unit) => Response::success(json!({})),
            Err(error) => Response::error(-32000, error.to_string()),
            _ => unreachable!("Page preload remove reply"),
        },
    ))
}
