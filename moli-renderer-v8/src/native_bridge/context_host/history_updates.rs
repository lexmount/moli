use super::{JsContextHost, OwnerDispatchScope};
use std::{
    cell::Cell,
    collections::HashMap,
    rc::Rc,
    time::{Duration, Instant},
};

// Allow bursts without permitting sustained history flooding. The limits are
// implementation policy; blocked History mutations and traversal requests
// return without throwing or starting a navigation.
const UPDATE_LIMIT: usize = 200;
const UPDATE_INTERVAL: Duration = Duration::from_secs(10);
// Synchronous navigate/currententrychange callbacks can recursively mutate
// history before a time-based budget is exhausted. Bound their native stack
// across browsing contexts, including cycles between different Windows.
const REENTRANT_UPDATE_LIMIT: usize = 64;

struct HistoryUpdateWindow {
    started: Instant,
    updates: usize,
}

#[derive(Default)]
pub(super) struct HistoryUpdateLimits {
    windows: HashMap<OwnerDispatchScope, HistoryUpdateWindow>,
    active_depth: Rc<Cell<usize>>,
}

pub(crate) struct HistoryUpdatePermit(Rc<Cell<usize>>);

impl Drop for HistoryUpdatePermit {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

impl HistoryUpdateLimits {
    fn admit(&mut self, owner: OwnerDispatchScope, now: Instant) -> Option<HistoryUpdatePermit> {
        let depth = self.active_depth.get();
        if depth >= REENTRANT_UPDATE_LIMIT {
            return None;
        }
        let window = self.windows.entry(owner).or_insert(HistoryUpdateWindow {
            started: now,
            updates: 0,
        });
        if now.saturating_duration_since(window.started) >= UPDATE_INTERVAL {
            window.started = now;
            window.updates = 0;
        }
        if window.updates >= UPDATE_LIMIT {
            return None;
        }
        window.updates += 1;
        self.active_depth.set(depth + 1);
        Some(HistoryUpdatePermit(self.active_depth.clone()))
    }

    pub(super) fn remove(&mut self, owner: OwnerDispatchScope) {
        self.windows.remove(&owner);
    }
}

impl JsContextHost {
    pub(crate) fn begin_history_update(
        &mut self,
        owner: OwnerDispatchScope,
    ) -> Option<HistoryUpdatePermit> {
        self.history_update_limits.admit(owner, Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_update_limit_recovers_without_extending_the_window_on_denial() {
        let now = Instant::now();
        let mut limits = HistoryUpdateLimits::default();
        for _ in 0..UPDATE_LIMIT {
            assert!(limits.admit(OwnerDispatchScope::Top, now).is_some());
        }
        assert!(limits.admit(OwnerDispatchScope::Top, now).is_none());
        assert!(
            limits
                .admit(OwnerDispatchScope::Top, now + UPDATE_INTERVAL / 2)
                .is_none()
        );
        assert!(
            limits
                .admit(OwnerDispatchScope::Top, now + UPDATE_INTERVAL)
                .is_some()
        );
    }

    #[test]
    fn history_update_limit_is_scoped_to_live_browsing_contexts() {
        let now = Instant::now();
        let mut limits = HistoryUpdateLimits::default();
        let child = OwnerDispatchScope::Child(crate::document_runtime::DomHandle::new(1));
        for _ in 0..UPDATE_LIMIT {
            assert!(limits.admit(child, now).is_some());
        }
        assert!(limits.admit(child, now).is_none());
        assert!(limits.admit(OwnerDispatchScope::Top, now).is_some());
        assert!(
            limits
                .admit(OwnerDispatchScope::LightweightPopup(1), now)
                .is_some()
        );
        limits.remove(child);
        assert!(limits.admit(child, now).is_some());
    }

    #[test]
    fn history_update_reentrancy_limit_spans_windows_and_releases_on_return() {
        let now = Instant::now();
        let mut limits = HistoryUpdateLimits::default();
        let mut permits = Vec::new();
        for index in 0..REENTRANT_UPDATE_LIMIT {
            permits.push(
                limits
                    .admit(OwnerDispatchScope::LightweightPopup(index as u64), now)
                    .unwrap(),
            );
        }
        assert!(limits.admit(OwnerDispatchScope::Top, now).is_none());
        permits.pop();
        assert!(limits.admit(OwnerDispatchScope::Top, now).is_some());
        drop(permits);
        assert_eq!(limits.active_depth.get(), 0);
        assert!(limits.admit(OwnerDispatchScope::Top, now).is_some());
    }
}
