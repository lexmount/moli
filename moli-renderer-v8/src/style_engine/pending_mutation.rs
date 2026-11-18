use std::cell::RefCell;

use indexmap::IndexSet;

use crate::{document_runtime::DomHandle, protocol_types::EmulatedMediaOverrides};

#[cfg(test)]
use super::cause::PendingStyleInvalidationWorkKind;
use super::{StyleMutationEffect, StyleViewport};

#[derive(Default)]
pub(super) struct PendingStructuralStyleMutations {
    groups: RefCell<Vec<PendingStructuralStyleMutationGroup>>,
}

pub(super) struct PendingStructuralStyleMutationGroup {
    pub(super) effects: PendingStructuralStyleEffects,
    pub(super) emulated_media: EmulatedMediaOverrides,
    pub(super) viewport: StyleViewport,
}

#[derive(Default)]
pub(super) struct PendingStructuralStyleEffects {
    history: Vec<StyleMutationEffect>,
    connected_roots: IndexSet<DomHandle>,
    disconnected_roots: IndexSet<DomHandle>,
    input_count: usize,
}

impl PendingStructuralStyleEffects {
    pub(super) fn extend(&mut self, effects: &[StyleMutationEffect]) {
        self.input_count += effects.len();
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

    pub(super) fn into_effects(self) -> (Vec<StyleMutationEffect>, usize) {
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
        (effects, self.input_count)
    }
}

impl PendingStructuralStyleMutations {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn push(
        &self,
        effects: &[StyleMutationEffect],
        emulated_media: &EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) {
        if effects.is_empty() {
            return;
        }
        let mut groups = self.groups.borrow_mut();
        if let Some(group) = groups.last_mut()
            && group.emulated_media == *emulated_media
            && group.viewport == viewport
        {
            group.effects.extend(effects);
            return;
        }
        let mut pending_effects = PendingStructuralStyleEffects::default();
        pending_effects.extend(effects);
        groups.push(PendingStructuralStyleMutationGroup {
            effects: pending_effects,
            emulated_media: emulated_media.clone(),
            viewport,
        });
    }

    pub(super) fn take(&self) -> Vec<PendingStructuralStyleMutationGroup> {
        self.groups.take()
    }

    pub(super) fn clear(&self) {
        self.groups.borrow_mut().clear();
    }

    #[cfg(test)]
    pub(super) fn work_item_count_for_test(&self) -> usize {
        self.groups.borrow().len()
    }

    #[cfg(test)]
    pub(super) fn effect_count_for_test(&self) -> usize {
        self.groups
            .borrow()
            .iter()
            .map(|group| group.effects.input_count)
            .sum()
    }

    #[cfg(test)]
    pub(super) fn work_kind_names_for_test(&self) -> Vec<&'static str> {
        self.groups
            .borrow()
            .iter()
            .map(|_| PendingStyleInvalidationWorkKind::Mutation.name_for_test())
            .collect()
    }
}
