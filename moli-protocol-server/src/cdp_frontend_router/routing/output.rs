use serde_json::{Value, json};

use super::{
    CdpFrontendRoutingState, CdpRoutedFrontend,
    frontend_registry::FrontendSessionKind,
    pending_commands::{CdpCommandFrontend, PendingCommandEffect},
};

impl CdpFrontendRoutingState {
    pub(in crate::cdp_frontend_router) fn route_message(
        &mut self,
        message: Value,
        wire_session_id: Option<&str>,
        attachment_flattened_protocol: Option<bool>,
    ) -> Option<(CdpRoutedFrontend, Value)> {
        if let Some(internal_command_id) = message.get("id").and_then(Value::as_u64) {
            return self.route_response(message, internal_command_id);
        }
        self.route_event(message, wire_session_id, attachment_flattened_protocol)
    }

    fn route_response(
        &mut self,
        mut message: Value,
        internal_command_id: u64,
    ) -> Option<(CdpRoutedFrontend, Value)> {
        let pending = self.pending_commands.take(internal_command_id)?;
        message["id"] = json!(pending.client_command_id);
        let CdpCommandFrontend {
            frontend_id,
            dispatch_session_id,
        } = pending.frontend;
        let sink = self.frontends.frontend_sink(frontend_id)?;
        if message.get("error").is_none() {
            match pending.effect {
                PendingCommandEffect::AttachToTarget {
                    target_id,
                    flattened_protocol,
                } => {
                    if let Some(child_session_id) = message
                        .pointer("/result/sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                    {
                        self.register_child_session(
                            frontend_id,
                            dispatch_session_id.as_deref(),
                            &child_session_id,
                            target_id.as_deref(),
                            flattened_protocol,
                        );
                    }
                }
                PendingCommandEffect::None => {}
            }
        }
        Some((
            CdpRoutedFrontend { frontend_id, sink },
            pending.output.encode(message),
        ))
    }

    fn route_event(
        &mut self,
        message: Value,
        wire_session_id: Option<&str>,
        attachment_flattened_protocol: Option<bool>,
    ) -> Option<(CdpRoutedFrontend, Value)> {
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned);
        debug_assert_eq!(
            message.get("sessionId").and_then(Value::as_str),
            wire_session_id,
            "the frozen delivery route must match the encoded wire session"
        );
        let target_event_session_id = message
            .pointer("/params/sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let target_event_target_id = message
            .pointer("/params/targetInfo/targetId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if method.as_deref() == Some("Target.receivedMessageFromTarget") {
            // This payload already contains the child's envelope. Only apply
            // the parent's route, including any non-flat ancestors.
            let session = self
                .frontends
                .session(target_event_session_id.as_deref()?)?;
            let frontend_id = session.frontend_id;
            let sink = self.frontends.frontend_sink(frontend_id)?;
            let parent = match &session.kind {
                FrontendSessionKind::Child {
                    parent_session_id, ..
                } => parent_session_id.as_deref(),
                FrontendSessionKind::Base => None,
            };
            let output = parent
                .and_then(|id| self.frontends.session(id))
                .map(|parent| parent.output.clone())
                .unwrap_or_default();
            return Some((
                CdpRoutedFrontend { frontend_id, sink },
                output.encode(message),
            ));
        }
        if method.as_deref() == Some("Target.attachedToTarget")
            && let Some(child_session_id) = target_event_session_id.as_deref()
        {
            if self.frontends.is_internal_control_session(child_session_id) {
                return None;
            }
            if let Some(parent_session_id) = wire_session_id
                && let Some(parent) = self.frontends.session(parent_session_id).cloned()
            {
                self.register_child_session(
                    parent.frontend_id,
                    Some(parent_session_id),
                    child_session_id,
                    target_event_target_id.as_deref(),
                    attachment_flattened_protocol.unwrap_or(true),
                );
            }
        }

        if let Some(session_id) = wire_session_id
            && self.frontends.is_internal_control_session(session_id)
        {
            return None;
        }

        if let Some(session_id) = wire_session_id
            && let Some(session) = self.frontends.session(session_id).cloned()
            && let Some(sink) = self.frontends.frontend_sink(session.frontend_id)
        {
            if method.as_deref() == Some("Target.detachedFromTarget")
                && let Some(detached_session_id) = target_event_session_id.as_deref()
            {
                self.frontends
                    .remove_child_session_cascade(detached_session_id);
            }
            return Some((
                CdpRoutedFrontend {
                    frontend_id: session.frontend_id,
                    sink,
                },
                session.output.encode(message),
            ));
        }
        if wire_session_id.is_some() {
            return None;
        }

        if method.as_deref() == Some("Target.detachedFromTarget")
            && let Some(session_id) = target_event_session_id.as_deref()
        {
            if self.frontends.remove_internal_control_session(session_id) {
                return None;
            }
            if let Some(session) = self.frontends.session(session_id).cloned() {
                match session.kind {
                    FrontendSessionKind::Base => {
                        // Base sessions are private transport adapters and
                        // never surface on their frontend's wire protocol.
                        self.frontends.remove_session_descendants(session_id);
                        return None;
                    }
                    FrontendSessionKind::Child {
                        parent_session_id, ..
                    } => {
                        let output = parent_session_id
                            .as_deref()
                            .and_then(|parent| self.frontends.session(parent))
                            .map(|parent| parent.output.clone())
                            .unwrap_or_default();
                        let sink = self.frontends.frontend_sink(session.frontend_id)?;
                        self.frontends.remove_child_session_cascade(session_id);
                        return Some((
                            CdpRoutedFrontend {
                                frontend_id: session.frontend_id,
                                sink,
                            },
                            output.encode(message),
                        ));
                    }
                }
            }
            self.frontends.remove_child_session_cascade(session_id);
        }

        None
    }

    pub(super) fn register_child_session(
        &mut self,
        frontend_id: u64,
        parent_session_id: Option<&str>,
        child_session_id: &str,
        target_id: Option<&str>,
        flattened_protocol: bool,
    ) {
        self.frontends.register_child_session(
            frontend_id,
            parent_session_id,
            child_session_id,
            target_id,
            flattened_protocol,
        );
    }
}
