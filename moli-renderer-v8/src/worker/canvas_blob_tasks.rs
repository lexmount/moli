//! Canvas encoder results are delivered as worker tasks, outside timer IDs.
//! V8 residence stays on this event loop; only pixels and encoded bytes cross
//! the blocking-job boundary.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use tokio::sync::{mpsc, oneshot};

use super::WorkerMessage;
use crate::context_bootstrap::{CanvasBlobEncodeJob, CanvasBlobFile, CanvasBlobPromise};

struct PendingCanvasBlob {
    promise: CanvasBlobPromise,
    encoded: oneshot::Receiver<Option<CanvasBlobFile>>,
}

struct Tasks {
    next_id: u64,
    pending: HashMap<u64, PendingCanvasBlob>,
}

#[derive(Clone)]
pub(super) struct WorkerCanvasBlobTasks {
    tasks: Rc<RefCell<Tasks>>,
    wake_tx: mpsc::UnboundedSender<WorkerMessage>,
}

impl WorkerCanvasBlobTasks {
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
        let file = pending
            .encoded
            .try_recv()
            .expect("ready worker canvas task must retain its encoder result");
        pending.promise.settle(scope, file);
    }
}

pub(crate) fn queue_worker_canvas_blob_task(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    canvas_context: v8::Local<'_, v8::Context>,
    encode: CanvasBlobEncodeJob,
) -> bool {
    let Some(queue) = scope.get_slot::<WorkerCanvasBlobTasks>().cloned() else {
        return false;
    };
    let (encoded_tx, encoded_rx) = oneshot::channel();
    let promise = CanvasBlobPromise::new(scope, resolver, canvas_context);
    let task_id = {
        let mut tasks = queue.tasks.borrow_mut();
        let task_id = tasks.next_id;
        tasks.next_id = task_id
            .checked_add(1)
            .expect("worker canvas task id overflow");
        let replaced = tasks.pending.insert(
            task_id,
            PendingCanvasBlob {
                promise,
                encoded: encoded_rx,
            },
        );
        assert!(
            replaced.is_none(),
            "worker canvas task ids must never be reused"
        );
        task_id
    };
    let wake_tx = queue.wake_tx;
    encode.spawn(encoded_tx, move || {
        let _ = wake_tx.send(WorkerMessage::RunCanvasBlobTask(task_id));
    });
    true
}
