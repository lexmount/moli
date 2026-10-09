//! WebCodecs completions belong to the worker event loop, outside the author
//! timer ID space. Keeping this queue in an isolate slot also allows a codec
//! callback to enqueue work while WorkerGlobalState is borrowed.

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use tokio::sync::mpsc;

use super::{WorkerMessage, timer_callback::WorkerTimerCallback};

#[derive(Clone)]
pub(super) struct WorkerCodecTaskQueue {
    callbacks: Rc<RefCell<VecDeque<WorkerTimerCallback>>>,
    wake_tx: mpsc::UnboundedSender<WorkerMessage>,
}

impl WorkerCodecTaskQueue {
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

pub(crate) fn queue_worker_codec_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: v8::Local<'s, v8::Function>,
) -> bool {
    let Some(queue) = scope.get_slot::<WorkerCodecTaskQueue>().cloned() else {
        return false;
    };
    let callback = WorkerTimerCallback::browser_function(scope, callback);
    queue.callbacks.borrow_mut().push_back(callback);
    let _ = queue.wake_tx.send(WorkerMessage::RunCodecTask);
    true
}
