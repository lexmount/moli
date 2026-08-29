use std::collections::HashMap;

use dom::ElementState as StyloElementState;
use moli_selector::StyloStyleSourceScope as StyleSourceScope;

use crate::{
    document_runtime::DomHandle, dom::native::DomHost, protocol_types::EmulatedMediaOverrides,
};

#[cfg(test)]
use super::pending_invalidation::PendingStyleInvalidationWork;
#[cfg(test)]
use super::schedule::queue_style_invalidation_targets;
#[cfg(test)]
use super::target_queries::PendingStyleInvalidationTargetQueries;
use super::{
    MoliStyleEngine, StyleMutationEffect, StyleViewport,
    cause::PendingStyleInvalidationCause,
    document_world::DocumentStyleWorld,
    mutation_effect::detached_style_subtree_roots_for_mutations,
    schedule::queue_style_invalidation_for_scope,
    scope::{
        mutation_effects_have_source_scope, source_scope_for_custom_state_change,
        source_scope_for_element_state_change, source_scope_for_mutations,
    },
};

const IMMEDIATE_STRUCTURAL_MUTATION_EFFECT_LIMIT: usize = 256;

impl MoliStyleEngine {
    #[cfg(test)]
    pub(crate) fn invalidate_for_mutations(
        &mut self,
        host: &DomHost,
        effects: &[StyleMutationEffect],
        emulated_media: &EmulatedMediaOverrides,
    ) {
        self.invalidate_for_mutations_with_viewport(
            host,
            effects,
            emulated_media,
            StyleViewport::default(),
        );
    }

    pub(crate) fn invalidate_for_mutations_with_viewport(
        &mut self,
        host: &DomHost,
        effects: &[StyleMutationEffect],
        emulated_media: &EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) {
        let profile_enabled = moli_trace::cpu_profile_enabled();
        let total_started = profile_enabled.then(std::time::Instant::now);
        let grouping_started = profile_enabled.then(std::time::Instant::now);
        let grouped_effects = style_mutation_effects_by_owner_document(host, effects);
        let document_count = grouped_effects.len();
        let grouping_us = grouping_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let mut no_cached_style_us = 0u128;
        let mut deferred_us = 0u128;
        let mut detached_cleanup_us = 0u128;
        let mut queue_us = 0u128;
        for (document, document_effects) in grouped_effects {
            let world = self.world_for_document(document);
            let no_cached_style_started = profile_enabled.then(std::time::Instant::now);
            if self.style_invalidation_without_cached_styles_is_complete(&document_effects, &world)
            {
                no_cached_style_us += no_cached_style_started
                    .map(|started| started.elapsed().as_micros())
                    .unwrap_or_default();
                continue;
            }
            no_cached_style_us += no_cached_style_started
                .map(|started| started.elapsed().as_micros())
                .unwrap_or_default();
            if should_defer_structural_mutations(&document_effects) {
                let deferred_started = profile_enabled.then(std::time::Instant::now);
                world.document_state.bump_target_context_epoch();
                world.pending_structural_mutations.push(
                    &document_effects,
                    emulated_media,
                    viewport,
                );
                deferred_us += deferred_started
                    .map(|started| started.elapsed().as_micros())
                    .unwrap_or_default();
                continue;
            }
            let detached_cleanup_started = profile_enabled.then(std::time::Instant::now);
            self.invalidate_detached_style_subtrees_for_mutations(host, &document_effects);
            detached_cleanup_us += detached_cleanup_started
                .map(|started| started.elapsed().as_micros())
                .unwrap_or_default();
            let queue_started = profile_enabled.then(std::time::Instant::now);
            self.queue_style_invalidation_for_mutations(
                document,
                &world,
                host,
                &document_effects,
                emulated_media,
                viewport,
            );
            queue_us += queue_started
                .map(|started| started.elapsed().as_micros())
                .unwrap_or_default();
        }
        if let Some(started) = total_started {
            let total_us = started.elapsed().as_micros();
            if total_us >= 500 {
                tracing::info!(
                    target: "moli_cpu_profile",
                    stage = "style_invalidate_mutations",
                    effect_count = effects.len(),
                    document_count,
                    grouping_us,
                    no_cached_style_us,
                    deferred_us,
                    detached_cleanup_us,
                    queue_us,
                    total_us,
                );
            }
        }
    }

    fn style_invalidation_without_cached_styles_is_complete(
        &self,
        effects: &[StyleMutationEffect],
        world: &DocumentStyleWorld,
    ) -> bool {
        if world_has_style_state(self, world) {
            return false;
        }
        if mutation_effects_have_source_scope(effects) {
            world.document_state.bump_target_context_epoch();
        }
        true
    }

    fn queue_style_invalidation_for_mutations(
        &self,
        document: DomHandle,
        world: &DocumentStyleWorld,
        host: &DomHost,
        effects: &[StyleMutationEffect],
        emulated_media: &EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) {
        let profile_enabled = moli_trace::cpu_profile_enabled();
        let total_started = profile_enabled.then(std::time::Instant::now);
        let source_scope_started = profile_enabled.then(std::time::Instant::now);
        let source_scope = source_scope_for_mutations(host, effects);
        let source_scope_us = source_scope_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let pending_effects_started = profile_enabled.then(std::time::Instant::now);
        let pending_effects = effects
            .iter()
            .filter(|effect| !matches!(effect, StyleMutationEffect::DisconnectedSubtree { .. }))
            .cloned()
            .collect();
        let pending_effects_us = pending_effects_started
            .map(|started| started.elapsed().as_micros())
            .unwrap_or_default();
        let queue_scope_started = profile_enabled.then(std::time::Instant::now);
        self.queue_style_invalidation_scope(
            document,
            world,
            host,
            emulated_media,
            viewport,
            source_scope,
            PendingStyleInvalidationCause::Mutation(pending_effects),
        );
        if let Some(started) = total_started {
            let total_us = started.elapsed().as_micros();
            if total_us >= 500 {
                tracing::info!(
                    target: "moli_cpu_profile",
                    stage = "queue_style_invalidation_for_mutations",
                    effect_count = effects.len(),
                    source_scope_us,
                    pending_effects_us,
                    queue_scope_us = queue_scope_started
                        .map(|started| started.elapsed().as_micros())
                        .unwrap_or_default(),
                    total_us,
                );
            }
        }
    }

    pub(super) fn flush_pending_structural_mutations_for_world(
        &self,
        host: &DomHost,
        document: DomHandle,
        world: &DocumentStyleWorld,
    ) {
        for group in world.pending_structural_mutations.take() {
            self.queue_deferred_style_invalidation_for_mutations(
                document,
                world,
                host,
                &group.effects,
                &group.emulated_media,
                group.viewport,
            );
        }
    }

    fn queue_deferred_style_invalidation_for_mutations(
        &self,
        document: DomHandle,
        world: &DocumentStyleWorld,
        host: &DomHost,
        effects: &[StyleMutationEffect],
        emulated_media: &EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) {
        if effects.len() <= IMMEDIATE_STRUCTURAL_MUTATION_EFFECT_LIMIT {
            self.invalidate_detached_style_subtrees_for_mutations(host, effects);
            self.queue_style_invalidation_for_mutations(
                document,
                world,
                host,
                effects,
                emulated_media,
                viewport,
            );
            return;
        }
        self.queue_structural_mutation_source_scope_fallback(
            document,
            world,
            host,
            effects,
            emulated_media,
            viewport,
        );
    }

    fn queue_structural_mutation_source_scope_fallback(
        &self,
        document: DomHandle,
        world: &DocumentStyleWorld,
        host: &DomHost,
        effects: &[StyleMutationEffect],
        emulated_media: &EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) {
        let source_scope = source_scope_for_mutations(host, effects);
        let cause = PendingStyleInvalidationCause::Mutation(Vec::new());
        self.queue_style_invalidation_scope(
            document,
            world,
            host,
            emulated_media,
            viewport,
            source_scope,
            cause,
        );
    }

    pub(super) fn invalidate_detached_style_subtrees_for_mutations(
        &self,
        host: &DomHost,
        effects: &[StyleMutationEffect],
    ) {
        let mut roots_by_document = HashMap::<DomHandle, Vec<DomHandle>>::new();
        for root in detached_style_subtree_roots_for_mutations(effects) {
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
        emulated_media: &EmulatedMediaOverrides,
    ) {
        self.invalidate_for_focus_change_with_viewport(
            host,
            previous,
            next,
            emulated_media,
            StyleViewport::default(),
        );
    }

    pub(crate) fn invalidate_for_focus_change_with_viewport(
        &mut self,
        host: &DomHost,
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
        emulated_media: &EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) {
        self.invalidate_for_focus_change_with_previous_focus_within_and_viewport(
            host,
            previous,
            next,
            None,
            emulated_media,
            viewport,
        );
    }

    pub(crate) fn invalidate_for_focus_change_with_previous_focus_within_and_viewport(
        &mut self,
        host: &DomHost,
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
        previous_focus_within: Option<Vec<DomHandle>>,
        emulated_media: &EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) {
        let document_scopes =
            focus_change_document_scopes(host, previous, next, previous_focus_within.as_deref());
        if document_scopes.is_empty() {
            return;
        }
        for document_scope in document_scopes {
            let world = self.world_for_document(document_scope.document);
            self.queue_style_invalidation_scope(
                document_scope.document,
                &world,
                host,
                emulated_media,
                viewport,
                Some(StyleSourceScope::for_handles(
                    host,
                    document_scope.source_handles.iter().copied(),
                )),
                PendingStyleInvalidationCause::FocusChange {
                    previous: document_scope.previous,
                    next: document_scope.next,
                    previous_focus_within: non_empty_vec(document_scope.previous_focus_within),
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
        emulated_media: &EmulatedMediaOverrides,
    ) {
        self.invalidate_for_element_state_change_with_old_state_and_viewport(
            host,
            element,
            state,
            old_state,
            emulated_media,
            StyleViewport::default(),
        );
    }

    pub(crate) fn invalidate_for_element_state_change_with_old_state_and_viewport(
        &mut self,
        host: &DomHost,
        element: DomHandle,
        state: StyloElementState,
        old_state: Option<StyloElementState>,
        emulated_media: &EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) {
        let Some(document) = owner_document_for_handle(host, element) else {
            return;
        };
        let source_scope = source_scope_for_element_state_change(host, element, state);
        let world = self.world_for_document(document);
        self.queue_style_invalidation_scope(
            document,
            &world,
            host,
            emulated_media,
            viewport,
            source_scope,
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
        emulated_media: &EmulatedMediaOverrides,
    ) {
        self.invalidate_for_custom_state_change_with_viewport(
            host,
            element,
            state_names,
            old_custom_states,
            emulated_media,
            StyleViewport::default(),
        );
    }

    pub(crate) fn invalidate_for_custom_state_change_with_viewport(
        &mut self,
        host: &DomHost,
        element: DomHandle,
        state_names: Vec<String>,
        old_custom_states: Vec<String>,
        emulated_media: &EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) {
        let Some(document) = owner_document_for_handle(host, element) else {
            return;
        };
        let source_scope = source_scope_for_custom_state_change(host, element, &state_names);
        let world = self.world_for_document(document);
        self.queue_style_invalidation_scope(
            document,
            &world,
            host,
            emulated_media,
            viewport,
            source_scope,
            PendingStyleInvalidationCause::CustomStateChange {
                element,
                state_names,
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
        emulated_media: &EmulatedMediaOverrides,
    ) {
        self.invalidate_for_target_change_with_viewport(
            host,
            previous,
            next,
            emulated_media,
            StyleViewport::default(),
        );
    }

    pub(crate) fn invalidate_for_target_change_with_viewport(
        &mut self,
        host: &DomHost,
        previous: Option<DomHandle>,
        next: Option<DomHandle>,
        emulated_media: &EmulatedMediaOverrides,
        viewport: StyleViewport,
    ) {
        let document_scopes = target_change_document_scopes(host, previous, next);
        if document_scopes.is_empty() {
            return;
        }
        for document_scope in document_scopes {
            let world = self.world_for_document(document_scope.document);
            self.queue_style_invalidation_scope(
                document_scope.document,
                &world,
                host,
                emulated_media,
                viewport,
                Some(StyleSourceScope::for_handles(
                    host,
                    document_scope.source_handles.iter().copied(),
                )),
                PendingStyleInvalidationCause::TargetChange {
                    previous: document_scope.previous,
                    next: document_scope.next,
                },
            );
        }
    }

    pub(crate) fn invalidate_inline_style_subtree(&mut self, host: &DomHost, root: DomHandle) {
        let Some(world) = self.owner_document_world(host, root) else {
            return;
        };
        self.invalidation_cleanup_for_world(&world)
            .invalidate_inline_style_subtree(host, root);
    }

    pub(crate) fn invalidate_style_subtree(&mut self, host: &DomHost, root: DomHandle) {
        let Some(world) = self.owner_document_world(host, root) else {
            return;
        };
        self.invalidation_cleanup_for_world(&world)
            .invalidate_subtrees_and_shadow_cascade_data(host, [root]);
    }

    fn queue_style_invalidation_scope(
        &self,
        document: DomHandle,
        world: &DocumentStyleWorld,
        host: &DomHost,
        emulated_media: &EmulatedMediaOverrides,
        viewport: StyleViewport,
        source_scope: Option<StyleSourceScope>,
        cause: PendingStyleInvalidationCause,
    ) {
        let Some(source_scope) = source_scope else {
            return;
        };
        let source_stores = world.borrow_source_stores();
        if !world_has_style_state(self, world) {
            world.document_state.bump_target_context_epoch();
            return;
        }
        queue_style_invalidation_for_scope(
            &world.pending_invalidations,
            &world.document_state,
            &source_stores,
            &self.dom_adapter,
            host,
            document,
            emulated_media,
            viewport,
            Some(source_scope),
            cause,
        );
    }

    #[cfg(test)]
    pub(super) fn queue_style_invalidation_targets_for_document_for_test(
        &self,
        document: DomHandle,
        cause: PendingStyleInvalidationCause,
        target_queries: Vec<PendingStyleInvalidationTargetQueries>,
    ) {
        let world = self.world_for_document(document);
        queue_style_invalidation_targets(
            &world.pending_invalidations,
            PendingStyleInvalidationWork::new(
                cause.work_kind(),
                target_queries,
                cause.pending_merge_class(),
            ),
        );
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

fn should_defer_structural_mutations(effects: &[StyleMutationEffect]) -> bool {
    super::mutation_effect::style_mutation_effects_are_child_list_structural(effects)
}

fn style_mutation_effects_by_owner_document(
    host: &DomHost,
    effects: &[StyleMutationEffect],
) -> Vec<(DomHandle, Vec<StyleMutationEffect>)> {
    let mut groups = Vec::<(DomHandle, Vec<StyleMutationEffect>)>::new();
    for effect in effects {
        let Some(document) = owner_document_for_mutation_effect(host, effect) else {
            continue;
        };
        if let Some((_, group_effects)) = groups
            .iter_mut()
            .find(|(group_document, _)| *group_document == document)
        {
            group_effects.push(effect.clone());
        } else {
            groups.push((document, vec![effect.clone()]));
        }
    }
    groups
}

fn owner_document_for_mutation_effect(
    host: &DomHost,
    effect: &StyleMutationEffect,
) -> Option<DomHandle> {
    match effect {
        StyleMutationEffect::Attribute { element, .. } => owner_document_for_handle(host, *element),
        StyleMutationEffect::ConnectedSubtree { root }
        | StyleMutationEffect::DisconnectedSubtree { root }
        | StyleMutationEffect::CharacterData { node: root } => {
            owner_document_for_handle(host, *root)
        }
        StyleMutationEffect::SlotAssignment { slot, .. } => owner_document_for_handle(host, *slot),
        StyleMutationEffect::ChildList { parent, .. } => owner_document_for_handle(host, *parent),
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
    source_handles: Vec<DomHandle>,
}

struct TargetChangeDocumentScope {
    document: DomHandle,
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
    source_handles: Vec<DomHandle>,
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
        push_unique_handle(&mut scope.source_handles, previous);
    }
    if let Some(next) = next {
        let Some(document) = owner_document_for_handle(host, next) else {
            return scopes;
        };
        let scope = focus_change_scope_for_document(&mut scopes, document);
        scope.next = Some(next);
        push_unique_handle(&mut scope.source_handles, next);
    }
    for handle in previous_focus_within.into_iter().flatten().copied() {
        let Some(document) = owner_document_for_handle(host, handle) else {
            continue;
        };
        let scope = focus_change_scope_for_document(&mut scopes, document);
        push_unique_handle(&mut scope.previous_focus_within, handle);
        push_unique_handle(&mut scope.source_handles, handle);
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
        push_unique_handle(&mut scope.source_handles, previous);
    }
    if let Some(next) = next {
        let Some(document) = owner_document_for_handle(host, next) else {
            return scopes;
        };
        let scope = target_change_scope_for_document(&mut scopes, document);
        scope.next = Some(next);
        push_unique_handle(&mut scope.source_handles, next);
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
        source_handles: Vec::new(),
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
        source_handles: Vec::new(),
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

fn non_empty_vec(handles: Vec<DomHandle>) -> Option<Vec<DomHandle>> {
    if handles.is_empty() {
        None
    } else {
        Some(handles)
    }
}
