use super::*;

#[tokio::test]
async fn service_worker_notification_options_reject_original_conversion_errors_before_dispatch() {
    ensure_v8();
    let source = format!(
        "const verify = {}; self.addEventListener('message', event => event.waitUntil(verify(self.registration).then(value => {{if (value !== true) throw Error('notification fixture failed');}})));",
        include_str!("../../../../tests/fixtures/notification-persistent-dictionary.js")
    );
    let mut handle = spawn_service_worker_for_test(&source);
    let event_id = ServiceWorkerEventId::from_u64_for_worker(1);
    handle.dispatch_service_worker_message_event(ServiceWorkerMessageEvent {
        event_id,
        owner: crate::service_worker_runtime::ServiceWorkerRunOwner::new(
            ServiceWorkerVersionId::from_u64_for_test(1),
            crate::runtime::RendererServiceWorkerRunIdentity::fresh(),
        ),
        source_client_id: None,
        source_client_url: None,
        source_client_snapshot: None,
        source_worker: None,
        source_origin: String::new(),
        payload: serialize_test_string("verify"),
        window_interaction_allowed: false,
    });
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    loop {
        let message = tokio::time::timeout_at(deadline, handle.recv())
            .await
            .expect("notification conversion must settle without dispatch")
            .expect("service worker must report its result");
        match message {
            WorkerToParentMessage::ServiceWorkerMessageCompleted(completion) => {
                assert_eq!(completion.event_id, event_id);
                assert_eq!(completion.result, Ok(()));
                break;
            }
            WorkerToParentMessage::RuntimeInspectorMessages(_) => {}
            other => panic!("conversion must fail before notification dispatch: {other:?}"),
        }
    }
    handle.terminate_and_join();
}
