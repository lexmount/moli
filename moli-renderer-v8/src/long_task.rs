use std::{
    cell::Cell,
    future::{Future, poll_fn},
    pin::pin,
    time::{Duration, Instant},
};

use crate::native_bridge::WindowExecutionContextIdentity;

const LONG_TASK_THRESHOLD: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ExecutedWindow {
    pub(crate) host_address: usize,
    pub(crate) identity: WindowExecutionContextIdentity,
}

#[derive(Clone, Copy)]
struct RunningTask {
    started: Instant,
    start_unix_millis: f64,
    first_window: Option<ExecutedWindow>,
    multiple_contexts: bool,
}

pub(crate) struct LongTask {
    pub(crate) start_unix_millis: f64,
    pub(crate) duration: f64,
    pub(crate) first_window: Option<ExecutedWindow>,
    pub(crate) multiple_contexts: bool,
}

thread_local! {
    static CURRENT_TASK: Cell<Option<RunningTask>> = const { Cell::new(None) };
}

/// Scope one synchronous part of an HTML task on this agent. Nested script
/// entries contribute their settings to the enclosing task.
pub(crate) struct TaskScope(bool);

impl TaskScope {
    pub(crate) fn begin() -> Self {
        CURRENT_TASK.with(|task| {
            if task.get().is_some() {
                return Self(false);
            }
            task.set(Some(RunningTask {
                started: Instant::now(),
                start_unix_millis: moli_time::unix_epoch_millis(),
                first_window: None,
                multiple_contexts: false,
            }));
            Self(true)
        })
    }

    pub(crate) fn is_active() -> bool {
        CURRENT_TASK.with(|task| task.get().is_some())
    }

    pub(crate) fn finish(mut self) -> Option<LongTask> {
        if !std::mem::replace(&mut self.0, false) {
            return None;
        }
        let task = CURRENT_TASK.take()?;
        let elapsed = task.started.elapsed();
        (elapsed >= LONG_TASK_THRESHOLD).then_some(LongTask {
            start_unix_millis: task.start_unix_millis,
            duration: elapsed.as_millis() as f64,
            first_window: task.first_window,
            multiple_contexts: task.multiple_contexts,
        })
    }
}

impl Drop for TaskScope {
    fn drop(&mut self) {
        if self.0 {
            CURRENT_TASK.take();
        }
    }
}

/// A yielded future leaves the agent free to run other work. Account only for
/// the synchronous execution slices, including their microtask checkpoints.
pub(crate) async fn measure<T>(work: impl Future<Output = T>) -> (T, Vec<LongTask>) {
    let mut work = pin!(work);
    let mut tasks = Vec::new();
    let result = poll_fn(|cx| {
        let timing = TaskScope::begin();
        let result = work.as_mut().poll(cx);
        tasks.extend(timing.finish());
        result
    })
    .await;
    (result, tasks)
}

pub(crate) fn note_window_execution(scope: &mut v8::PinScope<'_, '_>) {
    if !TaskScope::is_active() {
        return;
    }
    let context = scope.get_current_context();
    let Some(host_ptr) = crate::util::context_host_ptr_from_context_slot(context) else {
        return;
    };
    let host = unsafe { &*host_ptr };
    let Some(identity) = host.window_execution_context_identity_for_v8_context(scope, context)
    else {
        return;
    };
    if identity.grants_universal_access() {
        return;
    }
    let window = ExecutedWindow {
        host_address: host_ptr as usize,
        identity,
    };
    CURRENT_TASK.with(|task| {
        let Some(mut running) = task.get() else {
            return;
        };
        if let Some(first) = running.first_window {
            running.multiple_contexts |= first != window;
        } else {
            running.first_window = Some(window);
        }
        task.set(Some(running));
    });
}
