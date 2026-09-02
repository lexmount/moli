use crate::devtools_runtime::{
    AutomationEvent, DevToolsFrameId, DevToolsRealmId, DevToolsTargetId,
    RuntimeExecutionContextEvent, RuntimeExecutionContextsClearedEvent,
};
use moli_core::page::{
    RuntimeContextRestoreEvent, RuntimeExecutionContextRestoreEvent,
    RuntimeExecutionContextsClearedRestoreEvent,
};
use serde_json::Value;
#[cfg(test)]
use serde_json::json;

use crate::conn::{BackgroundProtocolEvent, CdpConnection, CdpSessionRoute, CommandOwnerScope};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RuntimeContextProtocolEvent {
    Created(RuntimeExecutionContextEvent),
    Destroyed(RuntimeExecutionContextEvent),
    Cleared(RuntimeExecutionContextsClearedEvent),
}

impl RuntimeContextProtocolEvent {
    pub(crate) fn from_context_protocol_message(message: Value) -> Option<Self> {
        match message["method"].as_str()? {
            "Runtime.executionContextCreated" => Some(Self::Created(
                runtime_context_created_event_from_cdp_params(message["params"].clone()),
            )),
            "Runtime.executionContextDestroyed" => Some(Self::Destroyed(
                runtime_context_destroyed_event_from_cdp_params(message["params"].clone()),
            )),
            "Runtime.executionContextsCleared" => Some(Self::Cleared(
                runtime_contexts_cleared_event_from_cdp_params(message["params"].clone()),
            )),
            _ => None,
        }
    }

    pub(crate) fn from_restore_event(event: RuntimeContextRestoreEvent) -> Self {
        runtime_context_protocol_event_from_restore_event(event)
    }
}

pub(crate) fn runtime_context_protocol_event_from_restore_event(
    event: RuntimeContextRestoreEvent,
) -> RuntimeContextProtocolEvent {
    match event {
        RuntimeContextRestoreEvent::Created(event) => RuntimeContextProtocolEvent::Created(
            runtime_context_created_event_from_restore_event(event),
        ),
        RuntimeContextRestoreEvent::Destroyed(event) => RuntimeContextProtocolEvent::Destroyed(
            runtime_context_destroyed_event_from_restore_event(event),
        ),
        RuntimeContextRestoreEvent::Cleared(event) => RuntimeContextProtocolEvent::Cleared(
            runtime_contexts_cleared_event_from_restore_event(event),
        ),
    }
}

fn runtime_context_created_event_from_restore_event(
    event: RuntimeExecutionContextRestoreEvent,
) -> RuntimeExecutionContextEvent {
    RuntimeExecutionContextEvent {
        target_id: None,
        context_id: event.context_id,
        realm_id: event.realm_id.as_deref().map(DevToolsRealmId::from),
        frame_id: event.frame_id.as_deref().map(DevToolsFrameId::from),
        origin: event.origin,
        name: event.name,
        is_default: event.is_default,
        context_type: event.context_type,
        grant_universal_access: event.grant_universal_access,
    }
}

fn runtime_context_destroyed_event_from_restore_event(
    event: RuntimeExecutionContextRestoreEvent,
) -> RuntimeExecutionContextEvent {
    RuntimeExecutionContextEvent {
        target_id: None,
        context_id: event.context_id,
        realm_id: event.realm_id.as_deref().map(DevToolsRealmId::from),
        frame_id: event.frame_id.as_deref().map(DevToolsFrameId::from),
        origin: event.origin,
        name: event.name,
        is_default: event.is_default,
        context_type: event.context_type,
        grant_universal_access: event.grant_universal_access,
    }
}

fn runtime_contexts_cleared_event_from_restore_event(
    _event: RuntimeExecutionContextsClearedRestoreEvent,
) -> RuntimeExecutionContextsClearedEvent {
    RuntimeExecutionContextsClearedEvent { target_id: None }
}

pub(crate) fn emit_cdp_runtime_context_background_automation_event(
    out: &mut Vec<BackgroundProtocolEvent>,
    event: AutomationEvent,
    session_id: Option<&str>,
) {
    match event {
        AutomationEvent::RuntimeExecutionContextCreated(event) => {
            out.push(BackgroundProtocolEvent::runtime_execution_context_created(
                session_id, event,
            ));
        }
        AutomationEvent::RuntimeExecutionContextDestroyed(event) => {
            out.push(
                BackgroundProtocolEvent::runtime_execution_context_destroyed(session_id, event),
            );
        }
        AutomationEvent::RuntimeExecutionContextsCleared(event) => {
            out.push(BackgroundProtocolEvent::runtime_execution_contexts_cleared(
                session_id, event,
            ));
        }
        _ => {}
    }
}

pub(crate) fn runtime_context_created_event_from_cdp_params(
    params: Value,
) -> RuntimeExecutionContextEvent {
    let context = &params["context"];
    let aux_data = &context["auxData"];
    RuntimeExecutionContextEvent {
        target_id: None,
        context_id: context["id"].as_i64(),
        realm_id: context["uniqueId"].as_str().map(DevToolsRealmId::from),
        frame_id: aux_data["frameId"].as_str().map(DevToolsFrameId::from),
        origin: context["origin"].as_str().map(str::to_owned),
        name: context["name"].as_str().map(str::to_owned),
        is_default: aux_data["isDefault"].as_bool(),
        context_type: aux_data["type"].as_str().map(str::to_owned),
        grant_universal_access: aux_data["grantUniversalAccess"].as_bool(),
    }
}

pub(crate) fn runtime_context_destroyed_event_from_cdp_params(
    params: Value,
) -> RuntimeExecutionContextEvent {
    RuntimeExecutionContextEvent {
        target_id: None,
        context_id: params["executionContextId"].as_i64(),
        realm_id: params["executionContextUniqueId"]
            .as_str()
            .map(DevToolsRealmId::from),
        frame_id: None,
        origin: None,
        name: None,
        is_default: None,
        context_type: None,
        grant_universal_access: None,
    }
}

pub(crate) fn runtime_contexts_cleared_event_from_cdp_params(
    _params: Value,
) -> RuntimeExecutionContextsClearedEvent {
    RuntimeExecutionContextsClearedEvent { target_id: None }
}

#[cfg(test)]
pub(crate) fn apply_runtime_context_protocol_event_side_effects(
    conn: &mut CdpConnection,
    event: &Value,
    session_id: Option<&str>,
) {
    match event["method"].as_str() {
        Some("Runtime.executionContextsCleared") => {
            conn.clear_runtime_remote_object_tracking_for_session_owner(session_id);
            conn.record_runtime_contexts_cleared_for_session_owner(session_id);
        }
        Some("Runtime.executionContextCreated") => {
            conn.record_runtime_contexts_reported_for_session_owner(session_id);
        }
        Some("Runtime.executionContextDestroyed") => {
            if let Some(realm_id) = event["params"]["executionContextUniqueId"].as_str() {
                conn.clear_runtime_remote_objects_for_realm_for_session_owner(session_id, realm_id);
            }
        }
        _ => {}
    }
}

pub(crate) fn apply_runtime_context_protocol_event_side_effects_typed(
    conn: &mut CdpConnection,
    event: &RuntimeContextProtocolEvent,
    session_id: Option<&str>,
) {
    conn.record_runtime_context_protocol_event_for_session_owner(session_id, event);
    match event {
        RuntimeContextProtocolEvent::Cleared(_) => {
            conn.clear_runtime_remote_object_tracking_for_session_owner(session_id);
            conn.record_runtime_contexts_cleared_for_session_owner(session_id);
        }
        RuntimeContextProtocolEvent::Created(event) => {
            conn.record_runtime_contexts_reported_for_session_owner(session_id);
            record_child_default_context_delivery(conn, session_id, event);
        }
        RuntimeContextProtocolEvent::Destroyed(event) => {
            if let Some(realm_id) = event.realm_id.as_ref() {
                conn.clear_runtime_remote_objects_for_realm_for_session_owner(
                    session_id,
                    realm_id.as_str(),
                );
            }
        }
    }
}

pub(crate) fn apply_runtime_context_protocol_event_side_effects_for_owner_typed(
    conn: &mut CdpConnection,
    event: &RuntimeContextProtocolEvent,
    owner: &CommandOwnerScope,
) {
    if owner.session_id().is_some() {
        apply_runtime_context_protocol_event_side_effects_typed(conn, event, owner.session_id());
        return;
    }
    conn.record_runtime_context_protocol_event_for_owner(owner, event);
    match event {
        RuntimeContextProtocolEvent::Cleared(_) => {
            conn.clear_runtime_remote_object_tracking_for_owner(owner);
            conn.record_runtime_contexts_cleared_for_owner(owner);
        }
        RuntimeContextProtocolEvent::Created(event) => {
            conn.record_runtime_contexts_reported_for_owner(owner);
            record_child_default_context_delivery_for_owner(conn, owner, event);
        }
        RuntimeContextProtocolEvent::Destroyed(event) => {
            if let Some(realm_id) = event.realm_id.as_ref() {
                conn.clear_runtime_remote_objects_for_realm_for_owner(owner, realm_id.as_str());
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn qualify_runtime_context_protocol_event_for_session_owner(
    conn: &CdpConnection,
    event: &mut Value,
    session_id: Option<&str>,
) {
    let Some((_, Some(target_id))) = conn.runtime_context_owner_identity_for_session(session_id)
    else {
        return;
    };
    match event["method"].as_str() {
        Some("Runtime.executionContextCreated") => {
            qualify_runtime_realm_id_value(&target_id, &mut event["params"]["context"]["uniqueId"]);
        }
        Some("Runtime.executionContextDestroyed") => {
            qualify_runtime_realm_id_value(
                &target_id,
                &mut event["params"]["executionContextUniqueId"],
            );
        }
        _ => {}
    }
}

pub(crate) fn qualify_runtime_context_protocol_event_for_session_owner_typed(
    conn: &CdpConnection,
    event: &mut RuntimeContextProtocolEvent,
    session_id: Option<&str>,
) {
    qualify_worker_runtime_context_event_for_session_owner(conn, event, session_id);
    let Some((_, Some(target_id))) = conn.runtime_context_owner_identity_for_session(session_id)
    else {
        return;
    };
    match event {
        RuntimeContextProtocolEvent::Created(event) => {
            qualify_runtime_realm_id(&target_id, &mut event.realm_id);
        }
        RuntimeContextProtocolEvent::Destroyed(event) => {
            qualify_runtime_realm_id(&target_id, &mut event.realm_id);
        }
        RuntimeContextProtocolEvent::Cleared(_) => {}
    }
}

pub(crate) fn qualify_runtime_context_protocol_event_for_owner_typed(
    conn: &CdpConnection,
    event: &mut RuntimeContextProtocolEvent,
    owner: &CommandOwnerScope,
) {
    if owner.session_id().is_some() {
        qualify_runtime_context_protocol_event_for_session_owner_typed(
            conn,
            event,
            owner.session_id(),
        );
        return;
    }
    let Some((_, Some(target_id))) = conn.target_owner_identity_for_owner(owner) else {
        return;
    };
    match event {
        RuntimeContextProtocolEvent::Created(event) => {
            qualify_runtime_realm_id(&target_id, &mut event.realm_id);
        }
        RuntimeContextProtocolEvent::Destroyed(event) => {
            qualify_runtime_realm_id(&target_id, &mut event.realm_id);
        }
        RuntimeContextProtocolEvent::Cleared(_) => {}
    }
}

fn qualify_worker_runtime_context_event_for_session_owner(
    conn: &CdpConnection,
    event: &mut RuntimeContextProtocolEvent,
    session_id: Option<&str>,
) {
    let Some(route) = conn.session_route(session_id) else {
        return;
    };
    match route {
        CdpSessionRoute::SharedWorkerTarget { target_id, .. }
        | CdpSessionRoute::DedicatedWorkerTarget { target_id, .. } => {
            tag_runtime_context_event_with_target_id(event, &target_id);
        }
        CdpSessionRoute::ServiceWorkerTarget { target_id, .. } => {
            tag_runtime_context_event_with_target_id(event, &target_id);
            if let RuntimeContextProtocolEvent::Created(event) = event {
                event.context_type = Some("service-worker".to_owned());
                event.frame_id = None;
            }
        }
        CdpSessionRoute::Browser
        | CdpSessionRoute::BrowserContext { .. }
        | CdpSessionRoute::TabTarget { .. }
        | CdpSessionRoute::PageTarget { .. } => {}
    }
}

fn tag_runtime_context_event_with_target_id(
    event: &mut RuntimeContextProtocolEvent,
    target_id: &str,
) {
    match event {
        RuntimeContextProtocolEvent::Created(event)
        | RuntimeContextProtocolEvent::Destroyed(event) => {
            event.target_id = Some(DevToolsTargetId::from(target_id));
        }
        RuntimeContextProtocolEvent::Cleared(event) => {
            event.target_id = Some(DevToolsTargetId::from(target_id));
        }
    }
}

#[cfg(test)]
fn qualify_runtime_realm_id_value(target_id: &str, value: &mut Value) {
    let Some(native_realm_id) = value.as_str() else {
        return;
    };
    if native_realm_id.is_empty() || native_realm_id.starts_with(&format!("{target_id}:")) {
        return;
    }
    *value = json!(format!("{target_id}:{native_realm_id}"));
}

fn qualify_runtime_realm_id(target_id: &str, realm_id: &mut Option<DevToolsRealmId>) {
    let Some(native_realm_id) = realm_id.as_ref().map(DevToolsRealmId::as_str) else {
        return;
    };
    if native_realm_id.is_empty() || native_realm_id.starts_with(&format!("{target_id}:")) {
        return;
    }
    *realm_id = Some(DevToolsRealmId::from(format!(
        "{target_id}:{native_realm_id}"
    )));
}

pub(crate) fn emit_runtime_context_protocol_background_event_typed(
    out: &mut Vec<BackgroundProtocolEvent>,
    event: RuntimeContextProtocolEvent,
    session_id: Option<&str>,
) {
    let automation_event = match event {
        RuntimeContextProtocolEvent::Created(event) => {
            AutomationEvent::RuntimeExecutionContextCreated(event)
        }
        RuntimeContextProtocolEvent::Destroyed(event) => {
            AutomationEvent::RuntimeExecutionContextDestroyed(event)
        }
        RuntimeContextProtocolEvent::Cleared(event) => {
            AutomationEvent::RuntimeExecutionContextsCleared(event)
        }
    };
    emit_cdp_runtime_context_background_automation_event(out, automation_event, session_id);
}

/// Applies only to the live-inventory replay performed by `Runtime.enable`.
///
/// Real-time context lifecycle messages come from V8's inspector journal and
/// must never be suppressed here. Keeping the replay cursor at this boundary
/// makes repeated `Runtime.enable` calls idempotent without turning the cursor
/// into a second real-time event arbiter.
pub(crate) fn should_emit_child_default_context_inventory_replay_once_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    root_frame_id: Option<&str>,
    event: &RuntimeContextProtocolEvent,
) -> bool {
    let RuntimeContextProtocolEvent::Created(event) = event else {
        return true;
    };
    let Some(execution_context_id) = child_default_execution_context_id(event) else {
        return true;
    };
    if root_frame_id.is_none_or(|root_frame_id| {
        event
            .frame_id
            .as_ref()
            .is_some_and(|frame_id| frame_id.as_str() == root_frame_id)
    }) {
        return true;
    }
    if conn
        .target_devtools_session_state_for_owner(owner)
        .is_some_and(|state| {
            state.has_emitted_child_default_execution_context_id(execution_context_id)
        })
    {
        return false;
    }
    let _ = conn.with_target_devtools_session_state_for_owner_mut(owner, |state| {
        state.mark_child_default_execution_context_id_emitted(execution_context_id)
    });
    true
}

fn child_default_execution_context_id(event: &RuntimeExecutionContextEvent) -> Option<i64> {
    if event.is_default != Some(true) || event.context_type.as_deref() != Some("default") {
        return None;
    }
    event.context_id
}

fn record_child_default_context_delivery(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    event: &RuntimeExecutionContextEvent,
) {
    let Some(execution_context_id) = child_default_execution_context_id(event) else {
        return;
    };
    let Some(root_frame_id) = conn.runtime_session_owner_frame_id(session_id) else {
        return;
    };
    if event
        .frame_id
        .as_ref()
        .is_none_or(|frame_id| frame_id.as_str() == root_frame_id)
    {
        return;
    }
    mark_child_default_context_event_emitted(conn, session_id, execution_context_id);
}

fn record_child_default_context_delivery_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    event: &RuntimeExecutionContextEvent,
) {
    let Some(execution_context_id) = child_default_execution_context_id(event) else {
        return;
    };
    let Some(root_frame_id) = conn.runtime_session_owner_frame_id_for_owner(owner) else {
        return;
    };
    if event
        .frame_id
        .as_ref()
        .is_none_or(|frame_id| frame_id.as_str() == root_frame_id)
    {
        return;
    }
    let _ = conn.with_target_devtools_session_state_for_owner_mut(owner, |state| {
        state.mark_child_default_execution_context_id_emitted(execution_context_id)
    });
}

fn mark_child_default_context_event_emitted(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    execution_context_id: i64,
) {
    conn.with_target_devtools_session_state_for_session_mut(session_id, |state| {
        state.mark_child_default_execution_context_id_emitted(execution_context_id);
    });
}

#[cfg(test)]
mod tests {
    use crate::conn::{BrowserContext, CdpConnection, CommandOwnerScope};
    use crate::conn::{ServiceWorkerTargetState, SharedWorkerTargetState};
    use crate::devtools_runtime::AutomationEvent;
    use moli_core::{RendererOwnerLocalHostId, page::RendererServiceWorkerVersionStatus};
    use moli_shared_worker::SharedWorkerInstanceId;
    use serde_json::json;

    #[test]
    fn runtime_context_event_qualification_uses_target_owner_prefix() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        browser_context.set_active_target_id("TID-1".to_owned());
        conn.browser_context = Some(browser_context);

        let mut created = json!({
            "method": "Runtime.executionContextCreated",
            "params": {
                "context": {
                    "id": 7,
                    "origin": "https://example.test",
                    "name": "",
                    "uniqueId": "4.5",
                    "auxData": {
                        "isDefault": true,
                        "type": "default",
                        "frameId": "TID-1"
                    }
                }
            }
        });
        super::qualify_runtime_context_protocol_event_for_session_owner(&conn, &mut created, None);
        super::qualify_runtime_context_protocol_event_for_session_owner(&conn, &mut created, None);
        assert_eq!(created["params"]["context"]["uniqueId"], json!("TID-1:4.5"));

        let mut destroyed = json!({
            "method": "Runtime.executionContextDestroyed",
            "params": {
                "executionContextId": 7,
                "executionContextUniqueId": "4.5"
            }
        });
        super::qualify_runtime_context_protocol_event_for_session_owner(
            &conn,
            &mut destroyed,
            None,
        );
        assert_eq!(
            destroyed["params"]["executionContextUniqueId"],
            json!("TID-1:4.5")
        );
    }

    #[test]
    fn typed_runtime_context_qualification_tags_shared_worker_target() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-shared".to_owned());
        let mut target = SharedWorkerTargetState::new(
            RendererOwnerLocalHostId::new_for_testing(1),
            SharedWorkerInstanceId::from_u64(9),
            "TID-shared-worker".to_owned(),
            None,
            "https://example.test/shared-worker.js".to_owned(),
            "shared-worker".to_owned(),
        );
        target.attach_session("SID-shared-worker".to_owned());
        browser_context.insert_shared_worker_target(target);
        conn.browser_context = Some(browser_context);

        let mut event = super::RuntimeContextProtocolEvent::Created(
            super::runtime_context_created_event_from_cdp_params(json!({
                "context": {
                    "id": 81_081,
                    "origin": "https://example.test",
                    "name": "shared-worker",
                    "uniqueId": "native-realm",
                    "auxData": {
                        "isDefault": true,
                        "type": "worker"
                    }
                }
            })),
        );

        super::qualify_runtime_context_protocol_event_for_session_owner_typed(
            &conn,
            &mut event,
            Some("SID-shared-worker"),
        );

        let super::RuntimeContextProtocolEvent::Created(event) = event else {
            panic!("expected context-created event");
        };
        assert_eq!(
            event.target_id.as_ref().map(|target_id| target_id.as_str()),
            Some("TID-shared-worker")
        );
        assert_eq!(event.context_type.as_deref(), Some("worker"));
        assert_eq!(
            event.realm_id.as_ref().map(|realm| realm.as_str()),
            Some("TID-shared-worker:native-realm")
        );
    }

    #[test]
    fn typed_runtime_context_qualification_classifies_service_worker_target() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-service".to_owned());
        let mut target = ServiceWorkerTargetState::new(
            41,
            29,
            "TID-service-worker".to_owned(),
            "https://example.test/service-worker.js".to_owned(),
            "https://example.test/".to_owned(),
            RendererServiceWorkerVersionStatus::Activated,
            None,
        );
        target.attach_session("SID-service-worker".to_owned());
        browser_context.insert_service_worker_target(target);
        conn.browser_context = Some(browser_context);

        let mut event = super::RuntimeContextProtocolEvent::Created(
            super::runtime_context_created_event_from_cdp_params(json!({
                "context": {
                    "id": 91_081,
                    "origin": "https://example.test",
                    "name": "https://example.test/service-worker.js",
                    "uniqueId": "native-realm",
                    "auxData": {
                        "isDefault": true,
                        "type": "worker"
                    }
                }
            })),
        );

        super::qualify_runtime_context_protocol_event_for_session_owner_typed(
            &conn,
            &mut event,
            Some("SID-service-worker"),
        );

        let super::RuntimeContextProtocolEvent::Created(event) = event else {
            panic!("expected context-created event");
        };
        assert_eq!(
            event.target_id.as_ref().map(|target_id| target_id.as_str()),
            Some("TID-service-worker")
        );
        assert_eq!(event.context_type.as_deref(), Some("service-worker"));
        assert_eq!(event.frame_id, None);
        assert_eq!(
            event.realm_id.as_ref().map(|realm| realm.as_str()),
            Some("TID-service-worker:native-realm")
        );
    }

    #[test]
    fn runtime_context_adapter_serializes_context_created() {
        let params = json!({
            "context": {
                "id": 7,
                "origin": "https://example.test",
                "name": "",
                "uniqueId": "realm-7",
                "auxData": {
                    "isDefault": true,
                    "type": "default",
                    "frameId": "FRAME-1",
                    "grantUniversalAccess": true
                }
            }
        });
        let mut out = Vec::new();

        super::emit_cdp_runtime_context_background_automation_event(
            &mut out,
            AutomationEvent::RuntimeExecutionContextCreated(
                super::runtime_context_created_event_from_cdp_params(params),
            ),
            Some("SID-1"),
        );

        assert_eq!(out.len(), 1);
        assert!(out[0].protocol_message().is_none());
        assert!(out[0].has_protocol_wire_message());
        let (message, automation_event) = out.pop().expect("event").into_parts();
        let Some(AutomationEvent::RuntimeExecutionContextCreated(event)) = automation_event else {
            panic!("expected RuntimeExecutionContextCreated sidecar");
        };
        assert_eq!(event.grant_universal_access, Some(true));
        assert_eq!(message["sessionId"], json!("SID-1"));
        assert_eq!(message["method"], json!("Runtime.executionContextCreated"));
        assert_eq!(message["params"]["context"]["id"], json!(7));
        assert_eq!(message["params"]["context"]["uniqueId"], json!("realm-7"));
        assert_eq!(
            message["params"]["context"]["auxData"]["frameId"],
            json!("FRAME-1")
        );
        assert_eq!(
            message["params"]["context"]["auxData"]["grantUniversalAccess"],
            json!(true)
        );
    }

    #[test]
    fn live_child_context_delivery_advances_runtime_enable_inventory_cursor() {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        browser_context.set_active_target_id("TID-1".to_owned());
        browser_context.attach_active_session("SID-1".to_owned());
        conn.browser_context = Some(browser_context);
        let event = super::RuntimeContextProtocolEvent::Created(
            super::runtime_context_created_event_from_cdp_params(json!({
                "context": {
                    "id": 7,
                    "origin": "https://example.test",
                    "name": "",
                    "uniqueId": "realm-7",
                    "auxData": {
                        "isDefault": true,
                        "type": "default",
                        "frameId": "FRAME-1"
                    }
                }
            })),
        );

        let super::RuntimeContextProtocolEvent::Created(created) = &event else {
            unreachable!();
        };
        super::record_child_default_context_delivery(&mut conn, Some("SID-1"), created);

        assert!(
            !super::should_emit_child_default_context_inventory_replay_once_for_owner(
                &mut conn,
                &CommandOwnerScope::for_session("SID-1"),
                Some("TID-1"),
                &event,
            ),
            "Runtime.enable inventory must not replay a child context already delivered by V8"
        );
        assert!(
            super::should_emit_child_default_context_inventory_replay_once_for_owner(
                &mut conn,
                &CommandOwnerScope::for_session("SID-other"),
                Some("TID-1"),
                &event,
            ),
            "the inventory replay cursor remains local to each frontend session"
        );
    }

    #[test]
    fn runtime_context_adapter_serializes_context_destroyed() {
        let params = json!({
            "executionContextId": 7,
            "executionContextUniqueId": "realm-7",
        });
        let mut out = Vec::new();

        super::emit_cdp_runtime_context_background_automation_event(
            &mut out,
            AutomationEvent::RuntimeExecutionContextDestroyed(
                super::runtime_context_destroyed_event_from_cdp_params(params),
            ),
            Some("SID-1"),
        );

        assert_eq!(out.len(), 1);
        assert!(out[0].protocol_message().is_none());
        assert!(out[0].has_protocol_wire_message());
        let (message, automation_event) = out.pop().expect("event").into_parts();
        let Some(AutomationEvent::RuntimeExecutionContextDestroyed(_event)) = automation_event
        else {
            panic!("expected RuntimeExecutionContextDestroyed sidecar");
        };
        assert_eq!(message["sessionId"], json!("SID-1"));
        assert_eq!(
            message["method"],
            json!("Runtime.executionContextDestroyed")
        );
        assert_eq!(message["params"]["executionContextId"], json!(7));
        assert_eq!(
            message["params"]["executionContextUniqueId"],
            json!("realm-7")
        );
    }

    #[test]
    fn runtime_context_adapter_serializes_contexts_cleared() {
        let mut out = Vec::new();

        super::emit_cdp_runtime_context_background_automation_event(
            &mut out,
            AutomationEvent::RuntimeExecutionContextsCleared(
                super::runtime_contexts_cleared_event_from_cdp_params(json!({})),
            ),
            Some("SID-1"),
        );

        assert_eq!(out.len(), 1);
        assert!(out[0].protocol_message().is_none());
        assert!(out[0].has_protocol_wire_message());
        let (message, automation_event) = out.pop().expect("event").into_parts();
        let Some(AutomationEvent::RuntimeExecutionContextsCleared(_event)) = automation_event
        else {
            panic!("expected RuntimeExecutionContextsCleared sidecar");
        };
        assert_eq!(message["sessionId"], json!("SID-1"));
        assert_eq!(message["method"], json!("Runtime.executionContextsCleared"));
        assert_eq!(message["params"], json!({}));
    }

    #[test]
    fn runtime_context_destroyed_clears_matching_remote_object_realm() {
        let mut conn = CdpConnection::new();
        conn.browser_context = Some(BrowserContext::new_with_page_for_test(
            "BID-runtime-context",
            "TID-runtime-context",
        ));
        conn.register_runtime_remote_object_ids_for_session_owner_with_realm(
            None,
            vec!["object-realm-1".to_owned()],
            "realm-1",
        );
        conn.register_runtime_remote_object_alias_for_session_owner_with_realm(
            None,
            "alias-realm-1".to_owned(),
            "object-realm-1".to_owned(),
            "realm-1",
        );
        conn.register_runtime_remote_object_ids_for_session_owner_with_realm(
            None,
            vec!["object-realm-2".to_owned()],
            "realm-2",
        );

        super::apply_runtime_context_protocol_event_side_effects(
            &mut conn,
            &json!({
                "method": "Runtime.executionContextDestroyed",
                "params": {
                    "executionContextId": 7,
                    "executionContextUniqueId": "realm-1"
                }
            }),
            None,
        );

        assert!(!conn.runtime_remote_object_id_known_for_session_owner(None, "object-realm-1"));
        assert!(!conn.runtime_remote_object_id_known_for_session_owner(None, "alias-realm-1"));
        assert!(conn.runtime_remote_object_id_known_for_session_owner(None, "object-realm-2"));
    }

    #[test]
    fn runtime_contexts_cleared_clears_shared_worker_remote_object_tracking() {
        let mut conn = CdpConnection::new();
        let mut browser_context = BrowserContext::new("BID-shared-runtime-context".to_owned());
        let mut target = SharedWorkerTargetState::new(
            RendererOwnerLocalHostId::new_for_testing(7),
            SharedWorkerInstanceId::from_u64(11),
            "TID-shared-worker".to_owned(),
            None,
            "https://example.test/worker.js".to_owned(),
            "worker".to_owned(),
        );
        target.attach_session("SID-shared-worker".to_owned());
        browser_context.insert_shared_worker_target(target);
        conn.browser_context = Some(browser_context);
        conn.register_runtime_remote_object_ids_for_session_owner_with_realm(
            Some("SID-shared-worker"),
            vec!["worker-object".to_owned()],
            "worker-realm",
        );
        assert!(conn.runtime_remote_object_id_known_for_session_owner(
            Some("SID-shared-worker"),
            "worker-object"
        ));

        super::apply_runtime_context_protocol_event_side_effects(
            &mut conn,
            &json!({
                "method": "Runtime.executionContextsCleared",
                "params": {}
            }),
            Some("SID-shared-worker"),
        );

        assert!(!conn.runtime_remote_object_id_known_for_session_owner(
            Some("SID-shared-worker"),
            "worker-object"
        ));
    }
}
