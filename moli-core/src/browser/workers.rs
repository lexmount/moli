use moli_shared_worker::SharedWorkerInstanceId;

use super::BrowserContextId;
use crate::page::RendererSharedWorkerTargetInfo;

/// Physical Worker identity, scoped to its original BrowserContext incarnation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorkerHandle {
    Shared {
        context: BrowserContextId,
        instance: SharedWorkerInstanceId,
    },
}

/// Live native facts, with no protocol target, attachment or session state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerSnapshot {
    Shared {
        context: BrowserContextId,
        info: RendererSharedWorkerTargetInfo,
    },
}

impl WorkerSnapshot {
    pub fn handle(&self) -> WorkerHandle {
        match self {
            Self::Shared { context, info } => WorkerHandle::Shared {
                context: *context,
                instance: info.instance_id,
            },
        }
    }
}
