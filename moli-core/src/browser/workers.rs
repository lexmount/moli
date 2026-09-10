use moli_shared_worker::SharedWorkerInstanceId;

use super::BrowserContextId;
use crate::page::RendererSharedWorkerTargetInfo;

/// Physical Worker identity, scoped to its original BrowserContext incarnation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorkerHandle {
    Service {
        context: BrowserContextId,
        version: u64,
    },
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
    Service {
        context: BrowserContextId,
        worker: ServiceWorkerSnapshot,
    },
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
            Self::Service { context, worker } => WorkerHandle::Service {
                context: *context,
                version: worker.info.version_id,
            },
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceWorkerSnapshot {
    pub info: crate::page::RendererServiceWorkerTargetInfo,
    pub execution: ServiceWorkerExecution,
}

/// A stable version may outlive many exact physical runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceWorkerExecution {
    Stopped,
    Starting(crate::page::RendererServiceWorkerRunIdentity),
    /// The executor exists; bootstrap may still be waiting for a debugger.
    Bootstrapping(crate::page::RendererServiceWorkerRunIdentity),
    Running(crate::page::RendererServiceWorkerRunIdentity),
}

impl ServiceWorkerExecution {
    pub fn active_run(&self) -> Option<&crate::page::RendererServiceWorkerRunIdentity> {
        match self {
            Self::Stopped => None,
            Self::Starting(run) | Self::Bootstrapping(run) | Self::Running(run) => Some(run),
        }
    }
}
