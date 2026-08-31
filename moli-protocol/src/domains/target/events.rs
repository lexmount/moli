#[cfg(test)]
use serde_json::Value;

use crate::devtools_runtime::{AutomationEvent, DevToolsTargetId, DevToolsTargetInfo};
#[cfg(test)]
use crate::devtools_runtime::{DevToolsSessionId, TargetAttachmentEvent, TargetDetachmentEvent};
use crate::domains::command_output::CommandOutputPlan;

use super::*;

pub(super) trait CdpTargetAutomationEventSink {
    fn push_target_background_event(&mut self, event: crate::conn::BackgroundProtocolEvent);
}

impl CdpTargetAutomationEventSink for Vec<crate::conn::BackgroundProtocolEvent> {
    fn push_target_background_event(&mut self, event: crate::conn::BackgroundProtocolEvent) {
        self.push(event);
    }
}

#[derive(Default)]
pub(super) struct TargetProtocolSideEffects {
    events: Vec<crate::conn::BackgroundProtocolEvent>,
}

impl TargetProtocolSideEffects {
    pub(super) fn background_events_mut(
        &mut self,
    ) -> &mut Vec<crate::conn::BackgroundProtocolEvent> {
        &mut self.events
    }

    pub(super) fn into_plan(self) -> CommandOutputPlan {
        let mut plan = CommandOutputPlan::default();
        for event in self.events {
            plan.push_background_event(event);
        }
        plan
    }

    pub(super) fn into_background_events(self) -> Vec<crate::conn::BackgroundProtocolEvent> {
        self.events
    }

    pub(super) fn extend_background_events(
        &mut self,
        events: impl IntoIterator<Item = crate::conn::BackgroundProtocolEvent>,
    ) {
        self.events.extend(events);
    }

    pub(super) fn drain_into_plan(&mut self) -> CommandOutputPlan {
        let mut plan = CommandOutputPlan::default();
        for event in std::mem::take(&mut self.events) {
            plan.push_background_event(event);
        }
        plan
    }
}

pub(super) fn inspector_detached_event(
    session_id: &str,
    reason: &str,
) -> crate::conn::BackgroundProtocolEvent {
    crate::conn::BackgroundProtocolEvent::inspector_detached(Some(session_id), reason)
}

impl CdpTargetAutomationEventSink for TargetProtocolSideEffects {
    fn push_target_background_event(&mut self, event: crate::conn::BackgroundProtocolEvent) {
        self.events.push(event);
    }
}

#[cfg(test)]
pub(super) fn emit_attached_to_target_with_waiting(
    out: &mut impl CdpTargetAutomationEventSink,
    session_id: &str,
    target_info: DevToolsTargetInfo,
    parent_session_id: Option<&str>,
    waiting_for_debugger: bool,
) {
    let target_id = target_info
        .target_id
        .clone()
        .unwrap_or_else(|| DevToolsTargetId::from(""));
    emit_cdp_target_automation_event(
        out,
        AutomationEvent::TargetAttached(TargetAttachmentEvent {
            target_id,
            session_id: DevToolsSessionId::from(session_id),
            parent_session_id: parent_session_id.map(DevToolsSessionId::from),
            target_info,
            waiting_for_debugger,
        }),
    );
}

#[cfg(test)]
fn emit_attached_to_target_from_cdp_value(
    out: &mut impl CdpTargetAutomationEventSink,
    session_id: &str,
    target_info: Value,
    parent_session_id: Option<&str>,
) {
    emit_attached_to_target_with_waiting(
        out,
        session_id,
        devtools_target_info_from_cdp_value_lossy(target_info),
        parent_session_id,
        false,
    );
}

#[cfg(test)]
fn emit_attached_to_target_with_waiting_from_cdp_value(
    out: &mut impl CdpTargetAutomationEventSink,
    session_id: &str,
    target_info: Value,
    parent_session_id: Option<&str>,
    waiting_for_debugger: bool,
) {
    emit_attached_to_target_with_waiting(
        out,
        session_id,
        devtools_target_info_from_cdp_value_lossy(target_info),
        parent_session_id,
        waiting_for_debugger,
    );
}

pub(super) fn target_created_automation_event(target_info: DevToolsTargetInfo) -> AutomationEvent {
    let target_id = target_info
        .target_id
        .clone()
        .unwrap_or_else(|| DevToolsTargetId::from(""));
    let kind = target_info.kind;
    let browser_context_id = target_info.browser_context_id.clone();
    let url = target_info.url.clone();
    AutomationEvent::TargetCreated(crate::devtools_runtime::TargetLifecycleEvent {
        target_id,
        browser_context_id,
        kind,
        url,
        target_info: Some(target_info),
    })
}

#[cfg(test)]
pub(super) fn emit_target_created(
    out: &mut impl CdpTargetAutomationEventSink,
    target_info: DevToolsTargetInfo,
) {
    emit_cdp_target_automation_event(out, target_created_automation_event(target_info));
}

#[cfg(test)]
pub(super) fn emit_target_created_from_cdp_value(
    out: &mut impl CdpTargetAutomationEventSink,
    target_info: Value,
) {
    emit_target_created(out, devtools_target_info_from_cdp_value_lossy(target_info));
}

#[cfg(test)]
fn devtools_target_info_from_cdp_value_lossy(value: Value) -> DevToolsTargetInfo {
    DevToolsTargetInfo {
        target_id: value
            .get("targetId")
            .and_then(Value::as_str)
            .map(DevToolsTargetId::from),
        kind: crate::devtools_runtime::DevToolsTargetKind::from_cdp_type(
            value.get("type").and_then(Value::as_str),
        ),
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        url: value
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        attached: value
            .get("attached")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        opener_id: value
            .get("openerId")
            .and_then(Value::as_str)
            .map(DevToolsTargetId::from),
        opener_frame_id: value
            .get("openerFrameId")
            .and_then(Value::as_str)
            .map(crate::devtools_runtime::DevToolsFrameId::from),
        can_access_opener: value
            .get("canAccessOpener")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        browser_context_id: value
            .get("browserContextId")
            .and_then(Value::as_str)
            .map(crate::devtools_runtime::DevToolsBrowserContextId::from),
        moli_popup_id: None,
    }
}

#[cfg(test)]
pub(super) fn emit_target_destroyed(out: &mut impl CdpTargetAutomationEventSink, target_id: &str) {
    emit_cdp_target_automation_event(
        out,
        AutomationEvent::TargetDestroyed(crate::devtools_runtime::TargetLifecycleEvent {
            target_id: DevToolsTargetId::from(target_id),
            browser_context_id: None,
            kind: crate::devtools_runtime::DevToolsTargetKind::Other,
            url: String::new(),
            target_info: None,
        }),
    );
}

#[cfg(test)]
pub(super) fn emit_detached_from_target(
    out: &mut impl CdpTargetAutomationEventSink,
    target_id: &str,
    session_id: &str,
    reason: Option<&str>,
) {
    emit_detached_from_target_with_parent(out, target_id, session_id, reason, None);
}

#[cfg(test)]
pub(super) fn emit_detached_from_target_with_parent(
    out: &mut impl CdpTargetAutomationEventSink,
    target_id: &str,
    session_id: &str,
    reason: Option<&str>,
    parent_session_id: Option<&str>,
) {
    emit_cdp_target_automation_event(
        out,
        AutomationEvent::TargetDetached(TargetDetachmentEvent {
            target_id: DevToolsTargetId::from(target_id),
            session_id: DevToolsSessionId::from(session_id),
            parent_session_id: parent_session_id.map(DevToolsSessionId::from),
            reason: reason.map(str::to_owned),
        }),
    );
}

#[cfg(test)]
fn emit_cdp_target_automation_event(
    out: &mut impl CdpTargetAutomationEventSink,
    event: AutomationEvent,
) {
    match event {
        AutomationEvent::TargetCreated(event) => {
            out.push_target_background_event(crate::conn::BackgroundProtocolEvent::target_created(
                None, event,
            ));
        }
        AutomationEvent::TargetDestroyed(event) => {
            out.push_target_background_event(
                crate::conn::BackgroundProtocolEvent::target_destroyed(None, event),
            );
        }
        AutomationEvent::TargetAttached(event) => {
            out.push_target_background_event(
                crate::conn::BackgroundProtocolEvent::target_attached(event),
            );
        }
        AutomationEvent::TargetDetached(event) => {
            out.push_target_background_event(
                crate::conn::BackgroundProtocolEvent::target_detached(event),
            );
        }
        _ => {}
    }
}

pub(super) async fn fail_pending_fetch_state_for_target_background_events_async(
    conn: &mut CdpConnection,
    out: &mut Vec<crate::conn::BackgroundProtocolEvent>,
    session_id: Option<&str>,
    reason: &str,
) -> Option<moli_core::RendererOutputFence> {
    let (
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    ) = page::take_pending_fetch_state(conn, session_id);

    page::fail_pending_fetch_state_background_events_async(
        conn,
        out,
        session_id,
        reason,
        reason,
        pending_navigations,
        pending_auth_navigations,
        pending_response_navigations,
        pending_subresource_fetches,
        pending_subresource_auths,
        pending_subresource_responses,
    )
    .await
}

#[cfg(test)]
mod tests {
    use crate::conn::BackgroundProtocolEvent;
    use crate::devtools_runtime::{
        AutomationEvent, DevToolsSessionId, DevToolsTargetId, DevToolsTargetKind,
        TargetDetachmentEvent, TargetLifecycleEvent,
    };
    use serde_json::json;

    fn pop_protocol_message(out: &mut Vec<BackgroundProtocolEvent>) -> serde_json::Value {
        out.pop()
            .expect("target event should be emitted")
            .into_protocol_message()
    }

    #[test]
    fn target_created_serializes_from_automation_event_shape() {
        let mut out: Vec<BackgroundProtocolEvent> = Vec::new();

        super::emit_target_created_from_cdp_value(
            &mut out,
            json!({
                "targetId": "TID-created",
                "type": "page",
                "url": "https://example.test/",
                "browserContextId": "BID-created",
                "attached": false,
                "canAccessOpener": false
            }),
        );

        let message = pop_protocol_message(&mut out);
        assert!(out.is_empty());
        assert_eq!(message["method"], json!("Target.targetCreated"));
        assert_eq!(
            message["params"]["targetInfo"]["targetId"],
            json!("TID-created")
        );
        assert_eq!(
            message["params"]["targetInfo"]["browserContextId"],
            json!("BID-created")
        );
        assert_eq!(
            message["params"]["targetInfo"]["url"],
            json!("https://example.test/")
        );
    }

    #[test]
    fn target_created_preserves_typed_sidecar_for_background_output() {
        let mut out: Vec<crate::conn::BackgroundProtocolEvent> = Vec::new();

        super::emit_target_created_from_cdp_value(
            &mut out,
            json!({
                "targetId": "TID-created",
                "type": "page",
                "url": "https://example.test/",
                "browserContextId": "BID-created",
                "attached": false,
                "canAccessOpener": false
            }),
        );

        let (message, automation_event) = out
            .pop()
            .expect("target event should be emitted")
            .into_parts();
        assert_eq!(message["method"], json!("Target.targetCreated"));
        let Some(AutomationEvent::TargetCreated(event)) = automation_event else {
            panic!("expected TargetCreated automation sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-created");
        assert_eq!(event.kind, DevToolsTargetKind::Page);
        assert_eq!(
            event.browser_context_id.as_ref().map(|id| id.as_str()),
            Some("BID-created")
        );
    }

    #[test]
    fn target_created_sidecar_preserves_shared_worker_kind() {
        let mut out: Vec<crate::conn::BackgroundProtocolEvent> = Vec::new();

        super::emit_target_created_from_cdp_value(
            &mut out,
            json!({
                "targetId": "TID-worker",
                "type": "shared_worker",
                "url": "https://example.test/shared-worker.js",
                "browserContextId": "BID-worker",
                "attached": false,
                "canAccessOpener": false
            }),
        );

        let (message, automation_event) = out
            .pop()
            .expect("target event should be emitted")
            .into_parts();
        assert_eq!(
            message["params"]["targetInfo"]["type"],
            json!("shared_worker")
        );
        let Some(AutomationEvent::TargetCreated(event)) = automation_event else {
            panic!("expected TargetCreated automation sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-worker");
        assert_eq!(event.kind, DevToolsTargetKind::SharedWorker);
    }

    #[test]
    fn target_created_fallback_serializes_shared_worker_type() {
        let mut out: Vec<BackgroundProtocolEvent> = Vec::new();

        super::emit_cdp_target_automation_event(
            &mut out,
            AutomationEvent::TargetCreated(TargetLifecycleEvent {
                target_id: DevToolsTargetId::from("TID-worker"),
                browser_context_id: None,
                kind: DevToolsTargetKind::SharedWorker,
                url: "https://example.test/shared-worker.js".to_owned(),
                target_info: None,
            }),
        );

        let message = pop_protocol_message(&mut out);
        assert!(out.is_empty());
        assert_eq!(
            message["params"]["targetInfo"]["type"],
            json!("shared_worker")
        );
    }

    #[test]
    fn target_destroyed_serializes_from_automation_event_shape() {
        let mut out: Vec<BackgroundProtocolEvent> = Vec::new();

        super::emit_target_destroyed(&mut out, "TID-destroyed");

        let message = pop_protocol_message(&mut out);
        assert!(out.is_empty());
        assert_eq!(message["method"], json!("Target.targetDestroyed"));
        assert_eq!(message["params"]["targetId"], json!("TID-destroyed"));
    }

    #[test]
    fn attached_to_target_serializes_from_automation_event_shape() {
        let mut out: Vec<BackgroundProtocolEvent> = Vec::new();

        super::emit_attached_to_target_from_cdp_value(
            &mut out,
            "SID-child",
            json!({
                "targetId": "TID-child",
                "type": "page",
                "url": "about:blank",
                "attached": true,
                "canAccessOpener": false
            }),
            Some("SID-parent"),
        );

        let message = pop_protocol_message(&mut out);
        assert!(out.is_empty());
        assert_eq!(message["method"], json!("Target.attachedToTarget"));
        assert_eq!(message["sessionId"], json!("SID-parent"));
        assert_eq!(message["params"]["sessionId"], json!("SID-child"));
        assert_eq!(
            message["params"]["targetInfo"]["targetId"],
            json!("TID-child")
        );
        assert_eq!(message["params"]["waitingForDebugger"], json!(false));
    }

    #[test]
    fn attached_to_target_preserves_typed_sidecar_for_background_output() {
        let mut out: Vec<crate::conn::BackgroundProtocolEvent> = Vec::new();

        super::emit_attached_to_target_from_cdp_value(
            &mut out,
            "SID-child",
            json!({
                "targetId": "TID-child",
                "type": "page",
                "url": "about:blank",
                "attached": true,
                "canAccessOpener": false
            }),
            Some("SID-parent"),
        );

        let (message, automation_event) = out
            .pop()
            .expect("target attach event should be emitted")
            .into_parts();
        assert_eq!(message["method"], json!("Target.attachedToTarget"));
        assert_eq!(message["sessionId"], json!("SID-parent"));
        let Some(AutomationEvent::TargetAttached(event)) = automation_event else {
            panic!("expected TargetAttached automation sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-child");
        assert_eq!(event.session_id.as_str(), "SID-child");
        assert_eq!(
            event.parent_session_id.as_ref().map(|id| id.as_str()),
            Some("SID-parent")
        );
        assert!(!event.waiting_for_debugger);
    }

    #[test]
    fn attached_to_target_serializes_waiting_for_debugger() {
        let mut out: Vec<BackgroundProtocolEvent> = Vec::new();

        super::emit_attached_to_target_with_waiting_from_cdp_value(
            &mut out,
            "SID-child",
            json!({
                "targetId": "TID-child",
                "type": "service_worker",
                "url": "https://example.test/service-worker.js",
                "attached": true,
                "canAccessOpener": false
            }),
            Some("SID-parent"),
            true,
        );

        let message = pop_protocol_message(&mut out);
        assert!(out.is_empty());
        assert_eq!(message["method"], json!("Target.attachedToTarget"));
        assert_eq!(message["sessionId"], json!("SID-parent"));
        assert_eq!(message["params"]["sessionId"], json!("SID-child"));
        assert_eq!(message["params"]["waitingForDebugger"], json!(true));
    }

    #[test]
    fn detached_from_target_serializes_from_automation_event_shape() {
        let mut out: Vec<BackgroundProtocolEvent> = Vec::new();

        super::emit_detached_from_target(
            &mut out,
            "TID-detached",
            "SID-detached",
            Some("Render process gone."),
        );

        let message = pop_protocol_message(&mut out);
        assert!(out.is_empty());
        assert_eq!(message["method"], json!("Target.detachedFromTarget"));
        assert!(message.get("sessionId").is_none());
        assert_eq!(message["params"]["targetId"], json!("TID-detached"));
        assert_eq!(message["params"]["sessionId"], json!("SID-detached"));
        assert!(message["params"].get("reason").is_none());
    }

    #[test]
    fn target_protocol_side_effects_preserve_event_order_and_detached_sidecar() {
        let mut out = super::TargetProtocolSideEffects::default();
        out.extend_background_events([super::inspector_detached_event(
            "SID-child",
            "Target detached",
        )]);
        super::emit_target_destroyed(&mut out, "TID-destroyed");

        super::emit_detached_from_target_with_parent(
            &mut out,
            "TID-detached",
            "SID-child",
            None,
            Some("SID-parent"),
        );

        let mut events = out.into_background_events();
        assert_eq!(events.len(), 3);
        let (message, automation_event) = events.remove(0).into_parts();
        assert_eq!(message["method"], json!("Inspector.detached"));
        assert!(automation_event.is_none());

        let (message, automation_event) = events.remove(0).into_parts();
        assert_eq!(message["method"], json!("Target.targetDestroyed"));
        let Some(AutomationEvent::TargetDestroyed(event)) = automation_event else {
            panic!("expected TargetDestroyed automation sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-destroyed");

        let (message, automation_event) = events.remove(0).into_parts();
        assert_eq!(message["method"], json!("Target.detachedFromTarget"));
        assert_eq!(message["sessionId"], json!("SID-parent"));
        let Some(AutomationEvent::TargetDetached(event)) = automation_event else {
            panic!("expected TargetDetached automation sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-detached");
        assert_eq!(event.session_id.as_str(), "SID-child");
        assert_eq!(
            event.parent_session_id.as_ref().map(|id| id.as_str()),
            Some("SID-parent")
        );
    }

    #[test]
    fn target_protocol_side_effects_keep_event_order_before_background_event_mutation() {
        let mut out = super::TargetProtocolSideEffects::default();
        super::emit_target_destroyed(&mut out, "TID-before-background");

        out.background_events_mut()
            .push(crate::conn::BackgroundProtocolEvent::target_detached(
                TargetDetachmentEvent {
                    target_id: DevToolsTargetId::from("TID-after-background"),
                    session_id: DevToolsSessionId::from("SID-after-background"),
                    parent_session_id: None,
                    reason: None,
                },
            ));

        let mut events = out.into_background_events();
        assert_eq!(events.len(), 2);
        let (message, automation_event) = events.remove(0).into_parts();
        assert_eq!(message["method"], json!("Target.targetDestroyed"));
        let Some(AutomationEvent::TargetDestroyed(event)) = automation_event else {
            panic!("expected first TargetDestroyed sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-before-background");

        let (message, automation_event) = events.remove(0).into_parts();
        assert_eq!(message["method"], json!("Target.detachedFromTarget"));
        let Some(AutomationEvent::TargetDetached(event)) = automation_event else {
            panic!("expected later TargetDetached sidecar");
        };
        assert_eq!(event.target_id.as_str(), "TID-after-background");
    }
}
