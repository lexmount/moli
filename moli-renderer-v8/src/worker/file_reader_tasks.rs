//! FileReader requests stay in native state so queue operations cannot call
//! author code through Array.prototype. The worker loop drains these tasks
//! before waiting, with a microtask checkpoint after each request phase.

use std::sync::atomic::Ordering;

use crate::context_bootstrap::FileReaderTask;

pub(crate) fn queue_worker_file_reader_task(
    scope: &mut v8::PinScope<'_, '_>,
    task: FileReaderTask,
) -> bool {
    let Some(state) = super::get_worker_state(scope) else {
        return false;
    };
    let mut state = state.borrow_mut();
    if state.closed || state.termination_requested.load(Ordering::Acquire) {
        return false;
    }
    state.file_reader_tasks.push_back(task);
    true
}

pub(crate) fn take_worker_file_reader_task(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<FileReaderTask> {
    let state = super::get_worker_state(scope)?;
    let mut state = state.borrow_mut();
    if state.closed || state.termination_requested.load(Ordering::Acquire) {
        return None;
    }
    state.file_reader_tasks.pop_front()
}
