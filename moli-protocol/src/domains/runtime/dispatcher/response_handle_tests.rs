use super::*;
use crate::conn::BrowserContext;
use moli_inspector_json::InspectorJson;

#[derive(Clone, Copy, Debug)]
enum Owner {
    Page,
    PageSession,
    SharedWorker,
}

impl Owner {
    fn fixture(self) -> (CdpConnection, CommandOwnerScope) {
        if matches!(self, Self::SharedWorker) {
            return (
                worker_workflow_fixture(),
                CommandOwnerScope::for_session("SID-owned-worker"),
            );
        }
        let mut conn = CdpConnection::default();
        let mut context = BrowserContext::new("BID-response-dispatch".to_owned());
        context.set_active_target_id("TID-response-dispatch".to_owned());
        context.attach_active_session("SID-response-dispatch".to_owned());
        conn.install_browser_context_fixture_for_test(context);
        let session = matches!(self, Self::PageSession).then_some("SID-response-dispatch");
        let owner = CommandOwnerScope::capture(&conn, session);
        (conn, owner)
    }
}

#[derive(Clone, Copy, Debug)]
enum Container {
    Raw,
    Parsed,
    Typed,
}

#[test]
fn routed_output_response_registration_preserves_groups_without_materializing() {
    for owner_kind in [Owner::Page, Owner::PageSession, Owner::SharedWorker] {
        for container in [Container::Raw, Container::Parsed, Container::Typed] {
            for real_handles in [false, true] {
                let (mut conn, owner) = owner_kind.fixture();
                conn.register_runtime_remote_object_ids_from_value_for_owner_with_group(
                    &owner,
                    &json!({"objectId":"existing-handle"}),
                    "group-A",
                );
                let mut body = json!({"result":{"type":"object","value":{
                    "objectId":"existing-handle","nested":{"errorObjectId":"user-only"}
                },"deepSerializedValue":{"type":"array","value":[{"objectId":"deep-only"}]}},
                    "futureExtension":{"objectId":"unknown-only"}
                });
                if real_handles {
                    body["result"]["objectId"] = json!("actual");
                    body["exceptionDetails"] = json!({"exception":{
                        "type":"object","objectId":"exception","value":{"objectId":"user-only"}
                    }});
                }
                let envelope = json!({"id":45,"result":body});
                let raw = InspectorJson::from_slice(envelope.to_string().as_bytes()).unwrap();
                let wire = raw.to_string();
                let event = match container {
                    Container::Raw => BackgroundProtocolEvent::inspector_message(raw.clone(), None),
                    Container::Parsed => BackgroundProtocolEvent::inspector_message(envelope, None),
                    Container::Typed => {
                        BackgroundProtocolEvent::command_success(Some(45), owner.session_id(), body)
                    }
                };
                let routed = RuntimeInspectorRoutedOutput {
                    events: vec![event],
                    ..Default::default()
                };
                routed.register_object_group_for_success(&mut conn, &owner, None);
                assert!(!raw.is_materialized());
                routed.register_object_group_for_success(&mut conn, &owner, Some("group-B"));
                assert_eq!(
                    conn.runtime_remote_object_group_for_owner(&owner, "existing-handle")
                        .as_deref(),
                    Some("group-A"),
                    "{owner_kind:?}, {container:?}, real_handles={real_handles}"
                );
                for id in ["user-only", "deep-only", "unknown-only"] {
                    assert!(!conn.runtime_remote_object_id_known_for_owner(&owner, id));
                }
                assert!(!raw.is_materialized());
                assert_eq!(raw.to_string(), wire);
                for id in ["actual", "exception"] {
                    assert_eq!(
                        conn.runtime_remote_object_id_known_for_owner(&owner, id),
                        real_handles
                    );
                    assert_eq!(
                        conn.runtime_remote_object_group_for_owner(&owner, id)
                            .as_deref(),
                        real_handles.then_some("group-B")
                    );
                }
                conn.unregister_runtime_remote_object_group_for_owner(&owner, "group-B");
                assert!(conn.runtime_remote_object_id_known_for_owner(&owner, "existing-handle"));
                assert_eq!(
                    conn.runtime_remote_object_group_for_owner(&owner, "existing-handle")
                        .as_deref(),
                    Some("group-A")
                );
                for id in ["actual", "exception"] {
                    assert!(!conn.runtime_remote_object_id_known_for_owner(&owner, id));
                }
            }
        }
    }
}

#[test]
fn routed_output_registration_retains_notification_handle_semantics() {
    let notification = json!({"method":"Runtime.futureNotification","params":{
        "value":{"objectId":"notification"},"nested":[{"promiseObjectId":"promise"}]
    }});
    for message in [
        InspectorJson::from_slice(notification.to_string().as_bytes()).unwrap(),
        InspectorJson::from(notification),
    ] {
        let (mut conn, owner) = Owner::PageSession.fixture();
        let routed = RuntimeInspectorRoutedOutput {
            events: vec![BackgroundProtocolEvent::inspector_message(message, None)],
            ..Default::default()
        };
        routed.register_object_group_for_success(&mut conn, &owner, Some("notifications"));
        for id in ["notification", "promise"] {
            assert_eq!(
                conn.runtime_remote_object_group_for_owner(&owner, id)
                    .as_deref(),
                Some("notifications")
            );
        }
    }
}
