//! Native frontend commands publish from the renderer, just like Inspector.
//!
//! The pending adapter retains only cancellation and Page-cache bookkeeping.
//! Its wake-up order cannot decide when a successful reply reaches the client.
//! Internal queries deliberately keep their ordinary typed result channel.

mod node;
pub(crate) use node::{NodeLookupExecution, NodeReferenceParams, with_backend_node};

use serde_json::json;

use moli_core::page::{CompletedPageCommand, PendingPageCommand};
use moli_core::{RendererNativeCommandResponseGuard, RendererNativeProtocolResponse};
use moli_core::{RendererPageCommand, RendererPageReply};

use super::command_output::CommandOutputPlan;
use crate::conn::{
    BackgroundProtocolEvent, CdpConnection, Cmd, CommandDispatchContext, CommandOwnerScope,
    TargetPageResidenceIdentity,
};

pub(crate) enum NativeCommandStep {
    Pending(PendingNativeCommand),
    Complete(CommandOutputPlan),
}

type UnpublishedFailureProjection =
    Box<dyn FnOnce(&CdpConnection, String) -> CommandOutputPlan + Send>;

pub(crate) struct PendingNativeCommand {
    // Retain wire routing only; Page settlement uses the exact residence.
    session_id: Option<String>,
    residence: TargetPageResidenceIdentity,
    command_id: u64,
    pending: PendingPageCommand,
    guard: RendererNativeCommandResponseGuard,
    unpublished_failure: Option<UnpublishedFailureProjection>,
}

pub(crate) struct CompletedNativeCommand {
    session_id: Option<String>,
    residence: TargetPageResidenceIdentity,
    pub(crate) command_id: u64,
    result: Result<CompletedPageCommand, String>,
    guard: RendererNativeCommandResponseGuard,
    unpublished_failure: Option<UnpublishedFailureProjection>,
}

impl PendingNativeCommand {
    /// Browser policy may outlive a rejected old attachment. This projection
    /// only runs after publication lost; it cannot replace a committed reply.
    pub(crate) fn on_unpublished_failure(
        mut self,
        project: impl FnOnce(&CdpConnection, String) -> CommandOutputPlan + Send + 'static,
    ) -> Self {
        self.unpublished_failure = Some(Box::new(project));
        self
    }

    pub(crate) fn command_id(&self) -> u64 {
        self.command_id
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(crate) async fn wait(self) -> CompletedNativeCommand {
        CompletedNativeCommand {
            session_id: self.session_id,
            residence: self.residence,
            command_id: self.command_id,
            result: self.pending.wait().await.map_err(|error| error.to_string()),
            guard: self.guard,
            unpublished_failure: self.unpublished_failure,
        }
    }
}

impl CompletedNativeCommand {
    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(crate) async fn complete(
        self,
        conn: &mut CdpConnection,
        context: &mut CommandDispatchContext,
    ) -> CommandOutputPlan {
        match self.result {
            Ok(completed) => {
                let output = conn.settle_page_command_turn_for_owner(&self.residence, completed);
                let completion = context.consume_renderer_command_turn_output(output);
                assert!(completion.native_response_was_published());
                CommandOutputPlan::default()
            }
            Err(error) => match self.guard.cancel_or_published().await {
                Some(fence) => {
                    context.set_renderer_output_predecessor(fence);
                    CommandOutputPlan::default()
                }
                None => match self.unpublished_failure {
                    Some(project) => project(conn, error),
                    None => CommandOutputPlan::error(-32000, error),
                },
            },
        }
    }
}

pub(crate) fn start(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    operation: RendererPageCommand,
    project: impl FnOnce(anyhow::Result<RendererPageReply>) -> RendererNativeProtocolResponse
    + Send
    + 'static,
) -> NativeCommandStep {
    start_operation(
        conn,
        cmd,
        moli_core::RendererNativeOperation::new(operation, project),
    )
}

pub(crate) fn start_operation(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    operation: moli_core::RendererNativeOperation,
) -> NativeCommandStep {
    let owner = CommandOwnerScope::capture(conn, cmd.session_id);
    let Some(residence) = conn.target_page_residence_identity_for_owner(&owner) else {
        return NativeCommandStep::Complete(CommandOutputPlan::error(-32000, "NoDocumentLoaded"));
    };
    let page = match conn.loaded_page_mut_for_protocol_access_for_owner(&owner) {
        Ok(page) => page,
        Err(error) => return NativeCommandStep::Complete(CommandOutputPlan::error(-32000, error)),
    };
    let command_id = cmd.id.expect("a native frontend command has a call id");
    match page.start_cdp_call(command_id, cmd.session_id, operation) {
        Ok((pending, guard)) => NativeCommandStep::Pending(PendingNativeCommand {
            session_id: cmd.session_id.map(str::to_owned),
            residence,
            command_id,
            pending,
            guard,
            unpublished_failure: None,
        }),
        Err(error) => {
            NativeCommandStep::Complete(CommandOutputPlan::error(-32000, error.to_string()))
        }
    }
}

/// Admit only frontend calls with an accessible document and renderer.
/// Return the checked attachment so preparation can retain its replay identity.
/// Enqueue revalidates access after preparation, which may update Browser policy.
pub(crate) fn frontend_attachment(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> Option<moli_page_types::RendererAgentAttachmentId> {
    if cmd.id.is_none()
        || cmd.terminal_response_delivery()
            != moli_page_types::RendererInspectorResponseDelivery::SessionSink
    {
        return None;
    }
    conn.loaded_page_mut_for_protocol_access_for_owner(&CommandOwnerScope::capture(
        conn,
        cmd.session_id,
    ))
    .ok()?
    .renderer_agent_attachment_id()
}

pub(crate) fn try_start(conn: &mut CdpConnection, cmd: &Cmd<'_>) -> Option<NativeCommandStep> {
    let start: fn(&mut CdpConnection, &Cmd<'_>) -> Option<NativeCommandStep> =
        match cmd.method.split_once('.')?.0 {
            "DOMDebugger" => super::dom_debugger::native::try_start,
            "Autofill" => super::autofill::try_start_native_command,
            "Page" => super::page::native::try_start,
            "DOM" => super::dom::try_start_native_command,
            "DOMSnapshot" => super::dom_snapshot::try_start_native_command,
            "CSS" => super::css::native::try_start,
            "Accessibility" => super::accessibility::native::try_start,
            _ => return None,
        };
    frontend_attachment(conn, cmd)?;
    start(conn, cmd)
}

/// Apply Browser mirrors at the same ordered boundary as the response. The
/// immutable result survives retirement, while document caches and node events
/// are valid only for the exact attachment that produced them.
pub(crate) fn project_terminal_for_owner(
    conn: &mut CdpConnection,
    publication_owner: Option<&CommandOwnerScope>,
    mut terminal: moli_core::RendererNativeCommandTerminal,
) -> Vec<BackgroundProtocolEvent> {
    let owner = publication_owner.map(|owner| match terminal.session.wire_session_id() {
        Some(session) => CommandOwnerScope::for_session(session),
        None => owner.clone(),
    });
    let owner = owner.filter(|owner| {
        conn.runtime_session_owner_slot_for_owner(owner)
            .is_ok_and(|slot| {
                slot.has_loaded_page()
                    && slot
                        .current_renderer_attachment()
                        .is_some_and(|attachment| attachment.id() == terminal.attachment_id)
            })
    });
    let response = &mut terminal.reply;
    if owner.is_none() {
        response.notifications.clear();
    }
    for update in std::mem::take(&mut response.state_updates) {
        let Some(owner) = owner.as_ref() else {
            continue;
        };
        match update {
            moli_core::RendererNativeProtocolStateUpdate::RemoteObjects { object_group } => {
                let Ok(result) = &response.result else {
                    continue;
                };
                if let Some(group) = object_group {
                    conn.register_runtime_remote_object_ids_from_value_for_owner_with_group(
                        owner, result, &group,
                    );
                } else {
                    conn.register_runtime_remote_object_ids_from_value_for_owner(owner, result);
                }
            }
            moli_core::RendererNativeProtocolStateUpdate::DomRemoteObjectNode {
                object_id,
                node,
            } => {
                super::dom::cache_dom_remote_object_node_for_owner(conn, owner, object_id, node);
            }
        }
    }
    project_terminal(terminal)
}

/// Bounded projection of a terminal already committed by its renderer owner.
/// It intentionally needs no live Page: replacing a document cannot change
/// an earlier snapshot or steal the original frontend call's response.
pub(crate) fn project_terminal(
    terminal: moli_core::RendererNativeCommandTerminal,
) -> Vec<BackgroundProtocolEvent> {
    let mut events = Vec::new();
    for notification in terminal.reply.notifications {
        let session = terminal.session.wire_session_id();
        events.push(match notification {
            moli_core::RendererNativeProtocolNotification::Json(mut notification) => {
                if let Some(session) = session {
                    notification["sessionId"] = json!(session);
                }
                BackgroundProtocolEvent::immediate(notification)
            }
            moli_core::RendererNativeProtocolNotification::DomSetChildNodes {
                parent_node_id,
                nodes,
            } => BackgroundProtocolEvent::dom_set_child_nodes(session, parent_node_id, nodes),
        });
    }
    let mut message = json!({ "id": terminal.command_id.get() });
    match terminal.reply.result {
        Ok(result) => message["result"] = result,
        Err(error) => message["error"] = json!({ "code": error.code, "message": error.message }),
    }
    if let Some(session) = terminal.session.wire_session_id() {
        message["sessionId"] = json!(session);
    }
    events.push(BackgroundProtocolEvent::immediate(message));
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_native_terminal_keeps_response_and_discards_document_side_effects() {
        use moli_core::{
            RendererNativeCommandTerminal, RendererNativeProtocolNotification,
            RendererNativeProtocolStateUpdate,
        };
        use moli_page_types::{DevToolsSessionKey, FrontendCommandId, RendererAgentAttachmentId};
        let mut ctx = crate::testing::TestContext::new();
        let mut response = RendererNativeProtocolResponse::success(
            json!({"object": {"objectId": "retired-object"}}),
        );
        response
            .notifications
            .push(RendererNativeProtocolNotification::DomSetChildNodes {
                parent_node_id: 3,
                nodes: vec![json!({"nodeId": 7})],
            });
        response
            .state_updates
            .push(RendererNativeProtocolStateUpdate::DomRemoteObjectNode {
                object_id: "retired-object".to_owned(),
                node: json!({"nodeId": 7}),
            });
        let messages = project_terminal_for_owner(
            &mut ctx.conn,
            None,
            RendererNativeCommandTerminal {
                command_id: FrontendCommandId::new(42),
                session: DevToolsSessionKey::Attached("retired-session".to_owned()),
                attachment_id: RendererAgentAttachmentId::allocate(),
                reply: response,
            },
        );
        assert_eq!(
            messages.len(),
            1,
            "a retired document cannot emit node notifications"
        );
        assert_eq!(
            messages.into_iter().next().unwrap().into_parts().0,
            json!({
                "id": 42, "sessionId": "retired-session", "result": {"object": {"objectId": "retired-object"}},
            })
        );
        assert!(
            ctx.conn.browser_context.is_none(),
            "projection cannot recreate a retired owner"
        );
    }

    #[test]
    fn native_dom_notification_keeps_automation_sidecar_before_terminal() {
        use moli_core::{RendererNativeCommandTerminal, RendererNativeProtocolNotification};
        use moli_page_types::{DevToolsSessionKey, FrontendCommandId, RendererAgentAttachmentId};
        let nodes = vec![json!({"nodeId": 7, "nodeName": "BUTTON"})];
        let mut response = RendererNativeProtocolResponse::success(json!({"nodeId": 7}));
        response
            .notifications
            .push(RendererNativeProtocolNotification::DomSetChildNodes {
                parent_node_id: 3,
                nodes: nodes.clone(),
            });
        let mut events = project_terminal(RendererNativeCommandTerminal {
            command_id: FrontendCommandId::new(42),
            session: DevToolsSessionKey::Attached("SID-native".to_owned()),
            attachment_id: RendererAgentAttachmentId::allocate(),
            reply: response,
        })
        .into_iter();
        let (event, sidecar) = events.next().unwrap().into_parts();
        assert_eq!(event["method"], "DOM.setChildNodes");
        assert_eq!(event["sessionId"], "SID-native");
        let Some(crate::automation::AutomationEvent::DomSetChildNodes(sidecar)) = sidecar else {
            panic!("native DOM notifications retain their typed automation event");
        };
        assert_eq!(sidecar.parent_node_id, 3);
        assert_eq!(sidecar.nodes, nodes);
        let (terminal, _) = events.next().unwrap().into_parts();
        assert_eq!(terminal["id"], 42);
        assert_eq!(terminal["sessionId"], "SID-native");
        assert_eq!(terminal["result"]["nodeId"], 7);
        assert!(events.next().is_none());
    }
}
