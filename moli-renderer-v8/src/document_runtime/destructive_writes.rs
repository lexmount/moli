use std::{cell::RefCell, collections::HashMap, rc::Rc};

use super::{DocumentRuntime, DomHandle};

#[derive(Debug, Default)]
pub(super) struct DocumentWriteCounters(Rc<RefCell<HashMap<DomHandle, usize>>>);

/// Owns an execute-script-element counter through script cleanup, including
/// its microtask checkpoint, but not subsequent tasks or pending TLA work.
/// The shared state avoids borrowing the runtime across JavaScript reentry.
pub(crate) struct IgnoreDestructiveWritesGuard {
    counters: Rc<RefCell<HashMap<DomHandle, usize>>>,
    document: DomHandle,
}

impl DocumentWriteCounters {
    fn enter(&self, document: DomHandle) -> IgnoreDestructiveWritesGuard {
        let mut counters = self.0.borrow_mut();
        let counter = counters.entry(document).or_default();
        *counter = counter
            .checked_add(1)
            .expect("document write counter overflow");
        IgnoreDestructiveWritesGuard {
            counters: Rc::clone(&self.0),
            document,
        }
    }

    fn is_active(&self, document: DomHandle) -> bool {
        self.0.borrow().contains_key(&document)
    }
}

impl Drop for IgnoreDestructiveWritesGuard {
    fn drop(&mut self) {
        let mut counters = self.counters.borrow_mut();
        let counter = counters
            .get_mut(&self.document)
            .expect("document write guard requires a matching counter");
        *counter -= 1;
        if *counter == 0 {
            counters.remove(&self.document);
        }
    }
}

impl DocumentRuntime {
    pub(crate) fn enter_ignore_destructive_writes(
        &self,
        document: DomHandle,
    ) -> IgnoreDestructiveWritesGuard {
        self.destructive_write_counters.enter(document)
    }

    pub(crate) fn has_ignore_destructive_writes_counter(&self, document: DomHandle) -> bool {
        self.destructive_write_counters.is_active(document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_write_counters_are_nested_and_document_scoped() {
        let counters = DocumentWriteCounters::default();
        let document = DomHandle::new(0);
        let other = DomHandle::new(1);
        let outer = counters.enter(document);
        assert!(counters.is_active(document));
        assert!(!counters.is_active(other));
        let inner = counters.enter(document);
        let other_guard = counters.enter(other);
        drop(outer);
        assert!(counters.is_active(document));
        drop(inner);
        assert!(!counters.is_active(document));
        assert!(counters.is_active(other));
        drop(other_guard);
        assert!(counters.0.borrow().is_empty());
    }

    #[test]
    fn destructive_write_guard_can_outlive_retired_runtime_state() {
        let counters = DocumentWriteCounters::default();
        let guard = counters.enter(DomHandle::new(0));
        drop(counters);
        drop(guard);
    }
}
