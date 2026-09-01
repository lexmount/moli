//! CDP DOM domain.

use serde_json::Value;

use crate::conn::CdpConnection;
use crate::conn::Cmd;
use crate::domains::actions::DomAction;
use crate::domains::command_output::CommandOutputPlan;
use chromiumoxide_cdp::cdp::browser_protocol::dom::{EnableIncludeWhitespace, EnableParams};
use moli_core::page::{DocumentNodeRuntimeObjectResolution, Page, is_renderer_backend_node_id};

mod activity;
mod child_frame;
mod edit;
mod frontend_binding;
mod node_payload;
mod node_references;
#[cfg(test)]
mod patchright_shadow_tests;
mod resolve;
mod search;
mod set_file_input;
mod stack_traces;
#[cfg(test)]
mod tests;

pub(in crate::domains) use activity::{
    DomPreparedOutputSlot, DomPreparedOutputs, project_dom_mutations_async,
};
pub(crate) use resolve::{
    CompletedDomCommandDispatch, DomCommandTaskStep, PendingDomCommandDispatch,
    complete_pending_dom_command_output_plan, execute_devtools_dom_command_async,
};

#[cfg(test)]
use node_payload::{
    backend_node_id_for_snapshot, node_snapshot_base_payload, node_snapshot_to_cdp_with_limit,
};
use node_payload::{
    collect_flattened_node_snapshot, frontend_node_id_for_snapshot, node_snapshot_to_cdp,
};

pub(crate) enum DomCommandDispatchStep {
    Pending(Box<PendingDomCommandDispatch>),
    Complete(CommandOutputPlan),
}

pub(crate) fn try_start_dom_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<DomCommandDispatchStep> {
    let Some(action) = cmd.parse_action::<DomAction>() else {
        let plan = if conn.browser_context.is_none() {
            CommandOutputPlan::error_without_session(-31998, "BrowserContextNotLoaded")
        } else {
            CommandOutputPlan::error(-32601, "UnknownMethod")
        };
        return Some(DomCommandDispatchStep::Complete(plan));
    };
    if action == DomAction::Enable {
        let include_whitespace = match cmd.get_params::<EnableParams>() {
            Ok(Some(params)) => matches!(
                params.include_whitespace,
                Some(EnableIncludeWhitespace::All)
            ),
            Ok(None) => false,
            Err(_) => {
                return Some(DomCommandDispatchStep::Complete(CommandOutputPlan::error(
                    -32602,
                    "Invalid params",
                )));
            }
        };
        enable_dom_agent_for_session(conn, cmd.session_id, include_whitespace);
        return Some(DomCommandDispatchStep::Complete(
            CommandOutputPlan::success(),
        ));
    }
    if action == DomAction::Disable {
        if !dom_agent_enabled_for_session(conn, cmd.session_id) {
            return Some(DomCommandDispatchStep::Complete(CommandOutputPlan::error(
                -32000,
                "DOM agent hasn't been enabled",
            )));
        }
        disable_dom_agent_for_session(conn, cmd.session_id);
        return Some(match resolve::start_disable_dom_agent_command(conn, cmd) {
            Ok(Some(pending)) => DomCommandDispatchStep::Pending(Box::new(pending)),
            Ok(None) => DomCommandDispatchStep::Complete(CommandOutputPlan::success()),
            Err(error) => DomCommandDispatchStep::Complete(CommandOutputPlan::error(
                error.code,
                error.message,
            )),
        });
    }

    if action.requires_document_access()
        && let Err(message) = conn.ensure_document_accessible_for_session_owner(cmd.session_id)
    {
        return Some(DomCommandDispatchStep::Complete(CommandOutputPlan::error(
            -32000, message,
        )));
    }

    if action == DomAction::GetDocument {
        enable_dom_agent_for_session(conn, cmd.session_id, false);
    } else if action == DomAction::GetFlattenedDocument
        && !dom_agent_enabled_for_session(conn, cmd.session_id)
    {
        return Some(DomCommandDispatchStep::Complete(CommandOutputPlan::error(
            -32000,
            "DOM agent hasn't been enabled",
        )));
    }

    match action {
        DomAction::PushNodesByBackendIdsToFrontend => {
            if let Some(plan) = resolve::push_nodes_by_backend_ids_to_frontend(conn, cmd) {
                return Some(DomCommandDispatchStep::Complete(plan));
            }
        }
        DomAction::GetOuterHtml => {
            if let Some(plan) = resolve::get_outer_html(conn, cmd) {
                return Some(DomCommandDispatchStep::Complete(plan));
            }
        }
        DomAction::ScrollIntoViewIfNeeded => {
            if let Some(plan) = resolve::scroll_into_view_if_needed(conn, cmd) {
                return Some(DomCommandDispatchStep::Complete(plan));
            }
        }
        _ => {}
    }

    match resolve::try_start_pending_dom_command_result(conn, cmd) {
        Ok(Some(pending)) => Some(DomCommandDispatchStep::Pending(Box::new(pending))),
        Ok(None) => resolve::complete_non_pending_dom_command(conn, cmd)
            .map(DomCommandDispatchStep::Complete),
        Err((code, message)) => Some(DomCommandDispatchStep::Complete(CommandOutputPlan::error(
            code, message,
        ))),
    }
}

pub(crate) fn dom_agent_enabled_for_session(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> bool {
    conn.target_devtools_session_state_for_session(session_id)
        .is_some_and(|state| state.dom_session_state.enabled)
}

fn enable_dom_agent_for_session(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    include_whitespace: bool,
) -> bool {
    conn.with_target_devtools_session_state_for_session_mut(session_id, |state| {
        if !state.dom_session_state.enabled {
            state.dom_session_state.enabled = true;
            state.dom_session_state.include_whitespace = include_whitespace;
        }
    })
    .is_some()
}

fn disable_dom_agent_for_session(conn: &mut CdpConnection, session_id: Option<&str>) -> bool {
    conn.with_target_devtools_session_state_for_session_mut(session_id, |state| {
        state.dom_session_state.enabled = false;
        state.dom_session_state.include_whitespace = false;
    })
    .is_some()
}

pub(crate) fn dom_agent_includes_whitespace_for_session(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> bool {
    dom_agent_includes_whitespace_for_route(conn, session_id, None)
}

pub(crate) fn dom_agent_includes_whitespace_for_route(
    conn: &CdpConnection,
    session_id: Option<&str>,
    owner_route: Option<&crate::conn::CdpSessionRoute>,
) -> bool {
    conn.target_devtools_session_state_for_route(session_id, owner_route)
        .is_some_and(|state| {
            state.dom_session_state.enabled && state.dom_session_state.include_whitespace
        })
}

fn loaded_page_mut_for_session<'a>(
    conn: &'a mut CdpConnection,
    session_id: Option<&str>,
) -> Option<&'a mut Page> {
    conn.loaded_page_mut_for_protocol_access(session_id).ok()
}

fn target_owner_exists_for_session(conn: &CdpConnection, session_id: Option<&str>) -> bool {
    conn.target_owner_identity_for_session(session_id).is_some()
}

fn top_frame_id_for_session(conn: &CdpConnection, session_id: Option<&str>) -> Option<String> {
    conn.target_session_owner_frame_tree_identity(session_id)
        .map(|(frame_id, _, _, _)| frame_id)
}

fn cached_dom_remote_object_node_for_session(
    conn: &CdpConnection,
    session_id: Option<&str>,
    object_id: &str,
) -> Option<Value> {
    let (browser_context_id, target_id) = conn.target_owner_identity_for_session(session_id)?;
    let browser_context = conn.browser_context_by_id(&browser_context_id)?;
    let target_id = target_id
        .as_deref()
        .or_else(|| browser_context.active_target_id())?;
    browser_context
        .page_target(target_id)?
        .dom_remote_object_node_cache
        .get(object_id)
        .cloned()
}

fn cache_dom_remote_object_node_for_session(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    object_id: String,
    node: Value,
) {
    let Some((browser_context_id, target_id)) = conn.target_owner_identity_for_session(session_id)
    else {
        return;
    };
    if let Some(browser_context) = conn.browser_context_by_id_mut(&browser_context_id) {
        let target_id = target_id.or_else(|| browser_context.active_target_id_owned());
        let Some(target) = target_id
            .as_deref()
            .and_then(|target_id| browser_context.page_target_mut(target_id))
        else {
            return;
        };
        target.dom_remote_object_node_cache.insert(object_id, node);
    }
}
