use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

use style::{
    data::ElementStyles, properties::ComputedValues, selector_parser::PseudoElement,
    servo_arc::Arc as ServoArc,
};

use crate::document_runtime::DomHandle;

use super::{StyloDocumentComputedStyleInputCacheKey, StyloPreparedComputedStyleInputs};

const DOCUMENT_COMPUTED_STYLE_INPUT_CACHE_LIMIT: usize = 2;

/// Generations that can change the immutable inputs and retained-system key
/// supplied to Stylo.
///
/// The retained-style-system generation is deliberately excluded: resolving
/// the first element from freshly built inputs can create that system. Inputs
/// are published after the observation, against the resulting canonical
/// source/computed/target generations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ComputedStyleInputCacheGeneration {
    pub(super) source_set: u64,
    pub(super) computed_values: u64,
    pub(super) target_context: u64,
}

pub(super) struct ComputedStyleInputCache {
    generation: Cell<Option<ComputedStyleInputCacheGeneration>>,
    entries: RefCell<
        Vec<(
            StyloDocumentComputedStyleInputCacheKey,
            Rc<StyloPreparedComputedStyleInputs>,
        )>,
    >,
}

impl ComputedStyleInputCache {
    pub(super) fn new() -> Self {
        Self {
            generation: Cell::new(None),
            entries: RefCell::new(Vec::new()),
        }
    }

    pub(super) fn clear(&self) {
        self.generation.set(None);
        self.entries.borrow_mut().clear();
    }

    pub(super) fn get(
        &self,
        generation: ComputedStyleInputCacheGeneration,
        key: &StyloDocumentComputedStyleInputCacheKey,
    ) -> Option<Rc<StyloPreparedComputedStyleInputs>> {
        self.synchronize_generation(generation);
        self.entries
            .borrow()
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, inputs)| Rc::clone(inputs))
    }

    pub(super) fn insert(
        &self,
        generation: ComputedStyleInputCacheGeneration,
        key: StyloDocumentComputedStyleInputCacheKey,
        inputs: Rc<StyloPreparedComputedStyleInputs>,
    ) {
        self.synchronize_generation(generation);
        let mut entries = self.entries.borrow_mut();
        if let Some((_, cached_inputs)) =
            entries.iter_mut().find(|(candidate, _)| candidate == &key)
        {
            *cached_inputs = inputs;
            return;
        }
        if entries.len() == DOCUMENT_COMPUTED_STYLE_INPUT_CACHE_LIMIT {
            entries.remove(0);
        }
        entries.push((key, inputs));
    }

    fn synchronize_generation(&self, generation: ComputedStyleInputCacheGeneration) {
        if self.generation.get() == Some(generation) {
            return;
        }
        self.entries.borrow_mut().clear();
        self.generation.set(Some(generation));
    }
}

pub(super) struct ComputedStyleCache {
    entries: RefCell<HashMap<ComputedElementStyleCacheKey, ElementStyles>>,
    lazy_pseudo_entries: RefCell<HashMap<ComputedElementStyleCacheKey, ServoArc<ComputedValues>>>,
    keys_by_handle: RefCell<HashMap<DomHandle, HashSet<ComputedElementStyleCacheKey>>>,
    write_generation: Cell<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct ComputedElementStyleCacheKey {
    pub(super) computed_cache_generation: u64,
    pub(super) handle: DomHandle,
    pub(super) pseudo_element: Option<PseudoElement>,
}

impl ComputedStyleCache {
    pub(super) fn new() -> Self {
        Self {
            entries: RefCell::new(HashMap::new()),
            lazy_pseudo_entries: RefCell::new(HashMap::new()),
            keys_by_handle: RefCell::new(HashMap::new()),
            write_generation: Cell::new(0),
        }
    }

    pub(super) fn clear(&self) {
        self.entries.borrow_mut().clear();
        self.lazy_pseudo_entries.borrow_mut().clear();
        self.keys_by_handle.borrow_mut().clear();
    }

    pub(super) fn get(&self, key: &ComputedElementStyleCacheKey) -> Option<ElementStyles> {
        self.entries.borrow().get(key).cloned()
    }

    pub(super) fn insert(&self, key: ComputedElementStyleCacheKey, styles: ElementStyles) {
        self.index_key(&key);
        self.entries.borrow_mut().insert(key, styles);
        self.bump_write_generation();
    }

    pub(super) fn get_lazy_pseudo(
        &self,
        key: &ComputedElementStyleCacheKey,
    ) -> Option<ServoArc<ComputedValues>> {
        self.lazy_pseudo_entries.borrow().get(key).cloned()
    }

    pub(super) fn insert_lazy_pseudo(
        &self,
        key: ComputedElementStyleCacheKey,
        style: ServoArc<ComputedValues>,
    ) {
        self.index_key(&key);
        self.lazy_pseudo_entries.borrow_mut().insert(key, style);
        self.bump_write_generation();
    }

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
        let mut entries = self.entries.borrow_mut();
        let mut lazy_pseudo_entries = self.lazy_pseudo_entries.borrow_mut();
        for key in keys {
            entries.remove(&key);
            lazy_pseudo_entries.remove(&key);
        }
    }

    pub(super) fn handles(&self) -> Vec<DomHandle> {
        self.keys_by_handle.borrow().keys().copied().collect()
    }

    fn index_key(&self, key: &ComputedElementStyleCacheKey) {
        self.keys_by_handle
            .borrow_mut()
            .entry(key.handle)
            .or_default()
            .insert(key.clone());
    }

    fn bump_write_generation(&self) {
        self.write_generation
            .set(self.write_generation.get().saturating_add(1));
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.borrow().is_empty()
            && self.lazy_pseudo_entries.borrow().is_empty()
            && self.keys_by_handle.borrow().is_empty()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.borrow().len() + self.lazy_pseudo_entries.borrow().len()
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

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::{
        ComputedStyleInputCache, ComputedStyleInputCacheGeneration,
        StyloDocumentComputedStyleInputCacheKey, StyloPreparedComputedStyleInputs,
    };
    use crate::style_engine::StyloComputedStyleInputs;
    use crate::style_engine::{StyleViewport, StyloStyleEnvironment};

    fn key(path: &str) -> StyloDocumentComputedStyleInputCacheKey {
        StyloDocumentComputedStyleInputCacheKey::new(
            None,
            &[],
            &url::Url::parse(&format!("https://document.test/{path}"))
                .expect("cache test document URL should parse"),
            StyleViewport::default(),
            StyloStyleEnvironment::default(),
            &url::Url::parse(&format!("https://cache.test/{path}"))
                .expect("cache test URL should parse"),
        )
    }

    #[test]
    fn computed_style_input_cache_is_bounded_and_generation_scoped() {
        let cache = ComputedStyleInputCache::new();
        let generation = ComputedStyleInputCacheGeneration {
            source_set: 1,
            computed_values: 2,
            target_context: 3,
        };
        let first_key = key("first");
        let second_key = key("second");
        let third_key = key("third");
        let document_url = url::Url::parse("https://cache.test/document")
            .expect("cache test document URL should parse");
        let prepared = || {
            Rc::new(StyloPreparedComputedStyleInputs::new(
                &document_url,
                Rc::new(StyloComputedStyleInputs::default()),
                StyleViewport::default(),
            ))
        };
        let first = prepared();
        let second = prepared();
        let third = prepared();

        cache.insert(generation, first_key.clone(), Rc::clone(&first));
        cache.insert(generation, second_key.clone(), Rc::clone(&second));
        assert!(Rc::ptr_eq(
            &cache
                .get(generation, &first_key)
                .expect("first cache entry should exist"),
            &first
        ));

        cache.insert(generation, third_key.clone(), Rc::clone(&third));
        assert!(cache.get(generation, &first_key).is_none());
        assert!(Rc::ptr_eq(
            &cache
                .get(generation, &second_key)
                .expect("second cache entry should remain after bounded eviction"),
            &second
        ));
        assert!(Rc::ptr_eq(
            &cache
                .get(generation, &third_key)
                .expect("third cache entry should exist"),
            &third
        ));

        let next_generation = ComputedStyleInputCacheGeneration {
            target_context: 4,
            ..generation
        };
        assert!(cache.get(next_generation, &second_key).is_none());
        assert!(cache.get(next_generation, &third_key).is_none());

        cache.insert(next_generation, third_key.clone(), Rc::clone(&third));
        cache.clear();
        assert!(cache.get(next_generation, &third_key).is_none());
    }
}
