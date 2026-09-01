use std::collections::HashMap;

use moli_layout::{FrozenLayoutTree, LayoutRememberedSize, LayoutRememberedSizePolicy};

use crate::document_runtime::DomHandle;

/// Sparse element-lifetime layout state that must not be folded into computed
/// style or retained layout snapshots.
///
/// Blink stores the equivalent last-remembered intrinsic sizes in
/// `ElementRareData`. Keeping the map beside Moli's DOM owner gives it the
/// same lifetime while the pass-local layout crate remains DOM-neutral.
#[derive(Default)]
pub(super) struct ElementLayoutState {
    remembered_sizes: HashMap<DomHandle, LayoutRememberedSize>,
}

impl ElementLayoutState {
    pub(super) fn remembered_size(&self, element: DomHandle) -> Option<LayoutRememberedSize> {
        self.remembered_sizes.get(&element).copied()
    }

    pub(super) fn publish_remembered_size_observations(
        &mut self,
        tree: &FrozenLayoutTree<DomHandle>,
        observations: impl IntoIterator<Item = (DomHandle, LayoutRememberedSizePolicy)>,
    ) {
        for (element, policy) in observations {
            let previous = self.remembered_sizes.get(&element).copied();
            let next = policy.updated_value(previous, tree.local_content_box_for_source(element));
            match next {
                Some(next) => {
                    self.remembered_sizes.insert(element, next);
                }
                None => {
                    self.remembered_sizes.remove(&element);
                }
            }
        }
    }
}
