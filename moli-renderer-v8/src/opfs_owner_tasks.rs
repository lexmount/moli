use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use moli_storage_service::{DirectoryEntry, OpfsPath};
use parking_lot::Mutex;

/// Execution context plus the storage namespace authorized for an OPFS wrapper.
///
/// Window wrappers retain an exact realm identity so navigation retires them.
/// The authorized storage key normally equals the realm's ambient key, but can
/// differ for a Storage Access handle or any same-origin runtime clone that
/// preserves its original backing locator. Worker wrappers use no Window
/// identity and are tied to the receiving Worker realm by its registry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpfsHandleAccessContext {
    window_identity: Option<crate::native_bridge::WindowExecutionContextIdentity>,
    storage_key: String,
}

impl OpfsHandleAccessContext {
    pub(crate) fn window(
        window_identity: crate::native_bridge::WindowExecutionContextIdentity,
        storage_key: String,
    ) -> Self {
        Self {
            window_identity: Some(window_identity),
            storage_key,
        }
    }

    pub(crate) fn worker(storage_key: String) -> Self {
        Self {
            window_identity: None,
            storage_key,
        }
    }

    pub(crate) fn window_identity(
        &self,
    ) -> Option<crate::native_bridge::WindowExecutionContextIdentity> {
        self.window_identity
    }

    pub(crate) fn storage_key(&self) -> &str {
        &self.storage_key
    }
}

#[derive(Clone)]
pub(crate) struct OpfsHandlePathState {
    path: Arc<Mutex<OpfsPath>>,
    mutation_pending: Arc<AtomicBool>,
}

impl OpfsHandlePathState {
    pub(crate) fn current(&self) -> OpfsPath {
        self.path.lock().clone()
    }

    pub(crate) fn replace(&self, path: OpfsPath) {
        *self.path.lock() = path;
    }

    pub(crate) fn try_begin_mutation(&self) -> Option<OpfsHandleMutationGuard> {
        self.mutation_pending
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| OpfsHandleMutationGuard {
                pending: self.mutation_pending.clone(),
            })
    }
}

pub(crate) struct OpfsHandleMutationGuard {
    pending: Arc<AtomicBool>,
}

impl Drop for OpfsHandleMutationGuard {
    fn drop(&mut self) {
        let was_pending = self.pending.swap(false, Ordering::Release);
        debug_assert!(was_pending, "OPFS handle mutation guard released twice");
    }
}

#[derive(Clone, Default)]
pub(crate) struct OpfsHandleRegistry {
    inner: Rc<RefCell<OpfsHandleRegistryInner>>,
}

#[derive(Default)]
struct OpfsHandleRegistryInner {
    next_handle_id: u32,
    next_derived_access_id: u32,
    handles: HashMap<u32, OpfsHandleRegistryEntry>,
    derived_accesses: HashMap<u32, OpfsHandleAccessContext>,
}

struct OpfsHandleRegistryEntry {
    path: OpfsHandlePathState,
    handle_access: Option<OpfsHandleAccessContext>,
}

impl OpfsHandleRegistry {
    pub(crate) fn insert(
        &self,
        path: OpfsPath,
        handle_access: Option<OpfsHandleAccessContext>,
    ) -> u32 {
        let mut inner = self.inner.borrow_mut();
        let handle_id = next_unused_handle_id(&mut inner);
        inner.handles.insert(
            handle_id,
            OpfsHandleRegistryEntry {
                path: OpfsHandlePathState {
                    path: Arc::new(Mutex::new(path)),
                    mutation_pending: Arc::new(AtomicBool::new(false)),
                },
                handle_access,
            },
        );
        handle_id
    }

    pub(crate) fn remove(&self, handle_id: u32) {
        self.inner.borrow_mut().handles.remove(&handle_id);
    }

    pub(crate) fn path_state(&self, handle_id: u32) -> Option<OpfsHandlePathState> {
        self.inner
            .borrow()
            .handles
            .get(&handle_id)
            .map(|entry| entry.path.clone())
    }

    pub(crate) fn handle_access(&self, handle_id: u32) -> Option<OpfsHandleAccessContext> {
        self.inner
            .borrow()
            .handles
            .get(&handle_id)
            .and_then(|entry| entry.handle_access.clone())
    }

    pub(crate) fn insert_derived_access(&self, access: OpfsHandleAccessContext) -> u32 {
        let mut inner = self.inner.borrow_mut();
        let access_id = next_unused_derived_access_id(&mut inner);
        inner.derived_accesses.insert(access_id, access);
        access_id
    }

    pub(crate) fn remove_derived_access(&self, access_id: u32) {
        self.inner.borrow_mut().derived_accesses.remove(&access_id);
    }

    pub(crate) fn derived_access(&self, access_id: u32) -> Option<OpfsHandleAccessContext> {
        self.inner
            .borrow()
            .derived_accesses
            .get(&access_id)
            .cloned()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.inner.borrow().handles.len()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OpfsDirectoryIteratorDescriptor {
    pub(crate) state_json: String,
    pub(crate) mode: String,
    pub(crate) handle_access: Option<OpfsHandleAccessContext>,
}

pub(crate) struct OpfsDirectoryIteratorSettlement {
    pub(crate) descriptor: OpfsDirectoryIteratorDescriptor,
    pub(crate) resolver: v8::Global<v8::PromiseResolver>,
    pub(crate) entry: Option<DirectoryEntry>,
}

pub(crate) enum OpfsDirectoryIteratorNextAction {
    StartLoad,
    Queued,
    Cached {
        descriptor: OpfsDirectoryIteratorDescriptor,
        entry: Option<DirectoryEntry>,
    },
}

pub(crate) enum OpfsTaskSettlement {
    Promise {
        resolver: v8::Global<v8::PromiseResolver>,
        // Source writes can wait for input or quota. Retiring the pending task
        // must abort its temporary writer without waiting for either.
        writable_cleanup: Option<moli_storage_service::StorageOpfsWritableLease>,
    },
    Move {
        resolver: v8::Global<v8::PromiseResolver>,
        handle: v8::Global<v8::Object>,
        mutation: OpfsHandleMutationGuard,
    },
    DirectoryIterator {
        registry: OpfsDirectoryIteratorRegistry,
        iterator_id: u32,
        keep_alive: v8::Global<v8::Object>,
    },
}

fn next_unused_handle_id(inner: &mut OpfsHandleRegistryInner) -> u32 {
    for _ in 0..=u32::MAX {
        let candidate = inner.next_handle_id.max(1);
        inner.next_handle_id = candidate.wrapping_add(1).max(1);
        if !inner.handles.contains_key(&candidate) {
            return candidate;
        }
    }
    panic!("OPFS handle id space exhausted");
}

fn next_unused_derived_access_id(inner: &mut OpfsHandleRegistryInner) -> u32 {
    for _ in 0..=u32::MAX {
        let candidate = inner.next_derived_access_id.max(1);
        inner.next_derived_access_id = candidate.wrapping_add(1).max(1);
        if !inner.derived_accesses.contains_key(&candidate) {
            return candidate;
        }
    }
    panic!("OPFS derived access id space exhausted");
}

#[derive(Clone, Default)]
pub(crate) struct OpfsDirectoryIteratorRegistry {
    inner: Rc<RefCell<OpfsDirectoryIteratorRegistryInner>>,
}

#[derive(Default)]
struct OpfsDirectoryIteratorRegistryInner {
    next_iterator_id: u32,
    iterators: HashMap<u32, OpfsDirectoryIteratorState>,
}

struct OpfsDirectoryIteratorState {
    descriptor: OpfsDirectoryIteratorDescriptor,
    phase: OpfsDirectoryIteratorPhase,
    pending: VecDeque<v8::Global<v8::PromiseResolver>>,
}

enum OpfsDirectoryIteratorPhase {
    Initial,
    Loading,
    Ready {
        entries: Vec<DirectoryEntry>,
        cursor: usize,
    },
    Done,
}

impl OpfsDirectoryIteratorRegistry {
    pub(crate) fn insert(&self, descriptor: OpfsDirectoryIteratorDescriptor) -> u32 {
        let mut inner = self.inner.borrow_mut();
        let iterator_id = next_unused_iterator_id(&mut inner);
        inner.iterators.insert(
            iterator_id,
            OpfsDirectoryIteratorState {
                descriptor,
                phase: OpfsDirectoryIteratorPhase::Initial,
                pending: VecDeque::new(),
            },
        );
        iterator_id
    }

    pub(crate) fn remove(&self, iterator_id: u32) {
        self.inner.borrow_mut().iterators.remove(&iterator_id);
    }

    pub(crate) fn descriptor(&self, iterator_id: u32) -> Option<OpfsDirectoryIteratorDescriptor> {
        self.inner
            .borrow()
            .iterators
            .get(&iterator_id)
            .map(|state| state.descriptor.clone())
    }

    pub(crate) fn enqueue_next(
        &self,
        iterator_id: u32,
        resolver: v8::Global<v8::PromiseResolver>,
    ) -> Option<OpfsDirectoryIteratorNextAction> {
        let mut inner = self.inner.borrow_mut();
        let state = inner.iterators.get_mut(&iterator_id)?;
        let phase = std::mem::replace(&mut state.phase, OpfsDirectoryIteratorPhase::Done);
        let action = match phase {
            OpfsDirectoryIteratorPhase::Initial => {
                state.pending.push_back(resolver);
                state.phase = OpfsDirectoryIteratorPhase::Loading;
                OpfsDirectoryIteratorNextAction::StartLoad
            }
            OpfsDirectoryIteratorPhase::Loading => {
                state.pending.push_back(resolver);
                state.phase = OpfsDirectoryIteratorPhase::Loading;
                OpfsDirectoryIteratorNextAction::Queued
            }
            OpfsDirectoryIteratorPhase::Ready {
                entries,
                mut cursor,
            } => {
                let entry = entries.get(cursor).cloned();
                if entry.is_some() {
                    cursor += 1;
                    state.phase = OpfsDirectoryIteratorPhase::Ready { entries, cursor };
                } else {
                    state.phase = OpfsDirectoryIteratorPhase::Done;
                }
                OpfsDirectoryIteratorNextAction::Cached {
                    descriptor: state.descriptor.clone(),
                    entry,
                }
            }
            OpfsDirectoryIteratorPhase::Done => {
                state.phase = OpfsDirectoryIteratorPhase::Done;
                OpfsDirectoryIteratorNextAction::Cached {
                    descriptor: state.descriptor.clone(),
                    entry: None,
                }
            }
        };
        Some(action)
    }

    pub(crate) fn complete_load(
        &self,
        iterator_id: u32,
        entries: Vec<DirectoryEntry>,
    ) -> Vec<OpfsDirectoryIteratorSettlement> {
        let mut inner = self.inner.borrow_mut();
        let Some(state) = inner.iterators.get_mut(&iterator_id) else {
            return Vec::new();
        };
        if !matches!(state.phase, OpfsDirectoryIteratorPhase::Loading) {
            return Vec::new();
        }

        let mut cursor = 0;
        let mut settlements = Vec::with_capacity(state.pending.len());
        while let Some(resolver) = state.pending.pop_front() {
            let entry = entries.get(cursor).cloned();
            if entry.is_some() {
                cursor += 1;
            }
            settlements.push(OpfsDirectoryIteratorSettlement {
                descriptor: state.descriptor.clone(),
                resolver,
                entry,
            });
        }
        state.phase = if cursor < entries.len() {
            OpfsDirectoryIteratorPhase::Ready { entries, cursor }
        } else {
            OpfsDirectoryIteratorPhase::Done
        };
        settlements
    }

    pub(crate) fn fail_load(&self, iterator_id: u32) -> Vec<v8::Global<v8::PromiseResolver>> {
        let mut inner = self.inner.borrow_mut();
        let Some(state) = inner.iterators.get_mut(&iterator_id) else {
            return Vec::new();
        };
        state.phase = OpfsDirectoryIteratorPhase::Done;
        state.pending.drain(..).collect()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.inner.borrow().iterators.len()
    }
}

fn next_unused_iterator_id(inner: &mut OpfsDirectoryIteratorRegistryInner) -> u32 {
    for _ in 0..=u32::MAX {
        let candidate = inner.next_iterator_id.max(1);
        inner.next_iterator_id = candidate.wrapping_add(1).max(1);
        if !inner.iterators.contains_key(&candidate) {
            return candidate;
        }
    }
    panic!("OPFS directory iterator id space exhausted");
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_storage_service::EntryKind;

    #[test]
    fn handle_registry_shares_one_wrapper_path_without_retargeting_other_wrappers() {
        let original = OpfsPath::from_components(vec!["before.txt".to_owned()]).unwrap();
        let moved = OpfsPath::from_components(vec!["directory".to_owned(), "after.txt".to_owned()])
            .unwrap();
        let registry = OpfsHandleRegistry::default();
        let first = registry.insert(original.clone(), None);
        let second = registry.insert(original.clone(), None);
        let first_path = registry.path_state(first).unwrap();
        let first_path_clone = first_path.clone();
        let second_path = registry.path_state(second).unwrap();

        first_path.replace(moved.clone());

        assert_eq!(first_path_clone.current(), moved);
        assert_eq!(second_path.current(), original);
        assert_eq!(registry.len(), 2);
        registry.remove(first);
        assert!(registry.path_state(first).is_none());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn handle_registry_owns_derived_capability_access_until_explicit_cleanup() {
        let registry = OpfsHandleRegistry::default();
        let access = OpfsHandleAccessContext::worker("authorized-storage-key".to_owned());
        let access_id = registry.insert_derived_access(access.clone());

        assert_eq!(registry.derived_access(access_id), Some(access));
        registry.remove_derived_access(access_id);
        assert_eq!(registry.derived_access(access_id), None);
    }

    #[test]
    fn handle_mutation_guard_rejects_overlap_and_releases_on_drop() {
        let path = OpfsPath::from_components(vec!["before.txt".to_owned()]).unwrap();
        let registry = OpfsHandleRegistry::default();
        let handle_id = registry.insert(path, None);
        let path = registry.path_state(handle_id).unwrap();

        let first = path
            .try_begin_mutation()
            .expect("the first mutation should acquire the wrapper guard");
        assert!(
            path.try_begin_mutation().is_none(),
            "a second mutation on the same wrapper must be rejected while pending"
        );

        drop(first);
        assert!(
            path.try_begin_mutation().is_some(),
            "settlement must release the wrapper guard for a later mutation"
        );
    }

    #[test]
    fn concurrent_next_resolvers_share_one_load_and_settle_fifo_from_cache() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let registry = OpfsDirectoryIteratorRegistry::default();
        let iterator_id = registry.insert(OpfsDirectoryIteratorDescriptor {
            state_json: "state".to_owned(),
            mode: "keys".to_owned(),
            handle_access: None,
        });
        assert_eq!(registry.len(), 1);

        let first = v8::PromiseResolver::new(scope).unwrap();
        assert!(matches!(
            registry.enqueue_next(iterator_id, v8::Global::new(scope, first)),
            Some(OpfsDirectoryIteratorNextAction::StartLoad)
        ));
        let second = v8::PromiseResolver::new(scope).unwrap();
        assert!(matches!(
            registry.enqueue_next(iterator_id, v8::Global::new(scope, second)),
            Some(OpfsDirectoryIteratorNextAction::Queued)
        ));

        let settlements = registry.complete_load(
            iterator_id,
            vec![
                directory_entry("a"),
                directory_entry("b"),
                directory_entry("c"),
            ],
        );
        assert_eq!(settlements.len(), 2);
        assert_eq!(settlements[0].entry.as_ref().unwrap().name, "a");
        assert_eq!(settlements[1].entry.as_ref().unwrap().name, "b");

        let third = v8::PromiseResolver::new(scope).unwrap();
        let Some(OpfsDirectoryIteratorNextAction::Cached { entry, .. }) =
            registry.enqueue_next(iterator_id, v8::Global::new(scope, third))
        else {
            panic!("third next should use the cached directory read");
        };
        assert_eq!(entry.unwrap().name, "c");

        let done = v8::PromiseResolver::new(scope).unwrap();
        let Some(OpfsDirectoryIteratorNextAction::Cached { entry, .. }) =
            registry.enqueue_next(iterator_id, v8::Global::new(scope, done))
        else {
            panic!("exhausted iterator should settle from its done state");
        };
        assert!(entry.is_none());

        registry.remove(iterator_id);
        assert_eq!(registry.len(), 0);
    }

    fn directory_entry(name: &str) -> DirectoryEntry {
        DirectoryEntry {
            name: name.to_owned(),
            kind: EntryKind::File,
        }
    }
}
