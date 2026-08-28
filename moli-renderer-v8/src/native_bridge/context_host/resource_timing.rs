use super::JsContextHost;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::{Rc, Weak},
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ResourceTimingBufferId(u64);

impl ResourceTimingBufferId {
    pub(crate) fn from_raw(raw: u64) -> Option<Self> {
        (raw != 0).then_some(Self(raw))
    }

    pub(crate) fn raw(self) -> u64 {
        self.0
    }
}

struct ResourceTimingBufferState {
    owner_realm: Option<super::RuntimeObservableContextToken>,
    size_limit: u32,
    current_size: u32,
    secondary: VecDeque<v8::Global<v8::Object>>,
    full_task_pending: bool,
}

impl ResourceTimingBufferState {
    fn new(owner_realm: Option<super::RuntimeObservableContextToken>, size_limit: u32) -> Self {
        Self {
            owner_realm,
            size_limit,
            current_size: 0,
            secondary: VecDeque::new(),
            full_task_pending: false,
        }
    }

    fn can_add_to_primary(&self) -> bool {
        self.current_size < self.size_limit
    }

    fn can_add_immediately(&self) -> bool {
        self.can_add_to_primary() && !self.full_task_pending
    }
}

#[derive(Default)]
struct ResourceTimingBufferRegistry {
    next_id: u64,
    buffers: HashMap<ResourceTimingBufferId, ResourceTimingBufferState>,
}

impl ResourceTimingBufferRegistry {
    fn insert(
        &mut self,
        owner_realm: Option<super::RuntimeObservableContextToken>,
        size_limit: u32,
    ) -> ResourceTimingBufferId {
        loop {
            self.next_id = self
                .next_id
                .checked_add(1)
                .expect("resource timing entry id space exhausted");
            let id = ResourceTimingBufferId(self.next_id);
            if let std::collections::hash_map::Entry::Vacant(entry) = self.buffers.entry(id) {
                entry.insert(ResourceTimingBufferState::new(owner_realm, size_limit));
                return id;
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct SharedResourceTimingBufferRegistry(Rc<RefCell<ResourceTimingBufferRegistry>>);

impl SharedResourceTimingBufferRegistry {
    pub(super) fn new() -> Self {
        Self(Rc::new(RefCell::new(
            ResourceTimingBufferRegistry::default(),
        )))
    }
}

pub(crate) struct ResourceTimingBufferFinalizer {
    registry: Weak<RefCell<ResourceTimingBufferRegistry>>,
    id: ResourceTimingBufferId,
}

impl ResourceTimingBufferFinalizer {
    pub(crate) fn finalize(self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.borrow_mut().buffers.remove(&self.id);
        }
    }
}

impl JsContextHost {
    pub(crate) fn create_resource_timing_buffer(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        size_limit: u32,
    ) -> (ResourceTimingBufferId, ResourceTimingBufferFinalizer) {
        let owner_realm = super::current_runtime_observable_context_token(scope);
        let id = self
            .resource_timing_buffers
            .0
            .borrow_mut()
            .insert(owner_realm, size_limit);
        let finalizer = ResourceTimingBufferFinalizer {
            registry: Rc::downgrade(&self.resource_timing_buffers.0),
            id,
        };
        (id, finalizer)
    }

    pub(crate) fn retire_resource_timing_buffers_for_context_token(
        &mut self,
        context_token: super::RuntimeObservableContextToken,
    ) -> usize {
        let mut registry = self.resource_timing_buffers.0.borrow_mut();
        let before = registry.buffers.len();
        registry
            .buffers
            .retain(|_, state| state.owner_realm != Some(context_token));
        before.saturating_sub(registry.buffers.len())
    }

    #[cfg(test)]
    pub(crate) fn resource_timing_buffer_count_for_context_token_for_test(
        &self,
        context_token: super::RuntimeObservableContextToken,
    ) -> usize {
        self.resource_timing_buffers
            .0
            .borrow()
            .buffers
            .values()
            .filter(|state| state.owner_realm == Some(context_token))
            .count()
    }

    pub(crate) fn set_resource_timing_buffer_size_limit(
        &self,
        id: ResourceTimingBufferId,
        size_limit: u32,
    ) {
        if let Some(state) = self
            .resource_timing_buffers
            .0
            .borrow_mut()
            .buffers
            .get_mut(&id)
        {
            state.size_limit = size_limit;
        }
    }

    pub(crate) fn resource_timing_buffer_can_add_immediately(
        &self,
        id: ResourceTimingBufferId,
    ) -> bool {
        self.resource_timing_buffers
            .0
            .borrow()
            .buffers
            .get(&id)
            .is_some_and(ResourceTimingBufferState::can_add_immediately)
    }

    pub(crate) fn resource_timing_buffer_can_add_to_primary(
        &self,
        id: ResourceTimingBufferId,
    ) -> bool {
        self.resource_timing_buffers
            .0
            .borrow()
            .buffers
            .get(&id)
            .is_some_and(ResourceTimingBufferState::can_add_to_primary)
    }

    pub(crate) fn note_resource_timing_primary_entry_added(&self, id: ResourceTimingBufferId) {
        if let Some(state) = self
            .resource_timing_buffers
            .0
            .borrow_mut()
            .buffers
            .get_mut(&id)
        {
            state.current_size = state.current_size.saturating_add(1);
        }
    }

    pub(crate) fn clear_resource_timing_primary_buffer(&self, id: ResourceTimingBufferId) {
        if let Some(state) = self
            .resource_timing_buffers
            .0
            .borrow_mut()
            .buffers
            .get_mut(&id)
        {
            state.current_size = 0;
        }
    }

    pub(crate) fn mark_resource_timing_buffer_full_task_pending(
        &self,
        id: ResourceTimingBufferId,
    ) -> bool {
        let mut registry = self.resource_timing_buffers.0.borrow_mut();
        let Some(state) = registry.buffers.get_mut(&id) else {
            return false;
        };
        if state.full_task_pending {
            return false;
        }
        state.full_task_pending = true;
        true
    }

    pub(crate) fn finish_resource_timing_buffer_full_task(&self, id: ResourceTimingBufferId) {
        if let Some(state) = self
            .resource_timing_buffers
            .0
            .borrow_mut()
            .buffers
            .get_mut(&id)
        {
            state.full_task_pending = false;
        }
    }

    pub(crate) fn push_secondary_resource_timing_entry(
        &self,
        id: ResourceTimingBufferId,
        entry: v8::Global<v8::Object>,
    ) {
        if let Some(state) = self
            .resource_timing_buffers
            .0
            .borrow_mut()
            .buffers
            .get_mut(&id)
        {
            state.secondary.push_back(entry);
        }
    }

    pub(crate) fn pop_secondary_resource_timing_entry(
        &self,
        id: ResourceTimingBufferId,
    ) -> Option<v8::Global<v8::Object>> {
        self.resource_timing_buffers
            .0
            .borrow_mut()
            .buffers
            .get_mut(&id)?
            .secondary
            .pop_front()
    }

    pub(crate) fn secondary_resource_timing_buffer_len(&self, id: ResourceTimingBufferId) -> usize {
        self.resource_timing_buffers
            .0
            .borrow()
            .buffers
            .get(&id)
            .map(|state| state.secondary.len())
            .unwrap_or(0)
    }

    pub(crate) fn clear_secondary_resource_timing_buffer(&self, id: ResourceTimingBufferId) {
        if let Some(state) = self
            .resource_timing_buffers
            .0
            .borrow_mut()
            .buffers
            .get_mut(&id)
        {
            state.secondary.clear();
        }
    }

    #[cfg(test)]
    pub(crate) fn resource_timing_buffer_count_for_test(&self) -> usize {
        self.resource_timing_buffers.0.borrow().buffers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_buffer_state_keeps_pending_and_capacity_orthogonal() {
        let mut state = ResourceTimingBufferState::new(None, 1);
        assert!(state.can_add_immediately());

        state.full_task_pending = true;
        assert!(!state.can_add_immediately());
        assert!(state.can_add_to_primary());

        state.current_size = 1;
        assert!(!state.can_add_to_primary());
        state.current_size = 0;
        assert!(state.can_add_to_primary());
    }

    #[test]
    fn registry_ids_are_nonzero_and_finalizer_removes_state() {
        let shared = SharedResourceTimingBufferRegistry::new();
        let id = shared.0.borrow_mut().insert(None, 250);
        assert_ne!(id.raw(), 0);
        let finalizer = ResourceTimingBufferFinalizer {
            registry: Rc::downgrade(&shared.0),
            id,
        };

        finalizer.finalize();
        assert!(!shared.0.borrow().buffers.contains_key(&id));
    }
}
