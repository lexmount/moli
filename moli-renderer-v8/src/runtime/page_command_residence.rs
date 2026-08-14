//! Stable per-Page admission state for renderer Inspector commands.
//!
//! A Page entry is temporarily checked out while an owner turn runs, so
//! Inspector-session ordering cannot live in `RendererPageLocalEntry`. This
//! residence stays in the stable Page slot and admits each session's commands
//! in arrival order until they reach their first V8 dispatch. Independent
//! sessions on the same Page have independent lanes.

use std::collections::{BTreeMap, VecDeque};

pub(super) struct PageCommandFirstDispatchResidence<Key, Command> {
    /// Presence of a lane is the stable marker for its active command, which
    /// has already moved to the renderer owner. The deque retains only later
    /// same-session commands.
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
    /// Returns the command when it owns its session's active dispatch slot. A
    /// `None` result means the Page residence retained it behind a predecessor
    /// from the same session.
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

    /// Releases one session's active command and returns its next FIFO waiter,
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

    #[test]
    fn each_inspector_session_owns_an_independent_fifo_dispatch_lane() {
        let mut first_page = PageCommandFirstDispatchResidence::default();
        let mut other_page = PageCommandFirstDispatchResidence::default();

        assert_eq!(first_page.admit("session-a", "a-first"), Some("a-first"));
        assert_eq!(first_page.admit("session-a", "a-second"), None);
        assert_eq!(
            first_page.admit("session-b", "b-first"),
            Some("b-first"),
            "a parked command must not block a different Inspector session"
        );
        assert_eq!(first_page.admit("session-b", "b-second"), None);

        assert_eq!(other_page.admit("session-a", "other"), Some("other"));
        assert_eq!(other_page.complete(&"session-a"), None);

        assert_eq!(first_page.complete(&"session-b"), Some("b-second"));
        assert_eq!(first_page.complete(&"session-b"), None);
        assert_eq!(first_page.complete(&"session-a"), Some("a-second"));
        assert_eq!(first_page.complete(&"session-a"), None);

        assert_eq!(
            first_page.admit("session-a", "a-after-idle"),
            Some("a-after-idle")
        );
    }
}
