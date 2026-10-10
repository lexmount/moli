//! ImageBitmap completions belong to the worker event loop. Decoders retain
//! only owned pixels/bytes and a single-use completion route, never V8 handles.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use tokio::sync::{mpsc, oneshot};

use super::WorkerMessage;
use crate::context_bootstrap::{BitmapRejection, BitmapTaskResult, settle_bitmap_task_result};

type BitmapResult = Result<BitmapTaskResult, BitmapRejection>;

struct PendingBitmap {
    resolver: v8::Global<v8::PromiseResolver>,
    context: v8::Global<v8::Context>,
    result: oneshot::Receiver<BitmapResult>,
}

struct Tasks {
    next_id: u64,
    pending: HashMap<u64, PendingBitmap>,
}

#[derive(Clone)]
pub(super) struct WorkerBitmapTasks {
    tasks: Rc<RefCell<Tasks>>,
    wake_tx: mpsc::UnboundedSender<WorkerMessage>,
}

impl WorkerBitmapTasks {
    pub(super) fn new(wake_tx: mpsc::UnboundedSender<WorkerMessage>) -> Self {
        Self {
            tasks: Rc::new(RefCell::new(Tasks {
                next_id: 1,
                pending: HashMap::new(),
            })),
            wake_tx,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.tasks.borrow().pending.is_empty()
    }

    pub(super) fn clear(&self) {
        self.tasks.borrow_mut().pending.clear();
    }

    pub(super) fn settle(&self, scope: &mut v8::PinScope<'_, '_>, task_id: u64) {
        let Some(mut pending) = self.tasks.borrow_mut().pending.remove(&task_id) else {
            return;
        };
        let result = pending
            .result
            .try_recv()
            .expect("ready worker Bitmap task must retain its result");
        let context = v8::Local::new(scope, pending.context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let resolver = v8::Local::new(scope, pending.resolver);
        settle_bitmap_task_result(scope, resolver, result);
    }
}

pub(crate) struct WorkerBitmapTaskProducer {
    task_id: u64,
    result: oneshot::Sender<BitmapResult>,
    wake_tx: mpsc::UnboundedSender<WorkerMessage>,
}

impl WorkerBitmapTaskProducer {
    pub(crate) fn send(self, result: BitmapResult) {
        // A closed worker drops the receiver and every resident V8 handle.
        // Late native decoders can then discard their result without waking it.
        if self.result.send(result).is_ok() {
            let _ = self
                .wake_tx
                .send(WorkerMessage::RunBitmapTask(self.task_id));
        }
    }
}

pub(crate) fn register_worker_bitmap_task(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
) -> Option<WorkerBitmapTaskProducer> {
    let queue = scope.get_slot::<WorkerBitmapTasks>()?.clone();
    let (result_tx, result_rx) = oneshot::channel();
    let pending = PendingBitmap {
        resolver: v8::Global::new(scope, resolver),
        context: v8::Global::new(scope, scope.get_current_context()),
        result: result_rx,
    };
    let task_id = {
        let mut tasks = queue.tasks.borrow_mut();
        let task_id = tasks.next_id;
        tasks.next_id = task_id
            .checked_add(1)
            .expect("worker Bitmap task id overflow");
        let replaced = tasks.pending.insert(task_id, pending);
        assert!(
            replaced.is_none(),
            "worker Bitmap task ids must never be reused"
        );
        task_id
    };
    Some(WorkerBitmapTaskProducer {
        task_id,
        result: result_tx,
        wake_tx: queue.wake_tx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_bitmap_teardown_discards_late_native_results_without_waking() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let (wake_tx, mut wake_rx) = mpsc::unbounded_channel();
        let queue = WorkerBitmapTasks::new(wake_tx);
        scope.set_slot(queue.clone());
        let resolver = v8::PromiseResolver::new(scope).unwrap();
        let producer = register_worker_bitmap_task(scope, resolver).unwrap();
        assert!(!queue.is_empty());
        queue.clear();
        scope.remove_slot::<WorkerBitmapTasks>();
        producer.send(Err(BitmapRejection::InvalidState));
        assert!(queue.is_empty());
        assert!(matches!(
            wake_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn worker_bitmap_stale_completion_cannot_consume_a_later_request() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let (wake_tx, mut wake_rx) = mpsc::unbounded_channel();
        let queue = WorkerBitmapTasks::new(wake_tx);
        scope.set_slot(queue.clone());
        let first = v8::PromiseResolver::new(scope).unwrap();
        register_worker_bitmap_task(scope, first)
            .unwrap()
            .send(Err(BitmapRejection::InvalidState));
        let WorkerMessage::RunBitmapTask(first_id) = wake_rx.try_recv().unwrap() else {
            panic!("wrong task");
        };
        queue.clear();
        let second = v8::PromiseResolver::new(scope).unwrap();
        register_worker_bitmap_task(scope, second)
            .unwrap()
            .send(Err(BitmapRejection::InvalidState));
        let WorkerMessage::RunBitmapTask(second_id) = wake_rx.try_recv().unwrap() else {
            panic!("wrong task");
        };
        assert_ne!(first_id, second_id);
        queue.settle(scope, first_id);
        assert!(!queue.is_empty());
        queue.clear();
        scope.remove_slot::<WorkerBitmapTasks>();
    }
}
