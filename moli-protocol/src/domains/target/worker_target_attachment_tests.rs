//! Exact SharedWorker attachment tests for the shared worker-target coordinator.

use moli_core::{
    RendererOwnerLocalHostId,
    page::{
        RendererRuntimeInspectorMessage, RendererSharedWorkerConsoleMessage,
        RendererSharedWorkerTargetInfo,
    },
};
use moli_shared_worker::SharedWorkerInstanceId;
use serde_json::{Value, json};

use super::*;
use crate::{
    conn::{BrowserContext, CdpTargetFilter, CommandDispatchContext, CommandOwnerScope},
    devtools_runtime::AutomationEvent,
    domains::activity::{ProtocolOutputPayloads, ProtocolOutputProjectionContext},
};

const BROWSER_CONTEXT_ID: &str = "BID-shared-worker-attachment";
const TARGET_ID: &str = "TID-shared-worker-collision";
const SESSION_ID: &str = "SID-shared-worker-collision";
const INSTANCE_ID: u64 = 701;

fn renderer_info(instance_id: u64) -> RendererSharedWorkerTargetInfo {
    RendererSharedWorkerTargetInfo {
        owner_local_host_id: RendererOwnerLocalHostId::new_for_testing(17),
        instance_id: SharedWorkerInstanceId::from_u64(instance_id),
        url: "https://worker.test/shared.js".to_owned(),
        name: "exact-worker".to_owned(),
    }
}

fn install_collision_target(conn: &mut CdpConnection) {
    let mut target = SharedWorkerTargetState::new(
        RendererOwnerLocalHostId::new_for_testing(17),
        SharedWorkerInstanceId::from_u64(INSTANCE_ID),
        TARGET_ID.to_owned(),
        Some("TID-owner".to_owned()),
        "https://worker.test/shared.js".to_owned(),
        "exact-worker".to_owned(),
    );
    target.attach_session(SESSION_ID.to_owned());
    conn.browser_context
        .as_mut()
        .expect("test browser context")
        .insert_shared_worker_target(target);
    conn.register_session_route_for_test(
        SESSION_ID,
        crate::conn::CdpSessionRoute::SharedWorkerTarget {
            browser_context_id: BROWSER_CONTEXT_ID.to_owned(),
            target_id: TARGET_ID.to_owned(),
        },
    );
}

fn runtime_inspector_messages(value: &str) -> Vec<RendererRuntimeInspectorMessage> {
    vec![RendererRuntimeInspectorMessage::from_v8_inspector_message(
        json!({
            "method": "Runtime.consoleAPICalled",
            "params": {
                "type": "log",
                "args": [{ "type": "string", "value": value }],
                "executionContextId": 17
            }
        }),
    )]
}

async fn drain(
    conn: &mut CdpConnection,
    outputs: TargetPreparedOutputs,
) -> Vec<crate::conn::BackgroundProtocolEvent> {
    let mut prepared =
        ProtocolOutputPayloads::from_slot(TargetPreparedOutputSlot::from_outputs(outputs));
    let owner = CommandOwnerScope::capture(conn, None);
    let mut command = CommandDispatchContext::default();
    emit_target_lifecycle_events(
        conn,
        &mut ProtocolOutputProjectionContext::new(&owner, &mut command),
        Some(&mut prepared),
    )
    .await;
    command.take_protocol_events()
}

fn protocol_messages(events: Vec<crate::conn::BackgroundProtocolEvent>) -> Vec<Value> {
    events
        .into_iter()
        .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
        .collect()
}

#[tokio::test]
async fn short_lived_shared_worker_preserves_exact_lifecycle_order() {
    let mut conn = CdpConnection::default();
    conn.browser_context = Some(BrowserContext::new(BROWSER_CONTEXT_ID.to_owned()));
    conn.set_target_discovery_for_owner(None, CdpTargetFilter::default_target_discovery());
    conn.set_auto_attach_owner(None, true, true, CdpTargetFilter::default_auto_attach());

    let mut outputs = register_shared_worker_target(
        &mut conn,
        BROWSER_CONTEXT_ID,
        Some("TID-owner".to_owned()),
        renderer_info(702),
    );
    let (target_id, session_id, attachment) = outputs
        .worker_target_lifecycle_outputs
        .iter()
        .find_map(|output| match output {
            WorkerTargetLifecycleOutput::SharedWorkerAttached { attachment, .. } => Some((
                attachment.target_id().to_owned(),
                attachment.session_id().to_owned(),
                attachment.clone(),
            )),
            _ => None,
        })
        .expect("short-lived worker should prepare an exact attachment");
    outputs.extend(remove_shared_worker_target(
        &mut conn,
        BROWSER_CONTEXT_ID,
        SharedWorkerInstanceId::from_u64(702),
    ));

    assert!(
        attachment.is_current(),
        "the ordered detach output must keep earlier accepted output alive"
    );
    let events = drain(&mut conn, outputs).await;
    let lifecycle = events
        .into_iter()
        .filter_map(|event| {
            let (_, sidecar) = event.into_parts();
            match sidecar {
                Some(AutomationEvent::TargetCreated(event)) => {
                    Some(("created", event.target_id.as_str().to_owned(), None))
                }
                Some(AutomationEvent::TargetAttached(event)) => Some((
                    "attached",
                    event.target_id.as_str().to_owned(),
                    Some(event.session_id.as_str().to_owned()),
                )),
                Some(AutomationEvent::TargetDetached(event)) => Some((
                    "detached",
                    event.target_id.as_str().to_owned(),
                    Some(event.session_id.as_str().to_owned()),
                )),
                Some(AutomationEvent::TargetDestroyed(event)) => {
                    Some(("destroyed", event.target_id.as_str().to_owned(), None))
                }
                _ => None,
            }
        })
        .collect::<Vec<_>>();

    assert_eq!(
        lifecycle,
        vec![
            ("created", target_id.clone(), None),
            ("attached", target_id.clone(), Some(session_id.clone())),
            ("detached", target_id.clone(), Some(session_id)),
            ("destroyed", target_id, None),
        ]
    );
    assert!(
        !attachment.is_current(),
        "consuming the exact detach must retire all held attachment observers"
    );
}

#[tokio::test]
async fn detached_session_cannot_be_resurrected_by_held_auto_attach_output() {
    let mut conn = CdpConnection::default();
    conn.set_auto_attach_owner(
        None,
        true,
        false,
        crate::conn::CdpTargetFilter::default_auto_attach(),
    );
    conn.browser_context = Some(BrowserContext::new(BROWSER_CONTEXT_ID.to_owned()));

    let outputs =
        register_shared_worker_target(&mut conn, BROWSER_CONTEXT_ID, None, renderer_info(703));
    let attachment = outputs
        .worker_target_lifecycle_outputs
        .iter()
        .find_map(|output| match output {
            WorkerTargetLifecycleOutput::SharedWorkerAttached { attachment, .. } => {
                Some(attachment.clone())
            }
            _ => None,
        })
        .expect("worker should prepare auto-attach output");
    assert_eq!(
        conn.browser_context
            .as_mut()
            .expect("test browser context")
            .detach_shared_worker_target_session(attachment.session_id())
            .as_deref(),
        Some(attachment.target_id()),
    );
    assert!(!attachment.is_current());

    let events = protocol_messages(drain(&mut conn, outputs).await);

    assert!(
        events
            .iter()
            .all(|event| event["method"] != json!("Target.attachedToTarget")),
        "held auto-attach output must not recreate a detached attachment: {events:?}"
    );
    assert!(
        conn.attached_sessions_for_target(attachment.target_id())
            .is_empty()
    );
}

#[tokio::test]
async fn exact_scope_rejects_old_console_batch_after_complete_raw_identity_reuse() {
    let mut conn = CdpConnection::default();
    conn.browser_context = Some(BrowserContext::new(BROWSER_CONTEXT_ID.to_owned()));
    install_collision_target(&mut conn);
    conn.shared_worker_target_for_session_mut(Some(SESSION_ID))
        .expect("old worker target")
        .set_console_enabled(SESSION_ID, true);
    let mut outputs = record_shared_worker_target_console_message(
        &mut conn,
        BROWSER_CONTEXT_ID,
        SharedWorkerInstanceId::from_u64(INSTANCE_ID),
        RendererSharedWorkerConsoleMessage {
            message: "old attachment".to_owned(),
            args: Vec::new(),
            stack: None,
        },
    );

    drop(
        conn.browser_context
            .as_mut()
            .expect("test browser context")
            .remove_shared_worker_target_by_renderer_instance(SharedWorkerInstanceId::from_u64(
                INSTANCE_ID,
            )),
    );
    install_collision_target(&mut conn);
    conn.shared_worker_target_for_session_mut(Some(SESSION_ID))
        .expect("replacement worker target")
        .set_console_enabled(SESSION_ID, true);
    outputs.extend(record_shared_worker_target_console_message(
        &mut conn,
        BROWSER_CONTEXT_ID,
        SharedWorkerInstanceId::from_u64(INSTANCE_ID),
        RendererSharedWorkerConsoleMessage {
            message: "new attachment".to_owned(),
            args: Vec::new(),
            stack: None,
        },
    ));

    let messages = protocol_messages(drain(&mut conn, outputs).await);
    let console_texts = messages
        .iter()
        .filter(|message| message["method"] == json!("Console.messageAdded"))
        .map(|message| message["params"]["message"]["text"].clone())
        .collect::<Vec<_>>();

    assert_eq!(console_texts, vec![json!("new attachment")]);
    assert!(
        conn.shared_worker_target_for_session(Some(SESSION_ID))
            .expect("replacement worker target")
            .pending_console_domain_messages(SESSION_ID)
            .is_empty(),
        "the replacement attachment must advance only its own console cursor"
    );
}

#[tokio::test]
async fn exact_scope_independently_authorizes_old_and_new_inspector_batches_in_one_slot() {
    let mut conn = CdpConnection::default();
    conn.browser_context = Some(BrowserContext::new(BROWSER_CONTEXT_ID.to_owned()));
    install_collision_target(&mut conn);
    let mut outputs = record_shared_worker_target_runtime_inspector_messages(
        &mut conn,
        BROWSER_CONTEXT_ID,
        SharedWorkerInstanceId::from_u64(INSTANCE_ID),
        Some(SESSION_ID.to_owned()),
        runtime_inspector_messages("old attachment"),
    );

    drop(
        conn.browser_context
            .as_mut()
            .expect("test browser context")
            .remove_shared_worker_target_by_renderer_instance(SharedWorkerInstanceId::from_u64(
                INSTANCE_ID,
            )),
    );
    install_collision_target(&mut conn);
    outputs.extend(record_shared_worker_target_runtime_inspector_messages(
        &mut conn,
        BROWSER_CONTEXT_ID,
        SharedWorkerInstanceId::from_u64(INSTANCE_ID),
        Some(SESSION_ID.to_owned()),
        runtime_inspector_messages("new attachment"),
    ));

    let messages = protocol_messages(drain(&mut conn, outputs).await);
    let values = messages
        .iter()
        .filter(|message| message["method"] == json!("Runtime.consoleAPICalled"))
        .map(|message| message["params"]["args"][0]["value"].clone())
        .collect::<Vec<_>>();

    assert_eq!(values, vec![json!("new attachment")]);
}
