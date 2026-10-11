//! Sendable tasks and lifetime authority for a Window/Worker RTP transform pair.

use super::WorkerMessage;
use crate::structured_clone::V8StructuredClonePayload;
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc;

#[derive(Debug)]
pub(crate) struct TransformLease {
    id: u64,
    alive: AtomicBool,
    wake: Mutex<Option<mpsc::UnboundedSender<WorkerMessage>>>,
}

impl TransformLease {
    pub(crate) fn new(id: u64) -> Arc<Self> {
        Arc::new(Self {
            id,
            alive: AtomicBool::new(true),
            wake: Mutex::new(None),
        })
    }

    pub(crate) fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    pub(crate) fn bind(&self, wake: mpsc::UnboundedSender<WorkerMessage>) {
        *self.wake.lock() = Some(wake);
    }

    pub(crate) fn retire(&self) {
        self.alive.store(false, Ordering::Release);
        if let Some(wake) = self.wake.lock().as_ref() {
            let _ = wake.send(WorkerMessage::RtpTransform(RtpTransformTask::Retire {
                id: self.id,
            }));
        }
    }
}

#[derive(Debug)]
pub(crate) enum RtpTransformTask {
    Create {
        id: u64,
        options: Box<V8StructuredClonePayload>,
        lease: Arc<TransformLease>,
    },
    Associate {
        id: u64,
    },
    Disassociate {
        id: u64,
    },
    Retire {
        id: u64,
    },
}

pub(crate) struct TransformerEntry {
    pub(crate) transformer: v8::Global<v8::Object>,
    pub(crate) lease: Arc<TransformLease>,
}
