//! File-reading requests stay in native state so queue operations cannot call
//! author code through Array.prototype. The worker loop drains these tasks
//! before waiting, with a microtask checkpoint after each request phase.

use std::sync::atomic::Ordering;

use crate::context_bootstrap::FileReadingTask;

pub(crate) fn queue_worker_file_reading_task(
    scope: &mut v8::PinScope<'_, '_>,
    task: FileReadingTask,
) -> bool {
    let Some(state) = super::get_worker_state(scope) else {
        return false;
    };
    let mut state = state.borrow_mut();
    if state.closed || state.termination_requested.load(Ordering::Acquire) {
        return false;
    }
    let id = state.next_file_reading_id;
    state.next_file_reading_id = id.checked_add(1).expect("worker FileReading id exhausted");
    state.pending_file_reading_tasks.insert(id, task);
    state.ready_file_reading_tasks.lock().push_back(id);
    true
}

pub(crate) fn take_worker_file_reading_task(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<FileReadingTask> {
    let state = super::get_worker_state(scope)?;
    let mut state = state.borrow_mut();
    if !state.callback_task_delivery_allowed() {
        state.pending_file_reading_tasks.clear();
        state.ready_file_reading_tasks.lock().clear();
        return None;
    }
    let id = state.ready_file_reading_tasks.lock().pop_front()?;
    state.pending_file_reading_tasks.remove(&id)
}
