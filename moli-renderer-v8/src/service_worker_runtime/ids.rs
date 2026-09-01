use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) struct ServiceWorkerRegistrationId(pub(super) u64);

impl ServiceWorkerRegistrationId {
    pub(crate) fn as_u64(self) -> u64 {
        self.0
    }

    pub(crate) fn from_u64_for_binding(value: u64) -> Self {
        Self(value)
    }

    #[cfg(test)]
    pub(crate) fn from_u64_for_test(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) struct ServiceWorkerVersionId(pub(super) u64);

impl ServiceWorkerVersionId {
    pub(crate) fn as_u64(self) -> u64 {
        self.0
    }

    pub(crate) fn from_u64_for_binding(value: u64) -> Self {
        Self(value)
    }

    #[cfg(test)]
    pub(crate) fn from_u64_for_test(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) struct ServiceWorkerClientId(pub(super) u64);

impl ServiceWorkerClientId {
    pub(crate) fn as_u64(self) -> u64 {
        self.0
    }

    pub(crate) fn from_u64_for_worker(value: u64) -> Self {
        Self(value)
    }

    #[cfg(test)]
    pub(crate) fn from_u64_for_test(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerClientIdAllocator {
    next: Arc<AtomicU64>,
}

impl Default for ServiceWorkerClientIdAllocator {
    fn default() -> Self {
        Self {
            next: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl ServiceWorkerClientIdAllocator {
    pub(crate) fn allocate(&self) -> ServiceWorkerClientId {
        ServiceWorkerClientId(self.next.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) struct ServiceWorkerEventId(pub(super) u64);

impl ServiceWorkerEventId {
    pub(crate) fn as_u64(self) -> u64 {
        self.0
    }

    pub(crate) fn from_u64_for_worker(value: u64) -> Self {
        Self(value)
    }
}
