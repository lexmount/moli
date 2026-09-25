use crate::{HistoryEntryRef, ScrollRestoration, SerializedScriptValue};

/// No wrappers or engine handles are stored here. A binding's state cache is
/// invalidated by `revision`, including when a traversal returns to an entry
/// whose original serialized state has not changed.
#[derive(Debug)]
pub struct WindowHistory {
    entries: Vec<HistoryEntryRef>,
    current: u32,
    revision: u64,
}

impl WindowHistory {
    pub fn new(entries: Vec<HistoryEntryRef>, current: u32) -> Self {
        Self {
            entries,
            current,
            revision: 0,
        }
    }

    pub fn entries(&self) -> &[HistoryEntryRef] {
        &self.entries
    }

    pub fn current_index(&self) -> u32 {
        self.current
    }

    pub fn current_entry(&self) -> Option<&HistoryEntryRef> {
        self.entries.get(self.current as usize)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn invalidate_state_cache(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("history revision exhausted");
    }

    pub fn set_current_index(&mut self, index: u32) {
        if self.current != index {
            self.current = index;
            self.invalidate_state_cache();
        }
    }

    /// Preserve the selected record and its JS state identity when pruning
    /// unrelated entries. The binding can then select a different restored
    /// index explicitly when installing a new history view.
    pub fn restore_entries(&mut self, entries: Vec<HistoryEntryRef>) {
        let retained_current = self.current_entry().and_then(|current| {
            entries
                .iter()
                .position(|entry| std::rc::Rc::ptr_eq(current, entry))
        });
        self.entries = entries;
        if let Some(index) = retained_current {
            self.current = index as u32;
        } else {
            self.invalidate_state_cache();
        }
    }

    pub fn push(&mut self, entry: HistoryEntryRef) -> Vec<HistoryEntryRef> {
        let restoration = self.scroll_restoration();
        entry.borrow_mut().scroll_restoration = restoration;
        let next = (self.current as usize + 1).min(self.entries.len());
        let removed = self.entries.split_off(next);
        self.entries.push(entry);
        self.current = next as u32;
        self.invalidate_state_cache();
        removed
    }

    pub fn replace(&mut self, entry: HistoryEntryRef) -> Option<HistoryEntryRef> {
        let restoration = self.scroll_restoration();
        entry.borrow_mut().scroll_restoration = restoration;
        let current = self.entries.get_mut(self.current as usize)?;
        let previous = std::mem::replace(current, entry);
        self.invalidate_state_cache();
        Some(previous)
    }

    pub fn state(&self) -> Option<SerializedScriptValue> {
        self.current_entry()?.borrow().history_state.clone()
    }

    pub fn set_state(&mut self, state: Option<SerializedScriptValue>) {
        if let Some(entry) = self.current_entry() {
            entry.borrow_mut().history_state = state;
        }
        self.invalidate_state_cache();
    }

    pub fn scroll_restoration(&self) -> ScrollRestoration {
        self.current_entry()
            .map_or(ScrollRestoration::Auto, |entry| {
                entry.borrow().scroll_restoration
            })
    }

    pub fn set_scroll_restoration(&mut self, value: ScrollRestoration) {
        if let Some(entry) = self.current_entry() {
            entry.borrow_mut().scroll_restoration = value;
        }
    }
}
