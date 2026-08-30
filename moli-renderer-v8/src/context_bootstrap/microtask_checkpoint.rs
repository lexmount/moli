#[derive(Default)]
// One queue per V8 isolate/Agent. Tasks may retain exact realm objects, but the
// queue itself must not be hidden in any individual realm's runtime table.
pub(crate) struct AgentMicrotaskCheckpointTasks {
    tasks: Vec<AgentMicrotaskCheckpointTask>,
}

enum AgentMicrotaskCheckpointTask {
    DeactivateIndexedDbTransaction {
        context: v8::Global<v8::Context>,
        transaction: v8::Global<v8::Object>,
    },
    RestoreWindowEvent {
        host_ptr: *mut crate::native_bridge::JsContextHost,
        relevant_identity: Option<crate::native_bridge::WindowExecutionContextIdentity>,
        context: v8::Global<v8::Context>,
        previous: v8::Global<v8::Value>,
    },
}

pub(crate) fn enqueue_window_event_restore_after_microtask_checkpoint(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
    relevant_identity: Option<crate::native_bridge::WindowExecutionContextIdentity>,
    context: v8::Local<'_, v8::Context>,
    previous: v8::Local<'_, v8::Value>,
) {
    if scope.get_slot::<AgentMicrotaskCheckpointTasks>().is_none() {
        assert!(
            scope.set_slot(AgentMicrotaskCheckpointTasks::default()),
            "agent checkpoint state should be installed once on first use"
        );
    }
    let task = AgentMicrotaskCheckpointTask::RestoreWindowEvent {
        host_ptr,
        relevant_identity,
        context: v8::Global::new(scope, context),
        previous: v8::Global::new(scope, previous),
    };
    scope
        .get_slot_mut::<AgentMicrotaskCheckpointTasks>()
        .expect("agent checkpoint state should exist after installation")
        .tasks
        .push(task);
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
    if scope.get_slot::<AgentMicrotaskCheckpointTasks>().is_none() {
        assert!(
            scope.set_slot(AgentMicrotaskCheckpointTasks::default()),
            "agent checkpoint state should be installed once on first use"
        );
    }
    let context = scope.get_current_context();
    let task = AgentMicrotaskCheckpointTask::DeactivateIndexedDbTransaction {
        context: v8::Global::new(scope, context),
        transaction: v8::Global::new(scope, transaction),
    };
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

    let mut window_event_restores = Vec::new();
    for task in tasks {
        match task {
            AgentMicrotaskCheckpointTask::DeactivateIndexedDbTransaction {
                context,
                transaction,
            } => run_indexed_db_transaction_deactivation(scope, context, transaction),
            AgentMicrotaskCheckpointTask::RestoreWindowEvent {
                host_ptr,
                relevant_identity,
                context,
                previous,
            } => {
                window_event_restores.push((host_ptr, relevant_identity, context, previous));
            }
        }
    }
    for (host_ptr, relevant_identity, context, previous) in window_event_restores.into_iter().rev()
    {
        restore_window_event(scope, host_ptr, relevant_identity, context, previous);
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

fn restore_window_event(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
    relevant_identity: Option<crate::native_bridge::WindowExecutionContextIdentity>,
    context: v8::Global<v8::Context>,
    previous: v8::Global<v8::Value>,
) {
    if relevant_identity.is_some_and(|identity| {
        !unsafe { &*host_ptr }.window_execution_context_identity_is_current(identity)
    }) {
        return;
    }
    let context = v8::Local::new(scope, &context);
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);
    let previous = v8::Local::new(scope, &previous);
    let _ = global.set(
        scope,
        crate::util::v8str(scope, crate::host::WINDOW_EVENT_SLOT).into(),
        previous,
    );
}
