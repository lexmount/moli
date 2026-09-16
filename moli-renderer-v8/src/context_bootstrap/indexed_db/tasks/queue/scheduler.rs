use super::*;

pub(in crate::context_bootstrap::indexed_db) fn enqueue_indexed_db_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    task: v8::Local<'s, v8::Object>,
) {
    let task_id = indexed_db_typed_task_id(scope, task)
        .expect("IndexedDB runtime task must retain its exact typed task id before enqueue");
    push_object_to_indexed_db_runtime_array(scope, IndexedDbRuntimeArray::TaskQueue, task);
    if let Some(host_ptr) = crate::util::context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &*host_ptr };
        let execution_context = indexed_db_typed_task_execution_context(scope, task).expect(
            "Page IndexedDB tasks must retain the exact accepting Window realm before enqueue",
        );
        if host
            .page_indexed_db_task_sender()
            .send(
                execution_context,
                crate::page_task_queue::RendererPageIndexedDbTaskKind::RuntimeQueue(task_id),
            )
            .is_err()
        {
            // The stable Page consumer has retired. Do not leave a V8 task
            // that no scheduler can ever select, and never fall back to the
            // legacy Page queue after typed-route retirement.
            let _ = discard_indexed_db_task_by_id(scope, task_id);
        }
        return;
    }
    if signal_worker_indexed_db_task_wake(scope, task_id) {
        return;
    }
    let Some(callback) = v8::Function::builder(flush_indexed_db_task_callback).build(scope) else {
        return;
    };
    enqueue_host_microtask(scope, callback);
}
