use std::cell::RefCell;

use indexmap::IndexSet;

use crate::document_runtime::DomHandle;

use super::StyleMutationEffect;

#[derive(Default)]
pub(super) struct PendingStructuralStyleMutations {
    effects: RefCell<PendingStructuralStyleEffects>,
}

#[derive(Default)]
struct PendingStructuralStyleEffects {
    history: Vec<StyleMutationEffect>,
    connected_roots: IndexSet<DomHandle>,
    disconnected_roots: IndexSet<DomHandle>,
    #[cfg(test)]
    input_count: usize,
}

impl PendingStructuralStyleEffects {
    pub(super) fn extend(&mut self, effects: &[StyleMutationEffect]) {
        #[cfg(test)]
        {
            self.input_count += effects.len();
        }
        for effect in effects {
            match effect {
                StyleMutationEffect::ConnectedSubtrees { roots } => {
                    self.connected_roots.extend(roots.iter().copied());
                }
                StyleMutationEffect::DisconnectedSubtrees { roots } => {
                    self.disconnected_roots.extend(roots.iter().copied());
                }
                // Child-list splices and slot assignments carry ordered history.
                effect => self.history.push(effect.clone()),
            }
        }
    }

    fn into_effects(self) -> Vec<StyleMutationEffect> {
        let mut effects = self.history;
        if !self.connected_roots.is_empty() {
            effects.push(StyleMutationEffect::ConnectedSubtrees {
                roots: self.connected_roots.into_iter().collect::<Vec<_>>().into(),
            });
        }
        if !self.disconnected_roots.is_empty() {
            effects.push(StyleMutationEffect::DisconnectedSubtrees {
                roots: self
                    .disconnected_roots
                    .into_iter()
                    .collect::<Vec<_>>()
                    .into(),
            });
        }
        effects
    }
}

impl PendingStructuralStyleMutations {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn push(&self, effects: &[StyleMutationEffect]) {
        self.effects.borrow_mut().extend(effects);
    }

    pub(super) fn take(&self) -> Option<Vec<StyleMutationEffect>> {
        let effects = self.effects.take().into_effects();
        (!effects.is_empty()).then_some(effects)
    }

    pub(super) fn clear(&self) {
        self.effects.take();
    }

    #[cfg(test)]
    pub(super) fn work_item_count_for_test(&self) -> usize {
        usize::from(self.effect_count_for_test() > 0)
    }

    #[cfg(test)]
    pub(super) fn effect_count_for_test(&self) -> usize {
        self.effects.borrow().input_count
    }
}
