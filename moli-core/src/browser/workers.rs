use moli_shared_worker::SharedWorkerInstanceId;

use super::BrowserContextId;
use crate::page::RendererSharedWorkerTargetInfo;

/// Physical Worker identity, scoped to its original BrowserContext incarnation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorkerHandle {
    Dedicated {
        context: BrowserContextId,
        instance: u64,
    },
    Shared {
        context: BrowserContextId,
        instance: SharedWorkerInstanceId,
    },
}

/// Live native facts, with no protocol target, attachment or session state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerSnapshot {
    Dedicated {
        context: BrowserContextId,
        worker: DedicatedWorkerSnapshot,
    },
    Shared {
        context: BrowserContextId,
        info: RendererSharedWorkerTargetInfo,
    },
}

impl WorkerSnapshot {
    pub fn handle(&self) -> WorkerHandle {
        match self {
            Self::Dedicated { context, worker } => WorkerHandle::Dedicated {
                context: *context,
                instance: worker.info.instance_id,
            },
            Self::Shared { context, info } => WorkerHandle::Shared {
                context: *context,
                instance: info.instance_id,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DedicatedWorkerSnapshot {
    pub info: crate::page::RendererDedicatedWorkerTargetInfo,
    pub main_script: Option<std::sync::Arc<crate::page::RendererDedicatedWorkerMainScript>>,
}
