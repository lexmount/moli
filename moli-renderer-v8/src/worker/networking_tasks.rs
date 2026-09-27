//! Browser-owned networking tasks for one worker run. Stream close/error steps
//! can enqueue while WorkerGlobalState is borrowed, so the queue lives in an
//! independent isolate slot. The owner loop checks close/termination before
//! dispatch and retires any remaining callbacks during teardown.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use tokio::sync::mpsc;

use super::{WorkerMessage, timer_callback::WorkerTimerCallback};

#[derive(Clone)]
pub(super) struct WorkerNetworkingTaskQueue {
    callbacks: Rc<RefCell<VecDeque<WorkerTimerCallback>>>,
    wake_tx: mpsc::UnboundedSender<WorkerMessage>,
}

impl WorkerNetworkingTaskQueue {
    pub(super) fn new(wake_tx: mpsc::UnboundedSender<WorkerMessage>) -> Self {
        Self {
            callbacks: Rc::new(RefCell::new(VecDeque::new())),
            wake_tx,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.callbacks.borrow().is_empty()
    }

    pub(super) fn pop_front(&self) -> Option<WorkerTimerCallback> {
        self.callbacks.borrow_mut().pop_front()
    }

    pub(super) fn clear(&self) {
        self.callbacks.borrow_mut().clear();
    }
}

pub(crate) fn queue_worker_networking_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: v8::Local<'s, v8::Function>,
) -> bool {
    let Some(queue) = scope.get_slot::<WorkerNetworkingTaskQueue>().cloned() else {
        return false;
    };
    let callback = WorkerTimerCallback::browser_function(scope, callback);
    queue.callbacks.borrow_mut().push_back(callback);
    let _ = queue.wake_tx.send(WorkerMessage::RunNetworkingTask);
    true
}
