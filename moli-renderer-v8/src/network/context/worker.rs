use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use parking_lot::Mutex;

use crate::network::{
    RendererResourceTaskRunner, ResourceRequestClient,
    loads::{
        ResourceLoadDisposition, ResourceLoadKind, ResourceLoadLease, ResourceLoadRegistry,
        ResourceLoadRegistryDiagnostics,
    },
};

static NEXT_WORKER_RESOURCE_LOADER_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WorkerResourceOwner {
    Dedicated {
        name: Box<str>,
    },
    Shared {
        name: Box<str>,
    },
    Service {
        registration_id: u64,
        version_id: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerResourceLoaderState {
    Active,
    Detaching,
    Detached,
}

struct WorkerResourceLoaderAuthority {
    id: u64,
    owner: WorkerResourceOwner,
    state: Mutex<WorkerResourceLoaderState>,
    loads: ResourceLoadRegistry,
}

impl Drop for WorkerResourceLoaderAuthority {
    fn drop(&mut self) {
        // Normal Worker shutdown detaches before V8 teardown. Keep destruction
        // itself safe as well so bootstrap failures cannot leave an ordinary
        // transport running without its WorkerGlobalScope.
        self.loads.begin_detach();
    }
}

/// Resource authority for one actual WorkerGlobalScope lifetime.
///
/// Top-level script fetching happens in outside settings before this object is
/// created. Once the global exists, importScripts, module descendants,
/// fetch/XHR and other inside-settings requests all share this exact authority.
#[derive(Clone)]
pub(crate) struct WorkerResourceLoader {
    request_client: ResourceRequestClient,
    authority: Arc<WorkerResourceLoaderAuthority>,
}

impl WorkerResourceLoader {
    pub(crate) fn new(
        request_client: ResourceRequestClient,
        owner: WorkerResourceOwner,
        task_runner: RendererResourceTaskRunner,
    ) -> Self {
        let loads = ResourceLoadRegistry::new(task_runner);
        Self {
            request_client: request_client.with_load_context(&loads),
            authority: Arc::new(WorkerResourceLoaderAuthority {
                id: NEXT_WORKER_RESOURCE_LOADER_ID
                    .fetch_add(1, Ordering::Relaxed)
                    .checked_add(1)
                    .expect("worker resource loader id exhausted"),
                owner,
                state: Mutex::new(WorkerResourceLoaderState::Active),
                loads,
            }),
        }
    }

    pub(crate) fn register_load(
        &self,
        kind: ResourceLoadKind,
        disposition: ResourceLoadDisposition,
        cancel_handle: Option<moli_fetch::FetchCancelHandle>,
    ) -> Option<ResourceLoadLease> {
        if self.state() != WorkerResourceLoaderState::Active {
            return None;
        }
        self.authority.loads.register(
            kind,
            disposition,
            self.request_client.frozen_request_client(),
            cancel_handle,
        )
    }

    /// Returns the immutable request client captured for this Worker context.
    ///
    /// Low-level transport helpers intentionally receive only the browser
    /// runtime plus the Worker's request-policy view. Callers must first
    /// acquire a load lease from this authority; the client is not itself a
    /// Worker lifecycle capability.
    pub(crate) fn request_client(&self) -> &ResourceRequestClient {
        &self.request_client
    }

    pub(crate) fn begin_detach(&self) -> bool {
        let mut state = self.authority.state.lock();
        if *state != WorkerResourceLoaderState::Active {
            return false;
        }
        *state = WorkerResourceLoaderState::Detaching;
        drop(state);
        self.authority.loads.begin_detach();
        true
    }

    pub(crate) fn finish_detach(&self) {
        let mut state = self.authority.state.lock();
        if matches!(
            *state,
            WorkerResourceLoaderState::Active | WorkerResourceLoaderState::Detaching
        ) {
            *state = WorkerResourceLoaderState::Detached;
        }
    }

    pub(crate) fn state(&self) -> WorkerResourceLoaderState {
        *self.authority.state.lock()
    }

    pub(crate) fn loader_id_for_diagnostics(&self) -> u64 {
        self.authority.id
    }

    pub(crate) fn owner(&self) -> &WorkerResourceOwner {
        &self.authority.owner
    }

    pub(crate) fn load_diagnostics(&self) -> ResourceLoadRegistryDiagnostics {
        self.authority.loads.diagnostics()
    }
}

impl std::fmt::Debug for WorkerResourceLoader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkerResourceLoader")
            .field("id", &self.loader_id_for_diagnostics())
            .field("owner", self.owner())
            .field("state", &self.state())
            .field("loads", &self.load_diagnostics())
            .finish()
    }
}
