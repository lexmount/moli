use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

#[cfg(test)]
use std::cell::Cell;

use style::{
    properties::ComputedValues, selector_parser::PseudoElement, servo_arc::Arc as ServoArc,
};

use crate::document_runtime::DomHandle;

/// Cache for computed pseudo styles that have no canonical Stylo data slot.
///
/// Primary `ComputedValues` live only in Stylo `ElementData`. Production code
/// deliberately keeps no parallel index of primary publications; invalidation
/// consults the Document's canonical element-data bucket when it needs to know
/// whether any resolved style survives. The primary ledger below exists only
/// to preserve fine-grained invalidation assertions in tests.
pub(super) struct PseudoStyleCache {
    #[cfg(test)]
    primary_entries: RefCell<HashSet<ComputedElementStyleCacheKey>>,
    pseudo_entries: RefCell<HashMap<ComputedElementStyleCacheKey, ServoArc<ComputedValues>>>,
    keys_by_handle: RefCell<HashMap<DomHandle, HashSet<ComputedElementStyleCacheKey>>>,
    #[cfg(test)]
    write_generation: Cell<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct ComputedElementStyleCacheKey {
    pub(super) computed_cache_generation: u64,
    pub(super) handle: DomHandle,
    pub(super) pseudo_element: Option<PseudoElement>,
}

impl PseudoStyleCache {
    pub(super) fn new() -> Self {
        Self {
            #[cfg(test)]
            primary_entries: RefCell::new(HashSet::new()),
            pseudo_entries: RefCell::new(HashMap::new()),
            keys_by_handle: RefCell::new(HashMap::new()),
            #[cfg(test)]
            write_generation: Cell::new(0),
        }
    }

    pub(super) fn clear(&self) {
        #[cfg(test)]
        self.primary_entries.borrow_mut().clear();
        self.pseudo_entries.borrow_mut().clear();
        self.keys_by_handle.borrow_mut().clear();
    }

    #[cfg(test)]
    pub(super) fn record_primary(&self, key: ComputedElementStyleCacheKey) {
        debug_assert!(key.pseudo_element.is_none());
        if self.primary_entries.borrow_mut().insert(key.clone()) {
            self.index_key(&key);
            self.bump_write_generation();
        }
    }

    pub(super) fn get_pseudo(
        &self,
        key: &ComputedElementStyleCacheKey,
    ) -> Option<ServoArc<ComputedValues>> {
        self.pseudo_entries.borrow().get(key).cloned()
    }

    pub(super) fn insert_pseudo(
        &self,
        key: ComputedElementStyleCacheKey,
        style: ServoArc<ComputedValues>,
    ) {
        debug_assert!(key.pseudo_element.is_some());
        let is_new = self
            .pseudo_entries
            .borrow_mut()
            .insert(key.clone(), style)
            .is_none();
        if is_new {
            self.index_key(&key);
            #[cfg(test)]
            self.bump_write_generation();
        }
    }

    #[cfg(test)]
    pub(super) fn write_generation(&self) -> u64 {
        self.write_generation.get()
    }

    pub(super) fn invalidate_handles(&self, handles: impl IntoIterator<Item = DomHandle>) {
        let mut keys = Vec::new();
        {
            let mut keys_by_handle = self.keys_by_handle.borrow_mut();
            for handle in handles {
                if let Some(handle_keys) = keys_by_handle.remove(&handle) {
                    keys.extend(handle_keys);
                }
            }
        }
        if keys.is_empty() {
            return;
        }
        #[cfg(test)]
        let mut primary_entries = self.primary_entries.borrow_mut();
        let mut pseudo_entries = self.pseudo_entries.borrow_mut();
        for key in keys {
            #[cfg(test)]
            {
                primary_entries.remove(&key);
            }
            pseudo_entries.remove(&key);
        }
    }

    fn index_key(&self, key: &ComputedElementStyleCacheKey) {
        self.keys_by_handle
            .borrow_mut()
            .entry(key.handle)
            .or_default()
            .insert(key.clone());
    }

    #[cfg(test)]
    fn bump_write_generation(&self) {
        self.write_generation
            .set(self.write_generation.get().saturating_add(1));
    }

    pub(super) fn is_empty(&self) -> bool {
        self.pseudo_entries.borrow().is_empty() && self.keys_by_handle.borrow().is_empty()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.primary_entries.borrow().len() + self.pseudo_entries.borrow().len()
    }

    #[cfg(test)]
    pub(super) fn contains_handle_for_test(&self, handle: DomHandle) -> bool {
        self.keys_by_handle.borrow().contains_key(&handle)
    }

    #[cfg(test)]
    pub(super) fn entry_count_for_handle_for_test(&self, handle: DomHandle) -> usize {
        self.keys_by_handle
            .borrow()
            .get(&handle)
            .map(HashSet::len)
            .unwrap_or(0)
    }
}
