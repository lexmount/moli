use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
};

use indexmap::IndexSet;

use crate::{document_runtime::DomHandle, dom::native::DomHost};

/// Bounds sparse invalidation-root history without requiring a scan of
/// published element styles to retire individual generations.
const MAX_RETAINED_INVALIDATION_ROOTS: usize = 1_024;

/// One element on the style-parent path to a demanded target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StyleValidationPathEntry {
    pub(super) element: DomHandle,
    pub(super) required_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StylePathStamp {
    registry_generation: u64,
    required_generation: u64,
}

/// Document-local retained roots for demand-driven style invalidation.
///
/// An invalidation root is stamped with the mutation generation that created
/// it. Published element styles carry the newest generation against which they
/// were resolved. Reads walk only their own ancestor path, accumulate the
/// newest covering root, and recompute stale entries from parent to child.
/// This is Moli's side-table equivalent of Blink's per-node dirty flags and
/// `ChildNeedsStyleRecalc` breadcrumbs; it preserves Moli's on-demand style
/// model without an eager scan over every published `ElementData` entry.
#[derive(Default)]
pub(super) struct LazyStyleInvalidationRoots {
    generation: Cell<u64>,
    document_floor_generation: Cell<u64>,
    roots: RefCell<HashMap<DomHandle, u64>>,
    /// Memoized generation inherited from the nearest invalidation root.
    ///
    /// A new mutation generation invalidates these stamps logically, without
    /// walking the table. While an observation proceeds parent-before-child,
    /// each next lookup normally reaches an already stamped parent after one
    /// step. A whole tree observation is therefore linear rather than
    /// `elements * depth`.
    path_stamps: RefCell<HashMap<DomHandle, StylePathStamp>>,
    validated_elements: RefCell<HashMap<DomHandle, u64>>,
    #[cfg(test)]
    path_node_visits: Cell<u64>,
}

impl LazyStyleInvalidationRoots {
    pub(super) fn record_roots(
        &self,
        host: &DomHost,
        document: DomHandle,
        roots: impl IntoIterator<Item = DomHandle>,
    ) -> IndexSet<DomHandle> {
        let roots = roots
            .into_iter()
            .filter(|root| {
                host.node(*root).is_some()
                    && (*root == document || host.owner_document_handle(*root) == Some(document))
            })
            .collect::<IndexSet<_>>();
        if roots.is_empty() {
            return roots;
        }

        let generation = self.next_generation();
        self.generation.set(generation);
        let mut retained = self.roots.borrow_mut();
        for root in roots.iter().copied() {
            retained.insert(root, generation);
        }

        if retained.len() > MAX_RETAINED_INVALIDATION_ROOTS {
            self.document_floor_generation.set(generation);
            retained.clear();
        }
        roots
    }

    /// Returns whether this document has retained invalidation-root history.
    ///
    /// History remains present after individual elements consume it because
    /// there is intentionally no global "all descendants are current" scan.
    /// Current-generation path stamps make clean repeated reads O(1).
    pub(super) fn has_retained_roots(&self) -> bool {
        self.generation.get() != 0
    }

    pub(super) fn validation_path(
        &self,
        host: &DomHost,
        document: DomHandle,
        target: DomHandle,
    ) -> Vec<StyleValidationPathEntry> {
        if host.owner_document_handle(target) != Some(document) {
            return Vec::new();
        }

        let registry_generation = self.generation.get();
        let floor_generation = self.document_floor_generation.get();
        let roots = self.roots.borrow();
        let mut path_stamps = self.path_stamps.borrow_mut();
        let mut unstamped_path = Vec::new();
        let mut seen = HashSet::new();
        let mut current = Some(target);
        let mut required_generation = floor_generation;
        while let Some(handle) = current.filter(|handle| seen.insert(*handle)) {
            self.note_path_node_visit();
            if let Some(stamp) = path_stamps
                .get(&handle)
                .filter(|stamp| stamp.registry_generation == registry_generation)
            {
                required_generation = required_generation.max(stamp.required_generation);
                break;
            }
            unstamped_path.push(handle);
            current = host
                .parent_node(handle)
                .or_else(|| host.shadow_root_host(handle));
        }

        unstamped_path.reverse();
        let mut entries = unstamped_path
            .into_iter()
            .filter_map(|handle| {
                if let Some(root_generation) = roots.get(&handle) {
                    required_generation = required_generation.max(*root_generation);
                }
                path_stamps.insert(
                    handle,
                    StylePathStamp {
                        registry_generation,
                        required_generation,
                    },
                );
                host.node(handle)
                    .and_then(|node| node.as_element())
                    .map(|_| StyleValidationPathEntry {
                        element: handle,
                        required_generation,
                    })
            })
            .collect::<Vec<_>>();

        // A fully memoized target still needs one entry so callers can check
        // the canonical ElementData against its inherited requirement in O(1).
        if entries.is_empty()
            && let Some(stamp) = path_stamps
                .get(&target)
                .filter(|stamp| stamp.registry_generation == registry_generation)
            && host
                .node(target)
                .and_then(|node| node.as_element())
                .is_some()
        {
            entries.push(StyleValidationPathEntry {
                element: target,
                required_generation: stamp.required_generation,
            });
        }
        entries
    }

    pub(super) fn required_generation_for_checked_element(
        &self,
        element: DomHandle,
    ) -> Option<u64> {
        let registry_generation = self.generation.get();
        self.path_stamps
            .borrow()
            .get(&element)
            .filter(|stamp| stamp.registry_generation == registry_generation)
            .map(|stamp| stamp.required_generation)
    }

    pub(super) fn element_is_current(&self, element: DomHandle, required_generation: u64) -> bool {
        required_generation == 0
            || self
                .validated_elements
                .borrow()
                .get(&element)
                .copied()
                .unwrap_or_default()
                >= required_generation
    }

    pub(super) fn mark_element_current(&self, element: DomHandle, generation: u64) {
        if generation == 0 {
            return;
        }
        let mut validated = self.validated_elements.borrow_mut();
        let entry = validated.entry(element).or_default();
        *entry = (*entry).max(generation);
    }

    pub(super) fn clear(&self) {
        self.generation.set(0);
        self.document_floor_generation.set(0);
        self.roots.borrow_mut().clear();
        self.path_stamps.borrow_mut().clear();
        self.validated_elements.borrow_mut().clear();
        #[cfg(test)]
        self.path_node_visits.set(0);
    }

    #[cfg(test)]
    pub(super) fn root_count(&self) -> usize {
        self.roots.borrow().len()
    }

    #[cfg(test)]
    pub(super) fn generation(&self) -> u64 {
        self.generation.get()
    }

    #[cfg(test)]
    pub(super) fn path_node_visit_count(&self) -> u64 {
        self.path_node_visits.get()
    }

    fn next_generation(&self) -> u64 {
        let Some(next) = self.generation.get().checked_add(1) else {
            // A wrapped generation could make an old validation stamp appear
            // current. Reset the sparse history and conservatively invalidate
            // the whole document instead.
            self.document_floor_generation.set(1);
            self.roots.borrow_mut().clear();
            self.path_stamps.borrow_mut().clear();
            self.validated_elements.borrow_mut().clear();
            return 1;
        };
        next
    }

    #[cfg(test)]
    fn note_path_node_visit(&self) {
        self.path_node_visits
            .set(self.path_node_visits.get().saturating_add(1));
    }

    #[cfg(not(test))]
    fn note_path_node_visit(&self) {}
}
