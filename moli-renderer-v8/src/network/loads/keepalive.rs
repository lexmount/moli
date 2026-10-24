use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use moli_fetch::FetchCancelHandle;
use parking_lot::Mutex;

use super::registry::{ResourceLoadId, ResourceLoadKind};

const KEEPALIVE_BODY_QUOTA: usize = 65536;

/// In-flight body bytes belonging to one Fetch group, independent of its
/// current Document authority and of any JavaScript completion route.
#[derive(Clone, Default)]
pub(super) struct KeepaliveBodyBudget(Arc<AtomicUsize>);

impl KeepaliveBodyBudget {
    pub(super) fn reserve(&self, bytes: usize) -> Option<KeepaliveBodyReservation> {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|total| *total <= KEEPALIVE_BODY_QUOTA)
            })
            .ok()?;
        Some(KeepaliveBodyReservation {
            budget: self.clone(),
            bytes,
        })
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.load(Ordering::Acquire) == 0
    }
}

pub(super) struct KeepaliveBodyReservation {
    budget: KeepaliveBodyBudget,
    bytes: usize,
}

impl Drop for KeepaliveBodyReservation {
    fn drop(&mut self) {
        let previous = self.budget.0.fetch_sub(self.bytes, Ordering::AcqRel);
        debug_assert!(previous >= self.bytes);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DetachedKeepaliveLoadDiagnostics {
    pub(crate) active_load_count: usize,
}

struct DetachedKeepaliveLoad {
    _kind: ResourceLoadKind,
    cancel_handle: Option<FetchCancelHandle>,
}

/// Browser-runtime-owned tail of a keepalive request.
///
/// Entries here deliberately contain only network cancellation state and
/// diagnostics. A detached keepalive must not retain a Document, Worker, V8
/// context, promise resolver, or DOM completion route.
#[derive(Default)]
pub(crate) struct DetachedKeepaliveLoadRegistry {
    loads: Mutex<HashMap<ResourceLoadId, DetachedKeepaliveLoad>>,
}

impl DetachedKeepaliveLoadRegistry {
    pub(crate) fn insert(
        &self,
        id: ResourceLoadId,
        kind: ResourceLoadKind,
        cancel_handle: Option<FetchCancelHandle>,
    ) {
        let previous = self.loads.lock().insert(
            id,
            DetachedKeepaliveLoad {
                _kind: kind,
                cancel_handle,
            },
        );
        assert!(
            previous.is_none(),
            "detached keepalive load registered twice: {id:?}"
        );
    }

    pub(crate) fn attach_cancel_handle(
        &self,
        id: ResourceLoadId,
        cancel_handle: FetchCancelHandle,
    ) -> bool {
        let mut loads = self.loads.lock();
        let Some(load) = loads.get_mut(&id) else {
            return false;
        };
        load.cancel_handle = Some(cancel_handle);
        true
    }

    pub(crate) fn remove(&self, id: ResourceLoadId) -> bool {
        self.loads.lock().remove(&id).is_some()
    }

    pub(crate) fn diagnostics(&self) -> DetachedKeepaliveLoadDiagnostics {
        DetachedKeepaliveLoadDiagnostics {
            active_load_count: self.loads.lock().len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keepalive_body_budget_rejects_overflow_and_releases_only_its_own_bytes() {
        let budget = KeepaliveBodyBudget::default();
        assert!(budget.reserve(usize::MAX).is_none());
        assert!(budget.reserve(65537).is_none());
        let first = budget.reserve(65535).unwrap();
        let second = budget.reserve(1).unwrap();
        assert!(budget.reserve(1).is_none());
        assert!(budget.reserve(0).is_some());
        drop(second);
        assert!(budget.reserve(2).is_none());
        let third = budget.reserve(1).unwrap();
        drop(first);
        assert!(budget.reserve(65536).is_none());
        drop(third);
        assert!(budget.reserve(65536).is_some());
    }
}
