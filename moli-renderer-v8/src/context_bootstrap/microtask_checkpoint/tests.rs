use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::Duration;

#[test]
fn callback_cleanup_keeps_fifo_admission_until_task_completion() {
    crate::ensure_v8_for_test();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let queues = Arc::new(moli_indexeddb::ConnectionRequestQueues::default());
    let first = queues.enqueue("storage", "database", Arc::new(|| {}));
    let first_handle = first.handle().clone();
    let wakes = Arc::new(AtomicUsize::new(0));
    let next_wakes = Arc::clone(&wakes);
    let next = queues.enqueue(
        "storage",
        "database",
        Arc::new(move || {
            next_wakes.fetch_add(1, Ordering::SeqCst);
        }),
    );
    defer_indexed_db_connection_request(scope, first);
    for _ in 0..2 {
        crate::script_vm::perform_callback_microtask_checkpoint_and_report_pending_promise_rejections(scope);
        assert!(first_handle.is_active());
        assert!(first_handle.is_head());
        assert!(!next.handle().is_head());
        assert_eq!(wakes.load(Ordering::SeqCst), 0);
    }
    crate::script_vm::perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    assert!(!first_handle.is_active());
    assert!(next.handle().is_head());
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
}

#[test]
fn callback_cleanup_keeps_versionchange_acknowledgement_until_task_completion() {
    crate::ensure_v8_for_test();
    let mut service = moli_indexeddb::sqlite::service::Service::new(Default::default()).unwrap();
    let service_client = service.client();
    let (sender, receiver) = mpsc::channel();
    let client = service_client
        .resolve(
            b"checkpoint-storage".to_vec(),
            "database".into(),
            move |reply| {
                sender.send(reply).unwrap();
            },
        )
        .unwrap();
    receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .result
        .unwrap();
    let (sender, receiver) = mpsc::channel();
    client
        .open_connection(Arc::new(|| {}), move |reply| {
            sender.send(reply).unwrap();
        })
        .unwrap();
    let moli_indexeddb::sqlite::service::CommandResult::Connection(connection) = receiver
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .result
        .unwrap()
    else {
        panic!("expected a native SQL connection");
    };
    let batch = client.begin_version_change(1, Some(2), Arc::new(|| {}));
    let notification = connection.take_notification().unwrap();
    assert!(batch.blocked_event_required().is_none());

    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    defer_indexed_db_version_change_completion(scope, notification.completion);
    for _ in 0..2 {
        crate::script_vm::perform_callback_microtask_checkpoint_and_report_pending_promise_rejections(scope);
        assert!(batch.blocked_event_required().is_none());
    }
    crate::script_vm::perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
    assert_eq!(batch.blocked_event_required(), Some(true));
    drop(connection);
    service.shutdown().unwrap();
}
