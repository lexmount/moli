use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
};

use indexmap::{IndexMap, IndexSet};
use moli_selector::StyloDomStyleAdapter;
use selectors::context::SelectorCaches;
use style::{
    servo::restyle_damage::ServoRestyleDamage,
    servo_arc::Arc as ServoArc,
    stylist::{CascadeData, Stylist},
};

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::{
    CssCustomPropertyRegistrationRecord, StyleTreeScopeVersions, StyleViewport,
    StyloStyleEnvironment,
    active_stylesheets::ActiveStylesheetCollection,
    shadow_scopes::ShadowScopeStyles,
    source_dirty::{StyleSourceDirtyReason, StyleSourceDirtyScopeSnapshot, StyleSourceDirtyScopes},
    source_id::{StyleScopeId, StyleSourceId},
    style_paths::{StyleInvalidationScope, StylePathCache},
    stylesheet_resources::{
        StylesheetResourceGeneration, StylesheetResourceManifest, StylesheetResourceSnapshot,
    },
    world_key::StyleWorldKey,
};

pub(super) struct RetainedStyleSystem {
    pub(super) stylist_identity: u64,
    pub(super) key: StyleWorldKey,
    pub(super) stylist: Stylist,
    pub(super) document_stylesheets: ActiveStylesheetCollection,
    pub(super) shadow_scopes: Vec<ShadowScopeStyles>,
    pub(super) stylesheet_resources: StylesheetResourceManifest,
    pub(super) stylesheet_resource_revision: u64,
    pub(super) user_agent_cascade_data: ServoArc<CascadeData>,
    pub(super) shadow_cascade_data: Vec<(DomHandle, ServoArc<CascadeData>)>,
    pub(super) script_custom_property_registrations: Vec<CssCustomPropertyRegistrationRecord>,
}

/// Style inputs that are allowed to change a completed Document observation.
///
/// This deliberately excludes computed-style and retained-world generations:
/// those are outputs. Including either output here would let an accidental
/// clean-operation rebuild justify itself on the next read.
#[cfg(debug_assertions)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ComputedStyleObservationInputEpochs {
    pub(crate) dom_version: u64,
    pub(crate) style_viewport_generation: u64,
    pub(crate) tree_scope_versions: StyleTreeScopeVersions,
    pub(crate) document_generations: Vec<(DomHandle, u64, u64)>,
}

#[cfg(debug_assertions)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct CompletedStyleObservationWorld {
    retained_style_system_generation: u64,
    stylist_identity: u64,
    key: StyleWorldKey,
}

#[cfg(debug_assertions)]
#[derive(Clone, Debug)]
struct CompletedStyleObservation {
    input_epochs_after_read: ComputedStyleObservationInputEpochs,
    world: CompletedStyleObservationWorld,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StyleDocumentGenerationSnapshot {
    pub(super) source_set_generation: u64,
    pub(super) computed_cache_generation: u64,
    pub(super) retained_style_system_generation: u64,
    pub(super) target_context_epoch: u64,
}

const MAX_PENDING_SCREEN_RESTYLES: usize = 1_024;

#[derive(Default)]
pub(super) enum ScreenStyleInvalidation {
    #[default]
    Clean,
    Pending {
        after: u64,
        targets: IndexMap<DomHandle, StyleInvalidationScope>,
    },
    Full,
}

#[derive(Default)]
pub(crate) struct ScreenRestyleTargets {
    pub(crate) after: u64,
    pub(crate) targets: Vec<(DomHandle, StyleInvalidationScope)>,
}

/// Evidence that canonical styles were used by the completed layout. Lazy
/// pseudo styles that the layout read have no canonical ElementData slot.
#[derive(Default)]
pub(crate) struct ScreenStyleCoverage {
    context_generation: Option<u64>,
    pub(crate) lazy_pseudo_owners: HashSet<DomHandle>,
}

pub(super) struct StyleDocumentState {
    pub(super) retained_style_system: RefCell<Option<RetainedStyleSystem>>,
    selector_caches: RefCell<SelectorCaches>,
    source_dirty_scopes: StyleSourceDirtyScopes,
    source_set_generation: Cell<u64>,
    computed_cache_generation: Cell<u64>,
    retained_style_system_generation: Cell<u64>,
    target_context_epoch: Cell<u64>,
    /// Source-side invalidation only. The target context also advances when
    /// derived styles are cleared, including temporary capture environments.
    visual_input_generation: Cell<u64>,
    screen_style_context_generation: Cell<u64>,
    screen_style_coverage: RefCell<Option<ScreenStyleCoverage>>,
    pub(super) screen_style_invalidation: RefCell<ScreenStyleInvalidation>,
    // The damage itself belongs to Stylo ElementData. Keep the touched nodes
    // through a full-layout fallback so a successful publication can clear them.
    pub(super) screen_damage_nodes: RefCell<IndexSet<DomHandle>>,
    pub(super) style_paths: StylePathCache,
    #[cfg(debug_assertions)]
    completed_style_observation: RefCell<Option<CompletedStyleObservation>>,
    #[cfg(all(test, debug_assertions))]
    completed_style_observation_stability_checks: Cell<u64>,
    #[cfg(test)]
    retained_style_system_rebuilds: Cell<u64>,
    #[cfg(test)]
    retained_style_system_updates: Cell<u64>,
    #[cfg(test)]
    element_style_resolutions: Cell<u64>,
    #[cfg(test)]
    ancestor_style_validation_visits: Cell<u64>,
}

impl StyleDocumentState {
    pub(super) fn new() -> Self {
        Self {
            retained_style_system: RefCell::new(None),
            selector_caches: RefCell::new(SelectorCaches::default()),
            source_dirty_scopes: StyleSourceDirtyScopes::default(),
            source_set_generation: Cell::new(0),
            computed_cache_generation: Cell::new(0),
            retained_style_system_generation: Cell::new(0),
            target_context_epoch: Cell::new(0),
            visual_input_generation: Cell::new(0),
            screen_style_context_generation: Cell::new(0),
            screen_style_coverage: RefCell::default(),
            screen_style_invalidation: RefCell::default(),
            screen_damage_nodes: RefCell::default(),
            style_paths: StylePathCache::default(),
            #[cfg(debug_assertions)]
            completed_style_observation: RefCell::new(None),
            #[cfg(all(test, debug_assertions))]
            completed_style_observation_stability_checks: Cell::new(0),
            #[cfg(test)]
            retained_style_system_rebuilds: Cell::new(0),
            #[cfg(test)]
            retained_style_system_updates: Cell::new(0),
            #[cfg(test)]
            element_style_resolutions: Cell::new(0),
            #[cfg(test)]
            ancestor_style_validation_visits: Cell::new(0),
        }
    }

    pub(super) fn source_set_generation(&self) -> u64 {
        self.source_set_generation.get()
    }

    pub(super) fn bump_source_set_generation(&self) {
        self.source_set_generation
            .set(self.source_set_generation.get().saturating_add(1));
    }

    pub(super) fn computed_cache_generation(&self) -> u64 {
        self.computed_cache_generation.get()
    }

    pub(super) fn bump_computed_cache_generation(&self) {
        self.invalidate_screen_style_comparisons();
        self.computed_cache_generation
            .set(self.computed_cache_generation().saturating_add(1));
    }

    pub(super) fn note_screen_style_attribute_changes(
        &self,
        targets: impl IntoIterator<Item = DomHandle>,
    ) {
        let coverage = self.screen_style_coverage.borrow();
        let Some(coverage) = coverage.as_ref() else {
            return;
        };
        let mut change = self.screen_style_invalidation.borrow_mut();
        for handle in targets {
            // A primary-style read may already have evicted the lazy
            // pseudo's attr() dependencies. Screen layout still uses it.
            if coverage.lazy_pseudo_owners.contains(&handle) {
                *change = ScreenStyleInvalidation::Full;
                return;
            }
            if matches!(*change, ScreenStyleInvalidation::Clean) {
                *change = Self::pending_screen_style_invalidation(self.style_paths.generation());
            }
        }
    }

    pub(super) fn note_screen_hover_change(&self) {
        let mut change = self.screen_style_invalidation.borrow_mut();
        if matches!(*change, ScreenStyleInvalidation::Clean)
            && self.screen_style_coverage.borrow().is_some()
        {
            // Track the change before draining snapshots. A temporary
            // layout can replace the styles used by the screen comparison.
            *change = if self.screen_style_dependencies_are_current() {
                Self::pending_screen_style_invalidation(self.style_paths.generation())
            } else {
                ScreenStyleInvalidation::Full
            };
        }
    }

    pub(super) fn note_full_screen_style_invalidation(&self) {
        let mut change = self.screen_style_invalidation.borrow_mut();
        if !matches!(*change, ScreenStyleInvalidation::Clean) {
            *change = ScreenStyleInvalidation::Full;
        }
    }

    fn pending_screen_style_invalidation(after: u64) -> ScreenStyleInvalidation {
        ScreenStyleInvalidation::Pending {
            after,
            targets: IndexMap::new(),
        }
    }

    pub(super) fn record_style_targets(
        &self,
        host: &DomHost,
        dom_adapter: &StyloDomStyleAdapter,
        document: DomHandle,
        targets: impl IntoIterator<Item = (DomHandle, StyleInvalidationScope)>,
    ) {
        let mut targets = targets
            .into_iter()
            .filter(|(handle, _)| {
                host.node(*handle).is_some()
                    && (*handle == document
                        || host.owner_document_handle(*handle) == Some(document))
            })
            .peekable();
        if targets.peek().is_none() {
            return;
        }
        let mut screen = self.screen_style_invalidation.borrow_mut();
        let after = self.style_paths.generation();
        if self.style_paths.invalidate_paths() {
            dom_adapter.clear_style_paths(document);
        }

        for (handle, scope) in targets {
            if scope == StyleInvalidationScope::Subtree {
                self.style_paths.invalidate_subtree(handle);
            }
            // The canonical layout also needs restyles discovered while
            // draining native mutations or serving CSSOM reads.
            if host.is_connected(handle)
                && matches!(*screen, ScreenStyleInvalidation::Clean)
                && self.screen_style_dependencies_are_current()
            {
                *screen = Self::pending_screen_style_invalidation(after);
            }
            if let ScreenStyleInvalidation::Pending { targets, .. } = &mut *screen
                && host.is_connected(handle)
            {
                let target = targets.entry(handle).or_insert(scope);
                *target = (*target).max(scope);
                if targets.len() > MAX_PENDING_SCREEN_RESTYLES {
                    *screen = ScreenStyleInvalidation::Full;
                }
            }
        }
    }

    pub(super) fn record_screen_style_damage(
        &self,
        handle: DomHandle,
        has_previous_style: bool,
        compare: impl FnOnce() -> Option<ServoRestyleDamage>,
    ) -> ServoRestyleDamage {
        let mut change = self.screen_style_invalidation.borrow_mut();
        if (!has_previous_style && matches!(*change, ScreenStyleInvalidation::Clean))
            || !self.screen_style_dependencies_are_current()
        {
            return ServoRestyleDamage::empty();
        }

        let Some(observed) = compare() else {
            *change = ScreenStyleInvalidation::Full;
            return ServoRestyleDamage::empty();
        };
        if observed.is_empty() {
            return observed;
        }
        if matches!(*change, ScreenStyleInvalidation::Clean) {
            *change = Self::pending_screen_style_invalidation(self.style_paths.generation());
        }
        let mut nodes = self.screen_damage_nodes.borrow_mut();
        if nodes.len() == MAX_PENDING_SCREEN_RESTYLES && !nodes.contains(&handle) {
            *change = ScreenStyleInvalidation::Full;
            return ServoRestyleDamage::empty();
        }
        nodes.insert(handle);
        observed
    }

    pub(super) fn finish_screen_style_invalidation(&self) {
        let mut change = self.screen_style_invalidation.borrow_mut();
        let ScreenStyleInvalidation::Pending { after, targets } = &*change else {
            return;
        };
        if !self.screen_style_dependencies_are_current()
            || !self.style_paths.can_compare_after(*after)
        {
            *change = ScreenStyleInvalidation::Full;
        } else if self.screen_damage_nodes.borrow().is_empty() && targets.is_empty() {
            *change = ScreenStyleInvalidation::Clean;
        }
    }

    pub(super) fn screen_restyle_targets(
        &self,
        host: &DomHost,
        document: DomHandle,
    ) -> Option<ScreenRestyleTargets> {
        match &*self.screen_style_invalidation.borrow() {
            ScreenStyleInvalidation::Clean => Some(ScreenRestyleTargets::default()),
            ScreenStyleInvalidation::Pending { after, targets } => Some(ScreenRestyleTargets {
                after: *after,
                targets: targets
                    .iter()
                    .filter_map(|(&handle, &scope)| {
                        (host.is_connected(handle)
                            && host.owner_document_handle(handle) == Some(document))
                        .then_some((handle, scope))
                    })
                    .collect(),
            }),
            ScreenStyleInvalidation::Full => None,
        }
    }

    pub(super) fn screen_restyle_is_full(&self) -> bool {
        matches!(
            *self.screen_style_invalidation.borrow(),
            ScreenStyleInvalidation::Full
        )
    }

    pub(super) fn screen_needs_layout(&self) -> bool {
        !self.screen_damage_nodes.borrow().is_empty() || self.screen_restyle_is_full()
    }

    pub(super) fn screen_style_dependencies_are_current(&self) -> bool {
        let current = self.screen_style_context_generation.get();
        current != u64::MAX
            && self
                .screen_style_coverage
                .borrow()
                .as_ref()
                .is_some_and(|coverage| coverage.context_generation == Some(current))
    }

    pub(super) fn screen_style_coverage(&self) -> ScreenStyleCoverage {
        ScreenStyleCoverage {
            context_generation: Some(self.screen_style_context_generation.get()),
            ..Default::default()
        }
    }

    pub(super) fn invalidate_screen_style_comparisons(&self) {
        // A temporary observation can replace canonical styles while the
        // frozen geometry remains valid. Expire comparison evidence, and
        // require a rebuild when there is work waiting to use it.
        let generation = self.screen_style_context_generation.get();
        self.screen_style_context_generation
            .set(generation.saturating_add(1));
        self.note_full_screen_style_invalidation();
    }

    pub(super) fn begin_screen_layout(&self) {
        if self.screen_style_coverage.borrow().is_some() {
            *self.screen_style_invalidation.borrow_mut() = ScreenStyleInvalidation::Full;
        }
    }

    pub(super) fn publish_screen_styles(&self, coverage: ScreenStyleCoverage) {
        *self.screen_style_coverage.borrow_mut() = Some(coverage);
    }

    pub(super) fn retire_screen_style_publication(&self) {
        self.screen_style_coverage.take();
        self.consume_screen_style_work();
    }

    pub(super) fn consume_screen_style_work(&self) -> IndexSet<DomHandle> {
        *self.screen_style_invalidation.borrow_mut() = ScreenStyleInvalidation::Clean;
        self.screen_damage_nodes.take()
    }

    pub(super) fn screen_uses_lazy_pseudo(&self, handle: DomHandle) -> bool {
        self.screen_style_coverage
            .borrow()
            .as_ref()
            .is_some_and(|coverage| coverage.lazy_pseudo_owners.contains(&handle))
    }

    #[cfg(test)]
    pub(super) fn retained_style_system_generation(&self) -> u64 {
        self.retained_style_system_generation.get()
    }

    pub(super) fn target_context_epoch(&self) -> u64 {
        self.target_context_epoch.get()
    }

    pub(super) fn generation_snapshot(&self) -> StyleDocumentGenerationSnapshot {
        StyleDocumentGenerationSnapshot {
            source_set_generation: self.source_set_generation(),
            computed_cache_generation: self.computed_cache_generation(),
            retained_style_system_generation: self.retained_style_system_generation.get(),
            target_context_epoch: self.target_context_epoch(),
        }
    }

    pub(super) fn note_style_input_change(&self) {
        self.note_visual_input_change();
        self.bump_computed_context_epoch();
    }

    pub(super) fn visual_input_generation(&self) -> u64 {
        self.visual_input_generation.get()
    }

    pub(super) fn note_visual_input_change(&self) {
        self.visual_input_generation
            .set(self.visual_input_generation.get().saturating_add(1));
    }

    /// Cache invalidation during style computation is not a new source input.
    pub(super) fn bump_computed_context_epoch(&self) {
        self.target_context_epoch
            .set(self.target_context_epoch().saturating_add(1));
    }

    #[cfg(debug_assertions)]
    pub(super) fn complete_computed_style_observation(
        &self,
        document: DomHandle,
        input_epochs_before_read: &ComputedStyleObservationInputEpochs,
        input_epochs_after_read: ComputedStyleObservationInputEpochs,
    ) {
        let Some(world) =
            self.try_with_retained_style_system(|retained| CompletedStyleObservationWorld {
                retained_style_system_generation: self.retained_style_system_generation.get(),
                stylist_identity: retained.stylist_identity,
                key: retained.key.clone(),
            })
        else {
            return;
        };
        let current = CompletedStyleObservation {
            input_epochs_after_read,
            world,
        };
        let mut previous = self.completed_style_observation.borrow_mut();
        if previous.as_ref().is_some_and(|previous| {
            verify_completed_style_observation_transition(
                document,
                previous,
                input_epochs_before_read,
                &current,
            )
        }) {
            #[cfg(test)]
            self.completed_style_observation_stability_checks.set(
                self.completed_style_observation_stability_checks
                    .get()
                    .saturating_add(1),
            );
        }
        *previous = Some(current);
    }

    #[cfg(all(test, debug_assertions))]
    pub(super) fn completed_style_observation_stability_check_count(&self) -> u64 {
        self.completed_style_observation_stability_checks.get()
    }

    pub(super) fn clear_retained_style_system(&self) {
        self.invalidate_screen_style_comparisons();
        self.retained_style_system.borrow_mut().take();
        self.style_paths.clear();
        self.clear_source_dirty_scopes();
        self.clear_selector_caches();
    }

    pub(super) fn clear_selector_caches(&self) {
        *self.selector_caches.borrow_mut() = SelectorCaches::default();
    }

    pub(super) fn take_selector_caches(&self) -> SelectorCaches {
        std::mem::take(&mut *self.selector_caches.borrow_mut())
    }

    pub(super) fn replace_selector_caches(&self, selector_caches: SelectorCaches) {
        *self.selector_caches.borrow_mut() = selector_caches;
    }

    pub(super) fn record_source_dirty_scope(
        &self,
        scope_id: StyleScopeId,
        reason: StyleSourceDirtyReason,
        source_ids: impl IntoIterator<Item = StyleSourceId>,
        roots: impl IntoIterator<Item = DomHandle>,
    ) {
        self.source_dirty_scopes
            .record_scope(scope_id, reason, source_ids, roots);
    }

    pub(super) fn source_dirty_scope_snapshot(&self) -> StyleSourceDirtyScopeSnapshot {
        self.source_dirty_scopes.snapshot()
    }

    pub(super) fn clear_source_dirty_scopes(&self) {
        self.source_dirty_scopes.clear();
    }

    #[cfg(test)]
    pub(super) fn retained_style_system_matches(&self, key: &StyleWorldKey) -> bool {
        self.retained_style_system
            .borrow()
            .as_ref()
            .is_some_and(|retained| retained.key == *key)
    }

    pub(super) fn retained_style_system_is_current_for_observation(
        &self,
        viewport: StyleViewport,
        environment: StyloStyleEnvironment,
        quirks_mode: style::context::QuirksMode,
        tree_scope_versions: StyleTreeScopeVersions,
    ) -> bool {
        if self
            .source_dirty_scope_snapshot()
            .requires_retained_style_update()
        {
            return false;
        }
        self.retained_style_system
            .borrow()
            .as_ref()
            .is_some_and(|retained| {
                retained.key.matches_observation_environment(
                    viewport,
                    environment,
                    quirks_mode,
                    tree_scope_versions,
                )
            })
    }

    pub(super) fn set_retained_style_system(&self, mut retained: RetainedStyleSystem) {
        self.invalidate_screen_style_comparisons();
        let mut retained_slot = self.retained_style_system.borrow_mut();
        retained.stylesheet_resource_revision = retained_slot.as_ref().map_or(1, |previous| {
            if previous.stylesheet_resources == retained.stylesheet_resources {
                previous.stylesheet_resource_revision
            } else {
                previous.stylesheet_resource_revision.saturating_add(1)
            }
        });
        self.retained_style_system_generation.set(
            self.retained_style_system_generation
                .get()
                .saturating_add(1),
        );
        #[cfg(test)]
        self.retained_style_system_rebuilds
            .set(self.retained_style_system_rebuild_count().saturating_add(1));
        *retained_slot = Some(retained);
    }

    pub(super) fn update_retained_style_system_with_result<R>(
        &self,
        update: impl FnOnce(&mut RetainedStyleSystem) -> R,
    ) -> R {
        let result = {
            let mut retained = self.retained_style_system.borrow_mut();
            let retained = retained
                .as_mut()
                .expect("retained style system should exist before an incremental update");
            update(retained)
        };
        self.invalidate_screen_style_comparisons();
        self.retained_style_system_generation.set(
            self.retained_style_system_generation
                .get()
                .saturating_add(1),
        );
        #[cfg(test)]
        self.retained_style_system_updates
            .set(self.retained_style_system_update_count().saturating_add(1));
        result
    }

    #[cfg(test)]
    pub(super) fn retained_style_system_rebuild_count(&self) -> u64 {
        self.retained_style_system_rebuilds.get()
    }

    #[cfg(test)]
    pub(super) fn retained_style_system_update_count(&self) -> u64 {
        self.retained_style_system_updates.get()
    }

    #[cfg(test)]
    pub(super) fn note_element_style_resolutions(&self, count: u64) {
        self.element_style_resolutions
            .set(self.element_style_resolutions.get().saturating_add(count));
    }

    #[cfg(not(test))]
    pub(super) fn note_element_style_resolutions(&self, _count: u64) {}

    #[cfg(test)]
    pub(super) fn element_style_resolution_count(&self) -> u64 {
        self.element_style_resolutions.get()
    }

    #[cfg(test)]
    pub(super) fn note_ancestor_style_validation_visits(&self, count: u64) {
        self.ancestor_style_validation_visits.set(
            self.ancestor_style_validation_visits
                .get()
                .saturating_add(count),
        );
    }

    #[cfg(not(test))]
    pub(super) fn note_ancestor_style_validation_visits(&self, _count: u64) {}

    #[cfg(test)]
    pub(super) fn ancestor_style_validation_visit_count(&self) -> u64 {
        self.ancestor_style_validation_visits.get()
    }

    pub(super) fn with_shadow_cascade_data<R>(
        &self,
        callback: impl FnOnce(&[(DomHandle, ServoArc<CascadeData>)]) -> R,
    ) -> R {
        self.with_retained_style_system(|retained| callback(&retained.shadow_cascade_data))
    }

    pub(super) fn try_with_retained_style_system<R>(
        &self,
        callback: impl FnOnce(&RetainedStyleSystem) -> R,
    ) -> Option<R> {
        let retained = self.retained_style_system.borrow();
        retained.as_ref().map(callback)
    }

    pub(super) fn with_retained_style_system<R>(
        &self,
        callback: impl FnOnce(&RetainedStyleSystem) -> R,
    ) -> R {
        let retained = self.retained_style_system.borrow();
        let retained = retained
            .as_ref()
            .expect("retained style system should be prepared before resolving styles");
        callback(retained)
    }

    pub(super) fn stylesheet_resource_snapshot(
        &self,
        document: DomHandle,
    ) -> Option<StylesheetResourceSnapshot> {
        self.try_with_retained_style_system(|retained| {
            retained
                .stylesheet_resources
                .snapshot(StylesheetResourceGeneration::new(
                    document,
                    retained.stylesheet_resource_revision,
                ))
        })
    }
}

#[cfg(debug_assertions)]
fn verify_completed_style_observation_transition(
    document: DomHandle,
    previous: &CompletedStyleObservation,
    input_epochs_before_read: &ComputedStyleObservationInputEpochs,
    current: &CompletedStyleObservation,
) -> bool {
    if previous.input_epochs_after_read != *input_epochs_before_read {
        return false;
    }
    debug_assert_eq!(
        current.world, previous.world,
        "completed style observations changed the retained world for {document:?} without an intervening DOM, style-source, style-context, or style-viewport input change"
    );
    true
}

#[cfg(all(test, debug_assertions))]
mod observation_invariant_tests {
    use style::context::QuirksMode;

    use super::*;

    fn input_epochs(dom_version: u64) -> ComputedStyleObservationInputEpochs {
        ComputedStyleObservationInputEpochs {
            dom_version,
            style_viewport_generation: 4,
            tree_scope_versions: StyleTreeScopeVersions::for_test(5),
            document_generations: vec![(DomHandle::new(1), 2, 3)],
        }
    }

    fn world(generation: u64) -> CompletedStyleObservationWorld {
        CompletedStyleObservationWorld {
            retained_style_system_generation: generation,
            stylist_identity: 9,
            key: StyleWorldKey {
                viewport_width_bits: 800.0_f32.to_bits(),
                viewport_height_bits: 600.0_f32.to_bits(),
                screen_width_bits: 800.0_f32.to_bits(),
                screen_height_bits: 600.0_f32.to_bits(),
                device_pixel_ratio_bits: 1.0_f32.to_bits(),
                environment: StyloStyleEnvironment::default(),
                quirks_mode: QuirksMode::NoQuirks,
                tree_scope_versions: None,
            },
        }
    }

    #[test]
    #[should_panic(expected = "without an intervening DOM, style-source, style-context")]
    fn unchanged_inputs_reject_a_different_completed_world() {
        let epochs = input_epochs(1);
        let previous = CompletedStyleObservation {
            input_epochs_after_read: epochs.clone(),
            world: world(7),
        };
        let current = CompletedStyleObservation {
            input_epochs_after_read: epochs.clone(),
            world: world(8),
        };
        verify_completed_style_observation_transition(
            DomHandle::new(1),
            &previous,
            &epochs,
            &current,
        );
    }

    #[test]
    fn changed_inputs_allow_one_completed_world_update() {
        let previous = CompletedStyleObservation {
            input_epochs_after_read: input_epochs(1),
            world: world(7),
        };
        let current = CompletedStyleObservation {
            input_epochs_after_read: input_epochs(2),
            world: world(8),
        };
        assert!(!verify_completed_style_observation_transition(
            DomHandle::new(1),
            &previous,
            &input_epochs(2),
            &current,
        ));
    }
}
