use std::{borrow::Cow, collections::HashMap, sync::Arc};

use dom::ElementState as StyloElementState;
use indexmap::IndexSet;
use moli_selector::{CssDirection, html_auto_directionality_root, html_directionality};
use smallvec::SmallVec;
use style::invalidation::element::restyle_hints::RestyleHint;

#[cfg(test)]
use crate::protocol_types::EmulatedMediaOverrides;
use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::{
    MoliStyleEngine, StyleMutationEffect, cause::PendingStyleInvalidationCause,
    document_world::DocumentStyleWorld, snapshot::snapshot_state_change,
};

type StyleMutationGroups<'a> = SmallVec<[(DomHandle, Cow<'a, [StyleMutationEffect]>); 1]>;

impl MoliStyleEngine {
    pub(crate) fn document_has_style_state(&self, document: DomHandle) -> bool {
        let Some(world) = self.document_worlds.active_world(document) else {
            return false;
        };
        world_has_style_state(self, &world)
    }

    #[cfg(test)]
    pub(crate) fn invalidate_for_mutations(
        &mut self,
        host: &DomHost,
        effects: &[StyleMutationEffect],
        _emulated_media: &EmulatedMediaOverrides,
    ) {
        self.invalidate_dom_mutations(host, effects);
    }

    pub(crate) fn invalidate_dom_mutations(
        &mut self,
        host: &DomHost,
        effects: &[StyleMutationEffect],
    ) {
        for (document, effects) in style_mutation_effects_by_owner_document(host, effects) {
            let world = self.world_for_document(document);
            world
                .document_state
                .note_screen_style_attribute_changes(effects.iter().filter_map(|effect| {
                    let StyleMutationEffect::Attribute { element, .. } = effect else {
                        return None;
                    };
                    (!effect.affects_native_layout(host) && host.is_connected(*element))
                        .then_some(*element)
                }));
            if !world_has_style_state(self, &world) {
                world.document_state.note_full_screen_style_invalidation();
                world.document_state.note_style_input_change();
                continue;
            }
            world.document_state.note_style_input_change();
            let defer_cleanup = should_defer_structural_cleanup(&effects);
            if !defer_cleanup {
                self.invalidate_auto_direction_subtrees(&world, host, &effects);
                let detached_roots = effects
                    .iter()
                    .flat_map(StyleMutationEffect::detached_roots)
                    .copied()
                    .collect::<IndexSet<_>>();
                self.invalidate_detached_style_subtrees(host, detached_roots);
            }
            world.pending_invalidations.borrow_mut().record_mutations(
                self,
                &world,
                host,
                &effects,
                defer_cleanup,
            );
        }
    }

    fn invalidate_auto_direction_subtrees(
        &self,
        world: &DocumentStyleWorld,
        host: &DomHost,
        effects: &[StyleMutationEffect],
    ) {
        let mut roots = effects
            .iter()
            .filter_map(|effect| match effect {
                StyleMutationEffect::CharacterData { node } => {
                    let node = host.node(*node)?;
                    node.as_text()?;
                    html_auto_directionality_root(host, node.parent_node()?)
                }
                StyleMutationEffect::ChildList { parent, .. } => {
                    html_auto_directionality_root(host, *parent)
                }
                StyleMutationEffect::Attribute {
                    element,
                    namespace: None,
                    name,
                    ..
                } if name == "dir" => {
                    let parent = host.parent_node(*element)?;
                    html_auto_directionality_root(host, parent)
                }
                _ => None,
            })
            .peekable();
        if roots.peek().is_some() {
            // The direction of an ancestor can depend on this text or dir boundary.
            self.invalidation_cleanup_for_world(world)
                .invalidate_subtrees(host, roots);
        }
    }

    pub(super) fn retained_style_references_attribute(
        &self,
        world: &DocumentStyleWorld,
        host: &DomHost,
        element: DomHandle,
        name: &str,
    ) -> bool {
        self.dom_adapter
            .element_styles_reference_attribute(host, element, name)
            || world.pseudo_style_cache.references_attribute(element, name)
    }

    pub(super) fn invalidate_detached_style_subtrees(
        &self,
        host: &DomHost,
        roots: impl IntoIterator<Item = DomHandle>,
    ) {
        let mut roots_by_document = HashMap::<DomHandle, Vec<DomHandle>>::new();
        for root in roots {
            let Some(document) = owner_document_for_handle(host, root) else {
                continue;
            };
            roots_by_document.entry(document).or_default().push(root);
        }
        for (document, roots) in roots_by_document {
            let world = self.world_for_document(document);
            self.invalidation_cleanup_for_world(&world)
                .invalidate_detached_subtrees(host, roots);
        }
    }

    #[cfg(test)]
    pub(crate) fn invalidate_for_focus_change(
        &mut self,
        host: &DomHost,
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
        _emulated_media: &EmulatedMediaOverrides,
    ) {
        self.invalidate_focus_change(host, previous, next);
    }

    pub(crate) fn invalidate_focus_change(
        &mut self,
        host: &DomHost,
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
    ) {
        self.invalidate_focus_change_with_previous_focus_within(host, previous, next, None);
    }

    pub(crate) fn invalidate_focus_change_with_previous_focus_within(
        &mut self,
        host: &DomHost,
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
        previous_focus_within: Option<Vec<DomHandle>>,
    ) {
        let document_scopes =
            focus_change_document_scopes(host, previous, next, previous_focus_within.as_deref());
        for scope in document_scopes {
            let world = self.world_for_document(scope.document);
            self.queue_state_snapshot(
                &world,
                host,
                PendingStyleInvalidationCause::FocusChange {
                    previous: scope.previous,
                    next: scope.next,
                    previous_focus_within: scope.previous_focus_within,
                },
            );
        }
    }

    #[cfg(test)]
    pub(crate) fn invalidate_for_element_state_change_with_old_state(
        &mut self,
        host: &DomHost,
        element: DomHandle,
        state: StyloElementState,
        old_state: Option<StyloElementState>,
        _emulated_media: &EmulatedMediaOverrides,
    ) {
        self.invalidate_element_state_change(host, element, state, old_state);
    }

    pub(crate) fn invalidate_element_state_change(
        &mut self,
        host: &DomHost,
        element: DomHandle,
        state: StyloElementState,
        old_state: Option<StyloElementState>,
    ) {
        let Some(document) = owner_document_for_handle(host, element) else {
            return;
        };
        let world = self.world_for_document(document);
        if state.contains(StyloElementState::HOVER) && host.is_connected(element) {
            world.document_state.note_screen_hover_change();
        }
        if state.intersects(StyloElementState::LTR | StyloElementState::RTL)
            && html_auto_directionality_root(host, element) == Some(element)
            && old_state.is_none_or(|old| {
                let is_rtl = html_directionality(host, element) == CssDirection::Rtl;
                old.contains(StyloElementState::RTL) != is_rtl
            })
        {
            self.invalidation_cleanup_for_world(&world)
                .invalidate_subtrees(host, [element]);
        }
        // The snapshot matcher tests PAUSED positively for :playing.
        // Recheck these state changes with the native invalidation walk.
        let old_state = old_state.filter(|_| !state.contains(StyloElementState::PAUSED));
        self.queue_state_snapshot(
            &world,
            host,
            PendingStyleInvalidationCause::StateChange {
                element,
                state,
                old_state,
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn invalidate_for_custom_state_change(
        &mut self,
        host: &DomHost,
        element: DomHandle,
        state_names: Vec<String>,
        old_custom_states: Vec<String>,
        _emulated_media: &EmulatedMediaOverrides,
    ) {
        self.invalidate_custom_state_change(host, element, state_names, old_custom_states);
    }

    pub(crate) fn invalidate_custom_state_change(
        &mut self,
        host: &DomHost,
        element: DomHandle,
        state_names: Vec<String>,
        old_custom_states: Vec<String>,
    ) {
        let Some(document) = owner_document_for_handle(host, element) else {
            return;
        };
        if state_names.is_empty() {
            return;
        }
        let world = self.world_for_document(document);
        self.queue_state_snapshot(
            &world,
            host,
            PendingStyleInvalidationCause::CustomStateChange {
                element,
                old_custom_states,
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn invalidate_for_target_change(
        &mut self,
        host: &DomHost,
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
        _emulated_media: &EmulatedMediaOverrides,
    ) {
        self.invalidate_target_change(host, previous, next);
    }

    pub(crate) fn invalidate_target_change(
        &mut self,
        host: &DomHost,
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
    ) {
        let document_scopes = target_change_document_scopes(host, previous, next);
        for scope in document_scopes {
            let world = self.world_for_document(scope.document);
            self.queue_state_snapshot(
                &world,
                host,
                PendingStyleInvalidationCause::TargetChange {
                    previous: scope.previous,
                    next: scope.next,
                },
            );
        }
    }

    pub(crate) fn invalidate_inline_style_subtree(&mut self, host: &DomHost, root: DomHandle) {
        let Some(world) = self.active_owner_document_world(host, root) else {
            return;
        };
        // Attribute snapshots carry any selector dependencies. A retained
        // attr(style) value can also change when the inline declaration does.
        let hint = if self.retained_style_references_attribute(&world, host, root, "style") {
            RestyleHint::RESTYLE_SELF
        } else {
            RestyleHint::RESTYLE_STYLE_ATTRIBUTE
        };
        world.document_state.note_visual_input_change();
        self.invalidation_cleanup_for_world(&world)
            .invalidate_inline_style(host, root, hint);
    }

    pub(crate) fn invalidate_style_subtree(&mut self, host: &DomHost, root: DomHandle) {
        let Some(world) = self.owner_document_world(host, root) else {
            return;
        };
        world.document_state.note_visual_input_change();
        self.invalidation_cleanup_for_world(&world)
            .invalidate_subtrees_and_shadow_cascade_data(host, [root]);
    }

    fn queue_state_snapshot(
        &self,
        world: &DocumentStyleWorld,
        host: &DomHost,
        cause: PendingStyleInvalidationCause,
    ) {
        world.document_state.note_style_input_change();
        if !world_has_style_state(self, world) {
            return;
        }
        let mut pending = world.pending_invalidations.borrow_mut();
        if let PendingStyleInvalidationCause::StateChange {
            element,
            state,
            old_state: None,
        } = cause
        {
            // Some host state changes report what could have changed without
            // the old bits. Invalidate their scope only if such selectors exist.
            if world
                .document_state
                .try_with_retained_style_system(|retained| {
                    host.has_shadow_roots()
                        || retained
                            .stylist
                            .iter_origins()
                            .any(|(data, _)| data.has_state_dependency(state))
                })
                != Some(false)
            {
                pending.record_state_without_snapshot(host, element, state);
            }
            return;
        }
        let mut snapshots = moli_selector::StyleMutationSnapshots::default();
        snapshot_state_change(
            host,
            &self.dom_adapter,
            world.document,
            &mut snapshots,
            cause,
        );
        pending.extend_snapshots(host, snapshots);
    }

    #[cfg(test)]
    pub(super) fn invalidate_style_subtrees(
        &self,
        host: &DomHost,
        roots: impl IntoIterator<Item = DomHandle>,
    ) -> bool {
        let mut roots_by_document = HashMap::<DomHandle, Vec<DomHandle>>::new();
        for root in roots {
            let Some(document) = host.owner_document_handle(root) else {
                continue;
            };
            roots_by_document.entry(document).or_default().push(root);
        }
        let mut cleared = false;
        for (document, roots) in roots_by_document {
            let world = self.world_for_document(document);
            cleared |= self
                .invalidation_cleanup_for_world(&world)
                .invalidate_subtrees(host, roots);
        }
        cleared
    }
}

fn world_has_style_state(engine: &MoliStyleEngine, world: &DocumentStyleWorld) -> bool {
    world
        .document_state
        .try_with_retained_style_system(|_| ())
        .is_some()
        || engine
            .dom_adapter
            .has_element_styles_for_document(world.document)
        || !world.pseudo_style_cache.is_empty()
}

/// Pure child-list batches can wait for the final tree. Other effects can need
/// to retire detached style data or resolve attribute dependencies immediately.
fn should_defer_structural_cleanup(effects: &[StyleMutationEffect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, StyleMutationEffect::ChildList { .. }))
        && effects.iter().all(|effect| {
            matches!(
                effect,
                StyleMutationEffect::ChildList { .. }
                    | StyleMutationEffect::ConnectedSubtrees { .. }
                    | StyleMutationEffect::DisconnectedSubtrees { .. }
                    | StyleMutationEffect::SlotAssignment { .. }
            )
        })
}

#[inline]
fn style_mutation_effects_by_owner_document<'a>(
    host: &DomHost,
    effects: &'a [StyleMutationEffect],
) -> StyleMutationGroups<'a> {
    // A single-node effect already belongs to one document. Subtree effects
    // have no single owner here and continue through the batch grouping path.
    if let [effect] = effects
        && let Some(document) = owner_document_for_mutation_effect(host, effect)
    {
        // Borrowing also avoids the inner effect buffer and effect clones;
        // the outer SmallVec only avoids the document-group allocation.
        // A/B on fixed local WeChat HTML (2026-10-06, release, JS/CSS/subframes
        // disabled, 200 balanced randomized pairs, identical outer SmallVec):
        // owned Vec -> borrowed: 161.87 -> 156.24 ms; owned inner SmallVec ->
        // borrowed: 159.61 -> 157.12 ms. Paired-saving 95% bootstrap CIs were
        // [3.15, 6.58] and [0.19, 3.14] ms, respectively, for this workload.
        return SmallVec::from_buf([(document, Cow::Borrowed(effects))]);
    }
    group_batched_style_mutation_effects_by_owner_document(host, effects)
}

fn group_batched_style_mutation_effects_by_owner_document<'a>(
    host: &DomHost,
    effects: &'a [StyleMutationEffect],
) -> StyleMutationGroups<'a> {
    let mut groups = SmallVec::new();
    for effect in effects {
        match effect {
            StyleMutationEffect::ConnectedSubtrees { roots } => {
                push_subtree_effects_by_owner_document(host, &mut groups, roots, |roots| {
                    StyleMutationEffect::ConnectedSubtrees { roots }
                });
                continue;
            }
            StyleMutationEffect::DisconnectedSubtrees { roots } => {
                push_subtree_effects_by_owner_document(host, &mut groups, roots, |roots| {
                    StyleMutationEffect::DisconnectedSubtrees { roots }
                });
                continue;
            }
            _ => {}
        }
        let Some(document) = owner_document_for_mutation_effect(host, effect) else {
            continue;
        };
        push_document_effect(&mut groups, document, effect.clone());
    }
    groups
}

fn push_subtree_effects_by_owner_document(
    host: &DomHost,
    groups: &mut StyleMutationGroups<'_>,
    roots: &Arc<[DomHandle]>,
    make_effect: impl Fn(Arc<[DomHandle]>) -> StyleMutationEffect,
) {
    let Some((&first_root, remaining_roots)) = roots.split_first() else {
        return;
    };
    if let Some(document) = owner_document_for_handle(host, first_root)
        && remaining_roots
            .iter()
            .all(|root| owner_document_for_handle(host, *root) == Some(document))
    {
        push_document_effect(groups, document, make_effect(Arc::clone(roots)));
        return;
    }

    let mut document_roots = Vec::<(DomHandle, Vec<DomHandle>)>::new();
    for &root in roots.iter() {
        let Some(document) = owner_document_for_handle(host, root) else {
            continue;
        };
        if let Some((_, roots)) = document_roots
            .iter_mut()
            .find(|(candidate, _)| *candidate == document)
        {
            roots.push(root);
        } else {
            document_roots.push((document, vec![root]));
        }
    }
    for (document, roots) in document_roots {
        push_document_effect(groups, document, make_effect(roots.into()));
    }
}

fn push_document_effect(
    groups: &mut StyleMutationGroups<'_>,
    document: DomHandle,
    effect: StyleMutationEffect,
) {
    if let Some((_, group_effects)) = groups
        .iter_mut()
        .find(|(group_document, _)| *group_document == document)
    {
        group_effects.to_mut().push(effect);
    } else {
        groups.push((document, Cow::Owned(vec![effect])));
    }
}

fn owner_document_for_mutation_effect(
    host: &DomHost,
    effect: &StyleMutationEffect,
) -> Option<DomHandle> {
    match effect {
        StyleMutationEffect::Attribute { element, .. } => owner_document_for_handle(host, *element),
        StyleMutationEffect::CharacterData { node } => owner_document_for_handle(host, *node),
        StyleMutationEffect::SlotAssignment { slot, .. } => owner_document_for_handle(host, *slot),
        StyleMutationEffect::ChildList { parent, .. } => owner_document_for_handle(host, *parent),
        StyleMutationEffect::ConnectedSubtrees { .. }
        | StyleMutationEffect::DisconnectedSubtrees { .. } => None,
    }
}

fn owner_document_for_handle(host: &DomHost, handle: DomHandle) -> Option<DomHandle> {
    host.owner_document_handle(handle)
}

struct FocusChangeDocumentScope {
    document: DomHandle,
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
    previous_focus_within: Vec<DomHandle>,
}

struct TargetChangeDocumentScope {
    document: DomHandle,
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
}

fn focus_change_document_scopes(
    host: &DomHost,
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
    previous_focus_within: Option<&[DomHandle]>,
) -> Vec<FocusChangeDocumentScope> {
    let mut scopes = Vec::new();
    if let Some(previous) = previous {
        let Some(document) = owner_document_for_handle(host, previous) else {
            return scopes;
        };
        let scope = focus_change_scope_for_document(&mut scopes, document);
        scope.previous = Some(previous);
    }
    if let Some(next) = next {
        let Some(document) = owner_document_for_handle(host, next) else {
            return scopes;
        };
        let scope = focus_change_scope_for_document(&mut scopes, document);
        scope.next = Some(next);
    }
    for handle in previous_focus_within.into_iter().flatten().copied() {
        let Some(document) = owner_document_for_handle(host, handle) else {
            continue;
        };
        let scope = focus_change_scope_for_document(&mut scopes, document);
        push_unique_handle(&mut scope.previous_focus_within, handle);
    }
    scopes
}

fn target_change_document_scopes(
    host: &DomHost,
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
) -> Vec<TargetChangeDocumentScope> {
    let mut scopes = Vec::new();
    if let Some(previous) = previous {
        let Some(document) = owner_document_for_handle(host, previous) else {
            return scopes;
        };
        let scope = target_change_scope_for_document(&mut scopes, document);
        scope.previous = Some(previous);
    }
    if let Some(next) = next {
        let Some(document) = owner_document_for_handle(host, next) else {
            return scopes;
        };
        let scope = target_change_scope_for_document(&mut scopes, document);
        scope.next = Some(next);
    }
    scopes
}

fn focus_change_scope_for_document(
    scopes: &mut Vec<FocusChangeDocumentScope>,
    document: DomHandle,
) -> &mut FocusChangeDocumentScope {
    if let Some(index) = scopes.iter().position(|scope| scope.document == document) {
        return &mut scopes[index];
    }
    scopes.push(FocusChangeDocumentScope {
        document,
        previous: None,
        next: None,
        previous_focus_within: Vec::new(),
    });
    scopes
        .last_mut()
        .expect("scope was just pushed and must be present")
}

fn target_change_scope_for_document(
    scopes: &mut Vec<TargetChangeDocumentScope>,
    document: DomHandle,
) -> &mut TargetChangeDocumentScope {
    if let Some(index) = scopes.iter().position(|scope| scope.document == document) {
        return &mut scopes[index];
    }
    scopes.push(TargetChangeDocumentScope {
        document,
        previous: None,
        next: None,
    });
    scopes
        .last_mut()
        .expect("scope was just pushed and must be present")
}

fn push_unique_handle(handles: &mut Vec<DomHandle>, handle: DomHandle) {
    if !handles.contains(&handle) {
        handles.push(handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::native::NativeDom;

    #[test]
    fn batched_subtrees_are_split_by_owner_document() {
        let mut host = DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://example.test/").expect("valid test url"),
        ));
        let document = host.document_handle();
        let detached_document = host.create_detached_html_document();
        let active_root = host.create_element("main");
        let detached_root = host.create_element("aside");
        assert!(host.append_child(document, active_root));
        assert!(host.append_child(detached_document, detached_root));
        let effects = [StyleMutationEffect::ConnectedSubtrees {
            roots: vec![active_root, detached_root].into(),
        }];

        let groups = style_mutation_effects_by_owner_document(&host, &effects);
        assert_eq!(groups.len(), 2);
        for (expected_document, expected_root) in
            [(document, active_root), (detached_document, detached_root)]
        {
            let (_, effects) = groups
                .iter()
                .find(|(candidate, _)| *candidate == expected_document)
                .expect("each owner document should have a mutation group");
            assert!(matches!(
                effects.as_ref(),
                [StyleMutationEffect::ConnectedSubtrees { roots }]
                    if roots.as_ref() == [expected_root]
            ));
        }
    }
}
