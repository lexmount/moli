#[derive(Default)]
// One queue per V8 isolate/Agent. Tasks may retain exact realm objects, but the
// queue itself must not be hidden in any individual realm's runtime table.
pub(crate) struct AgentMicrotaskCheckpointTasks {
    tasks: Vec<AgentMicrotaskCheckpointTask>,
}

enum AgentMicrotaskCheckpointTask {
    IndexedDbVersionChange(moli_indexeddb::sqlite::service::VersionChangeAcknowledgement),
    IndexedDbConnectionRequest(moli_indexeddb::ConnectionRequestLease),
    DeactivateIndexedDbTransaction {
        context: v8::Global<v8::Context>,
        transaction: v8::Global<v8::Object>,
    },
}

pub(crate) fn install_agent_microtask_checkpoint_tasks(isolate: &mut v8::Isolate) {
    assert!(
        isolate.set_slot(AgentMicrotaskCheckpointTasks::default()),
        "agent microtask checkpoint state should be installed exactly once"
    );
}

pub(in crate::context_bootstrap) fn enqueue_indexed_db_transaction_deactivation(
    scope: &mut v8::PinScope<'_, '_>,
    transaction: v8::Local<'_, v8::Object>,
) {
    let context = scope.get_current_context();
    let task = AgentMicrotaskCheckpointTask::DeactivateIndexedDbTransaction {
        context: v8::Global::new(scope, context),
        transaction: v8::Global::new(scope, transaction),
    };
    enqueue_checkpoint_task(scope, task);
}

pub(in crate::context_bootstrap) fn defer_indexed_db_version_change_completion(
    scope: &mut v8::PinScope<'_, '_>,
    completion: moli_indexeddb::sqlite::service::VersionChangeAcknowledgement,
) {
    enqueue_checkpoint_task(
        scope,
        AgentMicrotaskCheckpointTask::IndexedDbVersionChange(completion),
    );
}

pub(in crate::context_bootstrap) fn defer_indexed_db_connection_request(
    scope: &mut v8::PinScope<'_, '_>,
    request: moli_indexeddb::ConnectionRequestLease,
) {
    enqueue_checkpoint_task(
        scope,
        AgentMicrotaskCheckpointTask::IndexedDbConnectionRequest(request),
    );
}

fn enqueue_checkpoint_task(scope: &mut v8::PinScope<'_, '_>, task: AgentMicrotaskCheckpointTask) {
    if scope.get_slot::<AgentMicrotaskCheckpointTasks>().is_none() {
        assert!(
            scope.set_slot(AgentMicrotaskCheckpointTasks::default()),
            "agent checkpoint state should be installed once on first use"
        );
    }
    scope
        .get_slot_mut::<AgentMicrotaskCheckpointTasks>()
        .expect("agent checkpoint state should exist after installation")
        .tasks
        .push(task);
}

pub(crate) fn run_end_of_microtask_checkpoint_tasks(scope: &mut v8::PinScope<'_, '_>) {
    // Tasks enqueued while this batch runs belong to the next checkpoint.
    let Some(tasks) = scope
        .get_slot_mut::<AgentMicrotaskCheckpointTasks>()
        .map(|state| std::mem::take(&mut state.tasks))
    else {
        return;
    };

    for task in tasks {
        match task {
            AgentMicrotaskCheckpointTask::IndexedDbVersionChange(completion) => drop(completion),
            AgentMicrotaskCheckpointTask::IndexedDbConnectionRequest(request) => drop(request),
            AgentMicrotaskCheckpointTask::DeactivateIndexedDbTransaction {
                context,
                transaction,
            } => run_indexed_db_transaction_deactivation(scope, context, transaction),
        }
    }
}

fn run_indexed_db_transaction_deactivation(
    scope: &mut v8::PinScope<'_, '_>,
    context: v8::Global<v8::Context>,
    transaction: v8::Global<v8::Object>,
) {
    let context = v8::Local::new(scope, &context);
    let scope = &mut v8::ContextScope::new(scope, context);
    let transaction = v8::Local::new(scope, &transaction);
    crate::context_bootstrap::indexed_db::deactivate_indexed_db_transaction_after_microtask_checkpoint(
        scope,
        transaction,
    );
}
