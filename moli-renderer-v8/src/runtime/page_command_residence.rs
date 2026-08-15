//! Stable per-Page admission state for renderer Inspector commands.
//!
//! A Page entry is temporarily checked out while an owner turn runs, so
//! Inspector-session ordering cannot live in `RendererPageLocalEntry`. This
//! residence stays in the stable Page slot and admits each `(session, route)`
//! stream in arrival order until commands reach their first V8 dispatch.
//! Independent sessions and Chromium-style main-thread/IO routes on the same
//! Page have independent lanes.

use std::collections::{BTreeMap, VecDeque};

pub(super) struct PageCommandFirstDispatchResidence<Key, Command> {
    /// Presence of a lane is the stable marker for its active command, which
    /// has already moved to the renderer owner. The deque retains only later
    /// same-lane commands.
    lanes: BTreeMap<Key, VecDeque<Command>>,
}

impl<Key, Command> std::fmt::Debug for PageCommandFirstDispatchResidence<Key, Command> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PageCommandFirstDispatchResidence")
            .field("active_session_count", &self.lanes.len())
            .field(
                "waiting_count",
                &self.lanes.values().map(VecDeque::len).sum::<usize>(),
            )
            .finish()
    }
}

impl<Key, Command> Default for PageCommandFirstDispatchResidence<Key, Command> {
    fn default() -> Self {
        Self {
            lanes: BTreeMap::new(),
        }
    }
}

impl<Key: Ord, Command> PageCommandFirstDispatchResidence<Key, Command> {
    /// Returns the command when it owns its lane's active dispatch slot. A
    /// `None` result means the Page residence retained it behind a predecessor
    /// from the same lane.
    pub(super) fn admit(&mut self, key: Key, command: Command) -> Option<Command> {
        match self.lanes.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(VecDeque::new());
                Some(command)
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                entry.get_mut().push_back(command);
                None
            }
        }
    }

    /// Releases one lane's active command and returns its next FIFO waiter,
    /// which inherits the lane without an intermediate idle state.
    pub(super) fn complete(&mut self, key: &Key) -> Option<Command> {
        debug_assert!(
            self.lanes.contains_key(key),
            "an Inspector session dispatch lane must be active"
        );
        let waiting = self.lanes.get_mut(key)?;
        if let Some(next) = waiting.pop_front() {
            return Some(next);
        }
        self.lanes.remove(key);
        None
    }

    pub(super) fn drain_waiting(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.lanes)
            .into_values()
            .flatten()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RendererInspectorCommandRoute;

    type Lane = (&'static str, RendererInspectorCommandRoute);

    #[test]
    fn each_inspector_session_and_route_owns_an_independent_fifo_dispatch_lane() {
        let mut first_page = PageCommandFirstDispatchResidence::default();
        let mut other_page = PageCommandFirstDispatchResidence::default();
        let session_a_main: Lane = ("session-a", RendererInspectorCommandRoute::MainThread);
        let session_a_io: Lane = ("session-a", RendererInspectorCommandRoute::Io);
        let session_b_main: Lane = ("session-b", RendererInspectorCommandRoute::MainThread);

        assert_eq!(
            first_page.admit(session_a_main, "a-main-first"),
            Some("a-main-first")
        );
        assert_eq!(first_page.admit(session_a_main, "a-main-second"), None);
        assert_eq!(
            first_page.admit(session_a_io, "a-io-first"),
            Some("a-io-first"),
            "an IO command must not wait for main-thread work in the same session"
        );
        assert_eq!(
            first_page.admit(session_b_main, "b-first"),
            Some("b-first"),
            "a parked command must not block a different Inspector session"
        );
        assert_eq!(first_page.admit(session_b_main, "b-second"), None);

        assert_eq!(other_page.admit(session_a_main, "other"), Some("other"));
        assert_eq!(other_page.complete(&session_a_main), None);

        assert_eq!(first_page.complete(&session_b_main), Some("b-second"));
        assert_eq!(first_page.complete(&session_b_main), None);
        assert_eq!(first_page.complete(&session_a_io), None);
        assert_eq!(first_page.complete(&session_a_main), Some("a-main-second"));
        assert_eq!(first_page.complete(&session_a_main), None);

        assert_eq!(
            first_page.admit(session_a_main, "a-after-idle"),
            Some("a-after-idle")
        );
    }

    #[test]
    fn retiring_a_page_drains_waiters_and_resets_every_lane() {
        let mut residence = PageCommandFirstDispatchResidence::default();
        let main: Lane = ("session-a", RendererInspectorCommandRoute::MainThread);
        let io: Lane = ("session-a", RendererInspectorCommandRoute::Io);

        assert_eq!(residence.admit(main, "main-active"), Some("main-active"));
        assert_eq!(residence.admit(main, "main-waiting"), None);
        assert_eq!(residence.admit(io, "io-active"), Some("io-active"));
        assert_eq!(residence.admit(io, "io-waiting"), None);

        let mut drained = residence.drain_waiting();
        drained.sort_unstable();
        assert_eq!(drained, ["io-waiting", "main-waiting"]);
        assert_eq!(
            residence.admit(main, "main-after-retirement"),
            Some("main-after-retirement"),
            "retirement must not leave a stale active-lane marker"
        );
    }
}
