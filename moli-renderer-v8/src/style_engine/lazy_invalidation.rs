use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

use style::invalidation::element::restyle_hints::RestyleHint;

use crate::{document_runtime::DomHandle, dom::native::DomHost};

pub(super) const MAX_RETAINED_INVALIDATION_ROOTS: usize = 1_024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum StyleInvalidationScope {
    Element,
    Subtree,
}

#[derive(Clone, Copy, Default)]
struct InvalidationGenerations {
    element: u64,
    subtree: u64,
}

#[derive(Clone, Copy, Default)]
pub(super) struct DescendantRestyles {
    pub(super) subtree: u64,
    pub(super) children: u64,
    pub(super) inheriting_reset: u64,
}

impl DescendantRestyles {
    pub(super) fn child_requirement(self, inherits_reset: bool) -> u64 {
        let explicit_inherit = if inherits_reset {
            self.inheriting_reset
        } else {
            0
        };
        self.children.max(explicit_inherit)
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct CheckedStylePath {
    registry_generation: u64,
    resolved_generation: u64,
    pub(super) subtree_generation: u64,
    pub(super) depth: usize,
}

/// Both CSSOM and Realtime observe these selector targets and cascade results.
/// Each CSSOM read validates its own path. A screen publication acknowledges a
/// generation for the whole document once its pending targets are checked.
#[derive(Default)]
pub(super) struct LazyStyleInvalidationRoots {
    generation: Cell<u64>,
    document_floor_generation: Cell<u64>,
    roots: RefCell<HashMap<DomHandle, InvalidationGenerations>>,
    descendant_restyles: RefCell<HashMap<DomHandle, DescendantRestyles>>,
    checked_paths: RefCell<HashMap<DomHandle, CheckedStylePath>>,
    #[cfg(test)]
    path_node_visits: Cell<u64>,
}

impl LazyStyleInvalidationRoots {
    pub(super) fn record_targets(
        &self,
        host: &DomHost,
        document: DomHandle,
        targets: impl IntoIterator<Item = (DomHandle, StyleInvalidationScope)>,
    ) {
        let mut targets = targets.into_iter().filter(|(root, _)| {
            host.node(*root).is_some()
                && (*root == document || host.owner_document_handle(*root) == Some(document))
        });
        let Some(first) = targets.next() else {
            return;
        };
        let generation = self.next_generation();
        self.generation.set(generation);
        let mut retained = self.roots.borrow_mut();
        for (root, scope) in std::iter::once(first).chain(targets) {
            let entry = retained.entry(root).or_default();
            match scope {
                StyleInvalidationScope::Element => entry.element = generation,
                StyleInvalidationScope::Subtree => entry.subtree = generation,
            }
        }
        if retained.len() > MAX_RETAINED_INVALIDATION_ROOTS {
            self.document_floor_generation.set(generation);
            retained.clear();
            self.descendant_restyles.borrow_mut().clear();
        }
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation.get()
    }

    pub(super) fn forget_checked_path(&self, handle: DomHandle) -> bool {
        self.checked_paths.borrow_mut().remove(&handle).is_some()
    }

    pub(super) fn targets_after(
        &self,
        generation: u64,
    ) -> Option<Vec<(DomHandle, StyleInvalidationScope)>> {
        if generation < self.document_floor_generation.get() || generation > self.generation() {
            return None;
        }
        Some(
            self.roots
                .borrow()
                .iter()
                .filter_map(|(&handle, entry)| {
                    let scope = if entry.subtree > generation {
                        StyleInvalidationScope::Subtree
                    } else if entry.element > generation {
                        StyleInvalidationScope::Element
                    } else {
                        return None;
                    };
                    Some((handle, scope))
                })
                .collect(),
        )
    }

    pub(super) fn record_descendant_restyle(
        &self,
        handle: DomHandle,
        hint: RestyleHint,
        generation: u64,
        subtree_generation: u64,
    ) {
        if hint.is_empty() || generation <= subtree_generation {
            return;
        }
        let mut restyles = self.descendant_restyles.borrow_mut();
        let entry = restyles.entry(handle).or_default();
        if hint.will_recascade_subtree() {
            entry.subtree = entry.subtree.max(generation);
        } else if hint.contains(RestyleHint::RECASCADE_SELF) {
            entry.children = entry.children.max(generation);
        } else {
            debug_assert!(hint.contains(RestyleHint::RECASCADE_SELF_IF_INHERIT_RESET_STYLE));
            entry.inheriting_reset = entry.inheriting_reset.max(generation);
        }
    }

    pub(super) fn descendant_restyles(&self, handle: DomHandle) -> DescendantRestyles {
        self.descendant_restyles
            .borrow()
            .get(&handle)
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn subtree_generation(&self, handle: DomHandle) -> u64 {
        self.roots
            .borrow()
            .get(&handle)
            .map_or(0, |entry| entry.subtree)
            .max(self.document_floor_generation.get())
    }

    pub(super) fn element_generation(&self, handle: DomHandle) -> u64 {
        self.roots
            .borrow()
            .get(&handle)
            .map_or(0, |entry| entry.element)
    }

    pub(super) fn checked_path(&self, handle: DomHandle) -> Option<CheckedStylePath> {
        self.checked_paths
            .borrow()
            .get(&handle)
            .copied()
            .filter(|stamp| stamp.registry_generation == self.generation())
    }

    pub(super) fn element_is_current(&self, element: DomHandle, required_generation: u64) -> bool {
        required_generation == 0
            || self
                .checked_paths
                .borrow()
                .get(&element)
                .is_some_and(|stamp| stamp.resolved_generation >= required_generation)
    }

    pub(super) fn mark_path_checked(
        &self,
        element: DomHandle,
        required_generation: u64,
        subtree_generation: u64,
        depth: usize,
    ) -> CheckedStylePath {
        // Stamp only after resolving ancestors. An ancestor's actual cascade
        // can add requirements that weren't known when the path was found.
        let mut paths = self.checked_paths.borrow_mut();
        let entry = paths.entry(element).or_default();
        *entry = CheckedStylePath {
            registry_generation: self.generation(),
            resolved_generation: entry.resolved_generation.max(required_generation),
            subtree_generation,
            depth,
        };
        *entry
    }

    pub(super) fn clear(&self) {
        self.generation.set(0);
        self.document_floor_generation.set(0);
        self.roots.borrow_mut().clear();
        self.descendant_restyles.borrow_mut().clear();
        self.checked_paths.borrow_mut().clear();
        #[cfg(test)]
        self.path_node_visits.set(0);
    }

    #[cfg(test)]
    pub(super) fn root_count(&self) -> usize {
        self.roots.borrow().len()
    }

    #[cfg(test)]
    pub(super) fn path_node_visit_count(&self) -> u64 {
        self.path_node_visits.get()
    }

    fn next_generation(&self) -> u64 {
        self.generation().checked_add(1).unwrap_or_else(|| {
            self.clear();
            self.document_floor_generation.set(1);
            1
        })
    }

    pub(super) fn note_path_node_visit(&self) {
        #[cfg(test)]
        self.path_node_visits
            .set(self.path_node_visits.get().saturating_add(1));
    }
}
