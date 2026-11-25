use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

use style::invalidation::element::restyle_hints::RestyleHint;

use crate::document_runtime::DomHandle;

pub(super) const MAX_CACHED_BRANCH_REQUIREMENTS: usize = 1_024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum StyleInvalidationScope {
    Element,
    Subtree,
}

#[derive(Clone, Copy, Default)]
pub(super) struct BranchStyleRequirements {
    pub(super) subtree: u64,
    children: u64,
    inheriting_reset: u64,
}

impl BranchStyleRequirements {
    pub(super) fn child_requirement(self, inherits_reset: bool) -> u64 {
        if inherits_reset {
            self.children.max(self.inheriting_reset)
        } else {
            self.children
        }
    }

    pub(super) fn child_hint_after(self, generation: u64) -> RestyleHint {
        if self.subtree > generation {
            RestyleHint::restyle_subtree()
        } else if self.children > generation {
            RestyleHint::RECASCADE_SELF
        } else if self.inheriting_reset > generation {
            RestyleHint::RECASCADE_SELF_IF_INHERIT_RESET_STYLE
        } else {
            RestyleHint::empty()
        }
    }
}

/// Cache the ancestry inputs used by published styles. An inherited change
/// updates just the parent entry. A demanded child checks that requirement
/// when it next reads the path, even if its siblings have already restyled.
#[derive(Default)]
pub(super) struct StylePathCache {
    generation: Cell<u64>,
    minimum_observation: Cell<u64>,
    branches: RefCell<HashMap<DomHandle, BranchStyleRequirements>>,
    #[cfg(test)]
    path_node_visits: Cell<u64>,
}

impl StylePathCache {
    pub(super) fn invalidate_paths(&self) -> bool {
        if let Some(generation) = self.generation().checked_add(1) {
            self.generation.set(generation);
            return false;
        }
        self.clear();
        self.minimum_observation.set(1);
        self.generation.set(1);
        true
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation.get()
    }

    pub(super) fn can_compare_after(&self, generation: u64) -> bool {
        (self.minimum_observation.get()..=self.generation()).contains(&generation)
    }

    pub(super) fn invalidate_subtree(&self, handle: DomHandle) {
        self.record_requirement(handle, |entry, generation| entry.subtree = generation);
    }

    pub(super) fn record_child_requirements(
        &self,
        handle: DomHandle,
        hint: RestyleHint,
        subtree_generation: u64,
    ) {
        if hint.is_empty() || self.generation() <= subtree_generation {
            return;
        }
        self.record_requirement(handle, |entry, generation| {
            if hint.will_recascade_subtree() {
                entry.subtree = generation;
            } else if hint.contains(RestyleHint::RECASCADE_SELF) {
                entry.children = generation;
            } else {
                debug_assert!(hint.contains(RestyleHint::RECASCADE_SELF_IF_INHERIT_RESET_STYLE));
                entry.inheriting_reset = generation;
            }
        });
    }

    fn record_requirement(
        &self,
        handle: DomHandle,
        update: impl FnOnce(&mut BranchStyleRequirements, u64),
    ) {
        let generation = self.generation();
        if generation <= self.minimum_observation.get() {
            return;
        }
        let mut branches = self.branches.borrow_mut();
        update(branches.entry(handle).or_default(), generation);
        if branches.len() > MAX_CACHED_BRANCH_REQUIREMENTS {
            // An unread style may still depend on any discarded branch key.
            self.minimum_observation.set(generation);
            branches.clear();
        }
    }

    pub(super) fn branch_requirement_count(&self) -> usize {
        self.branches.borrow().len()
    }

    pub(super) fn branch_requirements(&self, handle: DomHandle) -> BranchStyleRequirements {
        self.branches
            .borrow()
            .get(&handle)
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn subtree_generation(&self, handle: DomHandle) -> u64 {
        self.branch_requirements(handle)
            .subtree
            .max(self.minimum_observation.get())
    }

    pub(super) fn clear(&self) {
        self.generation.set(0);
        self.minimum_observation.set(0);
        self.branches.borrow_mut().clear();
        #[cfg(test)]
        self.path_node_visits.set(0);
    }

    #[cfg(test)]
    pub(super) fn root_count(&self) -> usize {
        self.branches
            .borrow()
            .values()
            .filter(|entry| entry.subtree != 0)
            .count()
    }

    #[cfg(test)]
    pub(super) fn path_node_visit_count(&self) -> u64 {
        self.path_node_visits.get()
    }

    pub(super) fn note_path_node_visit(&self) {
        #[cfg(test)]
        self.path_node_visits
            .set(self.path_node_visits.get().saturating_add(1));
    }
}
