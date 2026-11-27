pub(crate) use moli_css_parse::ensure_stylo_browser_compat_prefs;
#[cfg(test)]
use std::sync::Arc as StdArc;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::{document_runtime::DomHandle, dom::native::DomHost};
use dom::ElementState as StyloElementState;
use moli_selector::{QueryContainerSizes, StyloDomStyleAdapter};
use style::servo::restyle_damage::ServoRestyleDamage;

pub(crate) struct ScreenProjectionStyleUpdates {
    pub(crate) damage: ServoRestyleDamage,
    pub(crate) styles: Vec<(
        DomHandle,
        style::servo_arc::Arc<style::properties::ComputedValues>,
    )>,
}

mod active_stylesheets;
#[cfg(test)]
pub(crate) use active_stylesheets::{
    exact_rule_change_notification_count_for_test, full_cascade_update_fallback_count_for_test,
    reset_live_stylesheet_update_counts_for_test,
};
mod cache;
mod cause;
mod cleanup;
mod computed;
#[cfg(test)]
pub(crate) use computed::take_style_resolution_context_count_for_test;
mod document_world;
mod drain;
mod invalidation;
pub(crate) mod media_list;
mod mutation_effect;
mod native_invalidation;
mod property_metadata;
mod registered_properties;
mod retained;
mod style_paths;
pub(crate) use stylesheet::{
    NativeStylesheetFontFaceProjection, NativeStylesheetFontFaceRuleProjection,
    StylesheetFontFaceProjection, StylesheetFontFaceRuleProjection, initial_font_size_px,
    native_font_face_projection_for_stylesheet, native_font_face_rules_for_stylesheet,
};
#[cfg(test)]
pub(crate) use stylesheet::{
    author_source_text_parse_count_for_test, reset_author_source_text_parse_count_for_test,
};
mod runtime_invalidation;
mod shadow_scopes;
mod snapshot;
mod source;
mod source_dirty;
mod source_document;
mod source_id;
mod source_key;
mod source_owner;
mod source_owner_text;
mod state;
#[cfg(debug_assertions)]
pub(crate) use state::ComputedStyleObservationInputEpochs;
pub(crate) use state::ScreenStyleCoverage;
use state::{ScreenRestyleTargets, ScreenStyleInvalidation};
pub(crate) use style_paths::StyleInvalidationScope;
mod stylesheet;
mod stylesheet_resources;
#[cfg(test)]
mod tests;
mod ua;
mod world_environment;
mod world_key;
mod world_lifecycle;
mod world_trace;
mod world_update;

use cleanup::StyleInvalidationCleanup;
pub(crate) use computed::{
    ComputedDisplayKind, ComputedRenderedStyleFacts, ComputedTextTransformKind,
    ComputedTextWrapModeKind, ComputedWhiteSpaceCollapseKind, MarkerStyleDecision,
    StyleObservationSnapshot, StyleResolutionPass, StyloAnonymousBoxKind,
    StyloComputedStyleSnapshot, layout_damage_for_style_change, marker_style_decision,
    serialize_resolved_computed_property,
};
use document_world::{DocumentStyleWorld, DocumentStyleWorlds};
pub(in crate::style_engine) use drain::StyleInvalidationDrainBoundary;
pub(crate) use drain::StyleInvalidationTurnExitBoundary;
pub(crate) use mutation_effect::{StyleMutationEffect, normalized_style_attribute_name};
pub(crate) use property_metadata::{
    computed_longhand_count, computed_longhand_name_at, computed_property_is_queryable,
};
pub(crate) use registered_properties::{
    CssCustomPropertyRegistration, CssCustomPropertyRegistrationError,
    CssCustomPropertyRegistrationRecord,
};
pub(crate) use source::adopted::AdoptedStyleSheetInstallation;
pub(crate) use source::inline::InlineStyleCspState;
pub(crate) use source::store::{
    OwnerStyleSheetSource, StylesheetFontFaceDescriptor, StyloStylesheetSource,
};
pub(crate) use source_document::OwnedStyleSourceDocumentContext;
use source_document::StyleSourceDocumentContext;
#[cfg(test)]
use source_id::StyleScopeId;
pub(crate) use source_id::StyleSourceId;
pub(crate) use source_owner::{
    link_rel_qualifies_as_stylesheet, stylesheet_owner_type_is_supported,
};
pub(crate) use stylesheet_resources::{StylesheetResourceGeneration, StylesheetResourceSnapshot};
#[cfg(test)]
use stylesheet_resources::{
    reset_stylesheet_resource_manifest_build_count_for_test,
    stylesheet_resource_manifest_build_count_for_test,
};
pub(crate) use world_environment::{
    StyleTreeScopeVersions, StyleViewport, StyleWorldEnvironment, StyloStyleEnvironment,
};
#[cfg(test)]
use world_key::StyleWorldKey;
pub(crate) use world_update::{
    FullStyleWorldSnapshot, IncrementalStyleWorldUpdate, PreparedStyleWorldUpdate,
    StyleWorldUpdate, StyleWorldUpdatePlan,
};

/// Page-level facade for document-owned style state.
///
/// The selector/query adapter in `moli-selector` deliberately stays
/// query-only. The facade owns cross-document lookup indexes plus the Stylo
/// side-table adapter; style sources, retained state, invalidation state, and
/// pseudo cache live in per-document worlds.
pub(crate) struct MoliStyleEngine {
    dom_adapter: StyloDomStyleAdapter,
    document_worlds: DocumentStyleWorlds,
    author_styles_disabled: bool,
    owner_stylesheet_source_documents: RefCell<HashMap<DomHandle, DomHandle>>,
    linked_stylesheet_owner_documents: RefCell<HashMap<DomHandle, DomHandle>>,
    inline_style_metadata_documents: RefCell<HashMap<DomHandle, DomHandle>>,
}

impl Default for MoliStyleEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MoliStyleEngine {
    pub(crate) fn new() -> Self {
        Self::new_with_author_styles_disabled(false)
    }

    pub(crate) fn new_with_author_styles_disabled(author_styles_disabled: bool) -> Self {
        ensure_stylo_browser_compat_prefs();
        Self {
            dom_adapter: StyloDomStyleAdapter::new_with_author_styles_enabled(
                !author_styles_disabled,
            ),
            document_worlds: DocumentStyleWorlds::new(),
            author_styles_disabled,
            owner_stylesheet_source_documents: RefCell::new(HashMap::new()),
            linked_stylesheet_owner_documents: RefCell::new(HashMap::new()),
            inline_style_metadata_documents: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn author_styles_disabled(&self) -> bool {
        self.author_styles_disabled
    }

    pub(crate) fn author_shared_lock(&self) -> style::shared_lock::SharedRwLock {
        self.dom_adapter.shared_lock().clone()
    }

    pub(crate) fn documents_with_adopted_style_sheets(&self) -> Vec<DomHandle> {
        self.document_worlds.documents_with_adopted_style_sheets()
    }

    pub(crate) fn document_has_adopted_style_sheet_sources(&self, document: DomHandle) -> bool {
        self.document_worlds
            .active_world(document)
            .is_some_and(|world| {
                world
                    .adopted_style_sheet_sources
                    .borrow()
                    .document_source_count(document)
                    != 0
            })
    }

    pub(crate) fn shadow_root_has_adopted_style_sheet_sources(
        &self,
        host: &DomHost,
        root: DomHandle,
    ) -> bool {
        self.active_owner_document_world(host, root)
            .is_some_and(|world| {
                world
                    .adopted_style_sheet_sources
                    .borrow()
                    .shadow_root_source_count(root)
                    != 0
            })
    }

    pub(in crate::style_engine) fn world_for_document(
        &self,
        document: DomHandle,
    ) -> Rc<DocumentStyleWorld> {
        self.document_worlds.for_document(document)
    }

    pub(in crate::style_engine) fn owner_document_world(
        &self,
        host: &DomHost,
        handle: DomHandle,
    ) -> Option<Rc<DocumentStyleWorld>> {
        host.owner_document_handle(handle)
            .map(|document| self.world_for_document(document))
    }

    pub(in crate::style_engine) fn active_owner_document_world(
        &self,
        host: &DomHost,
        handle: DomHandle,
    ) -> Option<Rc<DocumentStyleWorld>> {
        host.owner_document_handle(handle)
            .and_then(|document| self.document_worlds.active_world(document))
    }

    pub(crate) fn computed_cache_generation_for_document(&self, document: DomHandle) -> u64 {
        self.document_worlds
            .generation_snapshot_for_document(document)
            .computed_cache_generation
    }

    pub(crate) fn has_screen_style_invalidation(&self, document: DomHandle) -> bool {
        self.document_worlds
            .active_world(document)
            .is_some_and(|world| {
                // Queued native snapshots still need to be drained before
                // they can contribute affected targets to screen restyles.
                !world.pending_invalidations.borrow().is_empty()
                    || !matches!(
                        *world.document_state.screen_style_invalidation.borrow(),
                        ScreenStyleInvalidation::Clean
                    )
            })
    }

    /// The scopes to settle while the existing screen layout may still be used.
    pub(crate) fn screen_restyle_targets(
        &self,
        host: &DomHost,
        document: DomHandle,
        supported_damage: ServoRestyleDamage,
    ) -> Option<ScreenRestyleTargets> {
        let Some(world) = self.document_worlds.active_world(document) else {
            return Some(ScreenRestyleTargets::default());
        };
        let state = &world.document_state;
        state.finish_screen_style_invalidation();
        if supported_damage.is_empty() && state.screen_needs_layout() {
            return None;
        }
        state.screen_restyle_targets(host, document)
    }

    pub(crate) fn screen_style_damage_is_within(
        &self,
        document: DomHandle,
        supported_damage: ServoRestyleDamage,
    ) -> bool {
        let Some(world) = self.document_worlds.active_world(document) else {
            return false;
        };
        let state = &world.document_state;
        state.finish_screen_style_invalidation();
        state.screen_style_dependencies_are_current()
            && !state.screen_restyle_is_full()
            && state.screen_damage_nodes.borrow().iter().all(|&handle| {
                self.dom_adapter
                    .element_restyle_damage(document, handle)
                    .is_some_and(|damage| supported_damage.contains(damage))
            })
    }

    /// Read canonical damage only after settling each affected path.
    pub(crate) fn screen_projection_style_updates(
        &self,
        document: DomHandle,
        supported_damage: ServoRestyleDamage,
    ) -> Option<ScreenProjectionStyleUpdates> {
        let mut updates = ScreenProjectionStyleUpdates {
            damage: ServoRestyleDamage::empty(),
            styles: Vec::new(),
        };
        let Some(world) = self.document_worlds.active_world(document) else {
            return Some(updates);
        };
        let state = &world.document_state;
        state.finish_screen_style_invalidation();
        if state.screen_restyle_is_full() {
            return None;
        }
        for &handle in &*state.screen_damage_nodes.borrow() {
            let damage = self.dom_adapter.element_restyle_damage(document, handle)?;
            if !supported_damage.contains(damage) {
                return None;
            }
            let styles = self.dom_adapter.clone_clean_element_styles(
                document,
                handle,
                state.style_paths.generation(),
            )?;
            let (primary, before, after) = StyloComputedStyleSnapshot::from_element_styles(&styles)
                .into_element_computed_values();
            if before.is_some() || after.is_some() {
                return None;
            }
            updates.damage |= damage;
            updates.styles.push((handle, primary));
        }
        Some(updates)
    }

    #[cfg(test)]
    pub(crate) fn observed_screen_damage_for_test(
        &self,
        document: DomHandle,
    ) -> ServoRestyleDamage {
        let world = self.world_for_document(document);
        world
            .document_state
            .screen_damage_nodes
            .borrow()
            .iter()
            .filter_map(|&handle| self.dom_adapter.element_restyle_damage(document, handle))
            .fold(ServoRestyleDamage::empty(), |all, damage| all | damage)
    }

    pub(crate) fn screen_layout_style_targets(
        &self,
        host: &DomHost,
        document: DomHandle,
    ) -> Option<Vec<(DomHandle, StyleInvalidationScope)>> {
        let world = self.document_worlds.active_world(document)?;
        let state = &world.document_state;
        if !state.screen_style_dependencies_are_current() {
            return None;
        }
        state.finish_screen_style_invalidation();
        let mut targets = state.screen_restyle_targets(host, document)?.targets;
        targets.extend(
            state
                .screen_damage_nodes
                .borrow()
                .iter()
                .copied()
                .filter(|&target| {
                    host.is_connected(target)
                        && host.owner_document_handle(target) == Some(document)
                })
                .map(|target| (target, StyleInvalidationScope::Subtree)),
        );
        Some(targets)
    }

    #[cfg(test)]
    pub(crate) fn screen_style_damage_for_node_for_test(
        &self,
        document: DomHandle,
        handle: DomHandle,
    ) -> Option<ServoRestyleDamage> {
        self.dom_adapter
            .element_restyle_damage(document, handle)
            .filter(|damage| !damage.is_empty())
    }

    pub(crate) fn begin_screen_layout(&self, document: DomHandle) {
        if let Some(world) = self.document_worlds.active_world(document) {
            world.document_state.begin_screen_layout();
        }
    }

    pub(crate) fn publish_screen_styles(&self, document: DomHandle, coverage: ScreenStyleCoverage) {
        if let Some(world) = self.document_worlds.active_world(document) {
            world.document_state.publish_screen_styles(coverage);
            self.dom_adapter
                .consume_restyle_damage(document, world.document_state.consume_screen_style_work());
        }
    }

    pub(crate) fn consume_screen_style_work(&self, document: DomHandle) {
        if let Some(world) = self.document_worlds.active_world(document) {
            self.dom_adapter
                .consume_restyle_damage(document, world.document_state.consume_screen_style_work());
        }
    }

    /// Visual output changes with source inputs, not with materialization of
    /// the computed cache (for example while observing temporary print media).
    pub(crate) fn visual_style_input_generations(
        &self,
        documents: impl IntoIterator<Item = DomHandle>,
    ) -> Vec<(DomHandle, u64, u64)> {
        self.document_worlds.visual_input_generations(documents)
    }

    pub(crate) fn stylesheet_resource_snapshot_for_document(
        &self,
        document: DomHandle,
    ) -> Option<StylesheetResourceSnapshot> {
        self.document_worlds
            .active_world(document)?
            .document_state
            .stylesheet_resource_snapshot(document)
    }

    /// Returns the source projection already installed in the retained world
    /// for compatibility queries that need stylesheet provenance.
    ///
    /// This clones cheap source handles; it does not walk stylesheet owners,
    /// serialize CSS, parse a sheet, or mutate the style world.
    pub(crate) fn retained_stylesheet_query_snapshot_for_document(
        &self,
        document: DomHandle,
    ) -> Option<Rc<FullStyleWorldSnapshot>> {
        let world = self.document_worlds.active_world(document)?;
        world
            .document_state
            .try_with_retained_style_system(|retained| {
                Rc::new(FullStyleWorldSnapshot {
                    document_stylesheet_sources: retained
                        .document_stylesheets
                        .entries()
                        .iter()
                        .map(|entry| entry.source().clone())
                        .collect(),
                    shadow_stylesheet_sources: retained
                        .shadow_scopes
                        .iter()
                        .map(|scope| {
                            (
                                scope.root(),
                                scope
                                    .active_stylesheets()
                                    .entries()
                                    .iter()
                                    .map(|entry| entry.source().clone())
                                    .collect(),
                            )
                        })
                        .collect(),
                    script_custom_property_registrations: retained
                        .script_custom_property_registrations
                        .clone(),
                    environment: retained.key.environment,
                    quirks_mode: retained.key.quirks_mode,
                })
            })
    }

    #[cfg(debug_assertions)]
    pub(crate) fn computed_style_observation_input_epochs(
        &self,
        documents: impl IntoIterator<Item = DomHandle>,
        dom_version: u64,
        style_viewport_generation: u64,
        tree_scope_versions: StyleTreeScopeVersions,
    ) -> ComputedStyleObservationInputEpochs {
        let document_generations = self
            .document_worlds
            .observation_generations(documents)
            .into_iter()
            .map(
                |(document, source_set_generation, _, target_context_epoch)| {
                    (document, source_set_generation, target_context_epoch)
                },
            )
            .collect();
        ComputedStyleObservationInputEpochs {
            dom_version,
            style_viewport_generation,
            tree_scope_versions,
            document_generations,
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn complete_computed_style_observation(
        &self,
        document: DomHandle,
        input_epochs_before_read: &ComputedStyleObservationInputEpochs,
        input_epochs_after_read: ComputedStyleObservationInputEpochs,
    ) {
        self.world_for_document(document)
            .document_state
            .complete_computed_style_observation(
                document,
                input_epochs_before_read,
                input_epochs_after_read,
            );
    }

    #[cfg(test)]
    pub(crate) fn source_set_generation_for_document_for_test(&self, document: DomHandle) -> u64 {
        self.world_for_document(document)
            .document_state
            .source_set_generation()
    }

    /// Source commits only; computed-style materialization and invalidation
    /// cleanup advance other counters and must not invalidate their own layout.
    pub(crate) fn source_set_generation_for_document(&self, document: DomHandle) -> u64 {
        self.document_worlds
            .generation_snapshot_for_document(document)
            .source_set_generation
    }

    #[cfg(test)]
    pub(crate) fn computed_cache_generation_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.computed_cache_generation_for_document(document)
    }

    #[cfg(test)]
    pub(crate) fn retained_style_system_generation_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.world_for_document(document)
            .document_state
            .retained_style_system_generation()
    }

    #[cfg(all(test, debug_assertions))]
    pub(crate) fn completed_style_observation_stability_check_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.world_for_document(document)
            .document_state
            .completed_style_observation_stability_check_count()
    }

    pub(crate) fn target_context_epoch_for_document(&self, document: DomHandle) -> u64 {
        self.document_worlds
            .generation_snapshot_for_document(document)
            .target_context_epoch
    }

    #[cfg(test)]
    pub(crate) fn target_context_epoch_for_document_for_test(&self, document: DomHandle) -> u64 {
        self.target_context_epoch_for_document(document)
    }

    pub(crate) fn bump_target_context_epoch_for_document(&self, document: DomHandle) {
        self.world_for_document(document)
            .document_state
            .note_style_input_change();
    }

    #[cfg(test)]
    pub(crate) fn computed_style_cache_entry_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> usize {
        self.document_worlds
            .active_world(document)
            .map_or(0, |world| world.pseudo_style_cache.len())
    }

    #[cfg(test)]
    pub(crate) fn computed_style_publication_generation_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.world_for_document(document)
            .pseudo_style_cache
            .write_generation()
    }

    #[cfg(test)]
    pub(crate) fn computed_style_cache_contains_handle_for_document_for_test(
        &self,
        document: DomHandle,
        handle: DomHandle,
    ) -> bool {
        self.world_for_document(document)
            .pseudo_style_cache
            .contains_handle_for_test(handle)
    }

    #[cfg(test)]
    pub(crate) fn computed_style_cache_entry_count_for_handle_for_document_for_test(
        &self,
        document: DomHandle,
        handle: DomHandle,
    ) -> usize {
        self.world_for_document(document)
            .pseudo_style_cache
            .entry_count_for_handle_for_test(handle)
    }

    #[cfg(test)]
    pub(crate) fn retained_style_invalidation_root_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> usize {
        self.world_for_document(document)
            .document_state
            .style_paths
            .root_count()
    }

    #[cfg(test)]
    pub(crate) fn style_invalidation_generation_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.world_for_document(document)
            .document_state
            .style_paths
            .generation()
    }

    #[cfg(test)]
    pub(crate) fn style_invalidation_path_node_visit_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.world_for_document(document)
            .document_state
            .style_paths
            .path_node_visit_count()
    }

    #[cfg(test)]
    pub(crate) fn ancestor_style_validation_visit_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.world_for_document(document)
            .document_state
            .ancestor_style_validation_visit_count()
    }

    #[cfg(test)]
    pub(crate) fn source_dirty_scope_source_ids_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> Vec<StyleSourceId> {
        self.world_for_document(document)
            .document_state
            .source_dirty_scope_snapshot()
            .source_ids_vec()
    }

    #[cfg(test)]
    pub(in crate::style_engine) fn source_dirty_scope_reasons_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> Vec<source_dirty::StyleSourceDirtyReason> {
        self.world_for_document(document)
            .document_state
            .source_dirty_scope_snapshot()
            .reasons_vec()
    }

    #[cfg(test)]
    pub(in crate::style_engine) fn source_dirty_scope_ids_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> Vec<StyleScopeId> {
        self.world_for_document(document)
            .document_state
            .source_dirty_scope_snapshot()
            .scope_ids_vec()
    }

    #[cfg(test)]
    pub(crate) fn source_dirty_scope_roots_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> Vec<DomHandle> {
        self.world_for_document(document)
            .document_state
            .source_dirty_scope_snapshot()
            .scoped_roots_vec()
    }

    #[cfg(test)]
    pub(crate) fn clear_retained_style_system_for_document_for_test(&self, document: DomHandle) {
        self.dom_adapter.clear_style_paths(document);
        self.world_for_document(document)
            .document_state
            .clear_retained_style_system();
    }

    #[cfg(test)]
    pub(crate) fn retained_style_system_rebuild_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.world_for_document(document)
            .document_state
            .retained_style_system_rebuild_count()
    }

    #[cfg(test)]
    pub(crate) fn retained_style_system_update_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.world_for_document(document)
            .document_state
            .retained_style_system_update_count()
    }

    #[cfg(test)]
    pub(crate) fn retained_stylist_identity_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.world_for_document(document)
            .document_state
            .with_retained_style_system(|retained| retained.stylist_identity)
    }

    #[cfg(test)]
    pub(crate) fn retained_stylist_flush_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.world_for_document(document)
            .document_state
            .with_retained_style_system(|retained| retained.stylist.num_rebuilds() as u64)
    }

    #[cfg(test)]
    pub(crate) fn retained_shadow_scope_flush_count_for_document_for_test(
        &self,
        document: DomHandle,
        root: DomHandle,
    ) -> Option<u64> {
        self.world_for_document(document)
            .document_state
            .with_retained_style_system(|retained| {
                retained
                    .shadow_scopes
                    .iter()
                    .find(|scope| scope.root() == root)
                    .map(shadow_scopes::ShadowScopeStyles::flush_count_for_test)
            })
    }

    #[cfg(test)]
    pub(crate) fn element_style_resolution_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.world_for_document(document)
            .document_state
            .element_style_resolution_count()
    }

    #[cfg(test)]
    pub(in crate::style_engine) fn retained_style_system_matches_for_document_for_test(
        &self,
        document: DomHandle,
        key: &StyleWorldKey,
    ) -> bool {
        self.world_for_document(document)
            .document_state
            .retained_style_system_matches(key)
    }

    #[cfg(test)]
    pub(in crate::style_engine) fn with_retained_style_system_for_document_for_test<R>(
        &self,
        document: DomHandle,
        callback: impl FnOnce(&state::RetainedStyleSystem) -> R,
    ) -> R {
        self.world_for_document(document)
            .document_state
            .with_retained_style_system(callback)
    }

    #[cfg(test)]
    pub(crate) fn pending_style_invalidation_work_item_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> usize {
        let world = self.world_for_document(document);
        usize::from(!world.pending_invalidations.borrow().is_empty())
    }

    #[cfg(test)]
    pub(crate) fn pending_structural_style_mutation_effect_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> usize {
        self.world_for_document(document)
            .pending_invalidations
            .borrow()
            .structural_effect_count_for_test()
    }

    pub(in crate::style_engine) fn invalidation_cleanup_for_world<'a>(
        &'a self,
        world: &'a DocumentStyleWorld,
    ) -> StyleInvalidationCleanup<'a> {
        StyleInvalidationCleanup::new(
            world.document,
            &self.dom_adapter,
            &world.pseudo_style_cache,
            &world.document_state,
        )
    }

    pub(crate) fn clear_for_document_replacement(&mut self, document: DomHandle) {
        self.clear_owner_document_indexes_for_document(document);
        self.dom_adapter.clear_element_data_for_document(document);
        let _ = self
            .dom_adapter
            .replace_query_container_sizes(document, QueryContainerSizes::new());
        self.dom_adapter
            .clear_inline_style_attributes_for_document(document);
        self.dom_adapter
            .clear_shadow_cascade_data_for_document(document);
        self.document_worlds
            .clear_for_document_replacement(document);
    }

    /// Retires all style state owned by a Document that left its browsing
    /// context. Unlike `clear_for_document_replacement`, this removes the world
    /// itself because child/popup navigation installs a different Document.
    /// Same-handle `document.open()` replacement continues to use `clear`.
    pub(crate) fn retire_document_style_world(&mut self, document: DomHandle) -> bool {
        self.clear_owner_document_indexes_for_document(document);
        self.dom_adapter.clear_element_data_for_document(document);
        let _ = self
            .dom_adapter
            .replace_query_container_sizes(document, QueryContainerSizes::new());
        self.dom_adapter
            .clear_inline_style_attributes_for_document(document);
        self.dom_adapter
            .clear_shadow_cascade_data_for_document(document);
        self.document_worlds.retire_document(document)
    }

    pub(crate) fn replace_layout_container_sizes(
        &self,
        host: &DomHost,
        document: DomHandle,
        sizes: QueryContainerSizes,
    ) -> Option<QueryContainerSizes> {
        let (previous, changed) = self
            .dom_adapter
            .replace_query_container_sizes(document, sizes)?;
        let world = self.world_for_document(document);
        world.document_state.invalidate_screen_style_comparisons();
        self.invalidation_cleanup_for_world(&world)
            .invalidate_subtrees(host, changed);
        Some(previous)
    }

    #[cfg(test)]
    pub(crate) fn active_document_style_world_count_for_test(&self) -> usize {
        self.document_worlds.active_world_count()
    }

    #[cfg(test)]
    pub(crate) fn document_style_world_is_active_for_test(&self, document: DomHandle) -> bool {
        self.document_worlds.contains_active_world(document)
    }

    /// Drops the exact target's canonical style and cached pseudo values after
    /// it leaves every rendered style context.
    ///
    /// This is deliberately target-local. Subtree invalidation remains lazy;
    /// a disconnected `getComputedStyle()` read must not reintroduce the old
    /// document-wide scan merely to retire the one target it observes.
    pub(crate) fn retire_computed_style_for_inactive_handle(
        &self,
        host: &DomHost,
        handle: DomHandle,
    ) {
        if let Some(world) = self.active_owner_document_world(host, handle) {
            if host.is_connected(handle) {
                world.document_state.invalidate_screen_style_comparisons();
            }
            if self
                .dom_adapter
                .style_path(world.document, handle)
                .is_some()
            {
                world.document_state.record_style_targets(
                    host,
                    &self.dom_adapter,
                    world.document,
                    [(handle, StyleInvalidationScope::Subtree)],
                );
            }
        }
        self.dom_adapter.clear_element_data(handle);
        self.document_worlds
            .invalidate_cached_pseudos_for_handle(handle);
    }

    fn clear_owner_document_indexes_for_document(&self, document: DomHandle) {
        retain_owner_documents_except_document(
            &mut self.owner_stylesheet_source_documents.borrow_mut(),
            document,
        );
        retain_owner_documents_except_document(
            &mut self.linked_stylesheet_owner_documents.borrow_mut(),
            document,
        );
        retain_owner_documents_except_document(
            &mut self.inline_style_metadata_documents.borrow_mut(),
            document,
        );
    }

    #[cfg(test)]
    fn ensure_retained_style_system_for_document(
        &self,
        host: &DomHost,
        document: DomHandle,
        key: StyleWorldKey,
        inputs: &FullStyleWorldSnapshot,
    ) {
        computed::ensure_retained_style_system_for_document_for_test(
            self, host, document, key, inputs,
        );
    }

    pub(crate) fn drain_pending_style_invalidations_for_computed_style_read_with_document_context(
        &self,
        host: &DomHost,
        owner_document: DomHandle,
        document_context: StyleSourceDocumentContext<'_>,
    ) {
        for document in document_context.documents_with_owner(owner_document) {
            self.drain_pending_style_invalidations_for_document_and_boundary(
                host,
                document,
                StyleInvalidationDrainBoundary::ComputedStyleRead,
            );
        }
    }

    pub(crate) fn drain_pending_style_invalidations_for_turn_exit_with_document_context(
        &self,
        host: &DomHost,
        document_context: StyleSourceDocumentContext<'_>,
        boundary: StyleInvalidationTurnExitBoundary,
    ) {
        for document in document_context.documents() {
            self.drain_pending_style_invalidations_for_document_and_boundary(
                host,
                document,
                boundary.into(),
            );
        }
    }

    fn drain_pending_style_invalidations_for_document_and_boundary(
        &self,
        host: &DomHost,
        document: DomHandle,
        boundary: StyleInvalidationDrainBoundary,
    ) {
        let world = self.world_for_document(document);
        let pending = world.pending_invalidations.take();
        if !pending.is_empty() && moli_trace::style_invalidation_trace_enabled() {
            tracing::info!(target: "moli_style_invalidation", ?document, ?boundary, "drain DOM invalidation");
        }
        pending.drain(self, &world, host);
        world.document_state.finish_screen_style_invalidation();
    }

    #[cfg(test)]
    pub(crate) fn drain_pending_style_invalidations_for_document_for_test(
        &self,
        host: &DomHost,
        document: DomHandle,
    ) {
        self.drain_pending_style_invalidations_for_document_and_boundary(
            host,
            document,
            StyleInvalidationDrainBoundary::TestExplicit,
        );
    }

    pub(crate) fn retained_current_element_state(
        &self,
        host: &DomHost,
        element: DomHandle,
    ) -> Option<StyloElementState> {
        computed::retained_current_element_state(self, host, element)
    }

    pub(crate) fn has_retained_style_attribute_selector(
        &self,
        host: &DomHost,
        handle: DomHandle,
    ) -> bool {
        let Some(world) = self.active_owner_document_world(host, handle) else {
            return false;
        };
        // The active cascade already indexes attribute selectors. A pending
        // source refresh can introduce new dependencies before the next read.
        world
            .document_state
            .source_dirty_scope_snapshot()
            .requires_retained_style_update()
            || host.has_shadow_roots()
            || world
                .document_state
                .try_with_retained_style_system(|retained| {
                    native_invalidation::style_attribute_in_selector(&retained.stylist)
                })
                != Some(false)
    }

    #[cfg(test)]
    pub(crate) fn computed_style_property_value(
        &self,
        host: &DomHost,
        document_url: &url::Url,
        handle: DomHandle,
        property: &str,
        pseudo_element: Option<&str>,
        inputs: &FullStyleWorldSnapshot,
        viewport: impl Into<StyleViewport>,
    ) -> Option<String> {
        let owner_document = host.owner_document_handle(handle)?;
        self.computed_style_property_value_with_document_context(
            host,
            document_url,
            handle,
            property,
            pseudo_element,
            inputs,
            StyleSourceDocumentContext::for_root_document(owner_document),
            owner_document,
            viewport.into(),
        )
    }

    pub(crate) fn computed_style_property_value_with_document_context(
        &self,
        host: &DomHost,
        document_url: &url::Url,
        handle: DomHandle,
        property: &str,
        pseudo_element: Option<&str>,
        inputs: &FullStyleWorldSnapshot,
        document_context: StyleSourceDocumentContext<'_>,
        read_document: DomHandle,
        viewport: StyleViewport,
    ) -> Option<String> {
        computed::computed_style_property_value(
            self,
            host,
            document_url,
            handle,
            property,
            pseudo_element,
            inputs,
            document_context,
            read_document,
            viewport,
        )
    }

    pub(crate) fn computed_style_snapshot_after_style_update_with_document_context(
        &self,
        host: &DomHost,
        document_url: &url::Url,
        handle: DomHandle,
        inputs: &FullStyleWorldSnapshot,
        document_context: StyleSourceDocumentContext<'_>,
        read_document: DomHandle,
        viewport: StyleViewport,
    ) -> Option<StyloComputedStyleSnapshot> {
        computed::computed_style_snapshot_after_style_update(
            self,
            host,
            document_url,
            handle,
            inputs,
            document_context,
            read_document,
            viewport,
        )
    }

    pub(crate) fn style_world_update_plan_for_observation(
        &self,
        host: &DomHost,
        document: DomHandle,
        viewport: StyleViewport,
        environment: StyloStyleEnvironment,
        tree_scope_versions: StyleTreeScopeVersions,
    ) -> Option<StyleWorldUpdatePlan> {
        let world = self.world_for_document(document);
        computed::style_world_update_plan_for_observation(
            host,
            &world,
            viewport,
            environment,
            computed::document_quirks_mode(host, document),
            tree_scope_versions,
        )
    }

    pub(crate) fn with_style_resolution_pass<R>(
        &self,
        host: &DomHost,
        document_url: &url::Url,
        document: DomHandle,
        update: Option<&PreparedStyleWorldUpdate>,
        resolve: impl FnOnce(&mut computed::StyleResolutionPass<'_, '_>) -> R,
    ) -> R {
        if let Some(update) = update {
            let world = self.world_for_document(document);
            computed::apply_style_world_update(self, host, &world, update);
        }
        computed::with_style_resolution_pass(self, host, document_url, document, resolve)
    }

    pub(crate) fn computed_style_snapshot_from_current_observation(
        &self,
        host: &DomHost,
        document_url: &url::Url,
        handle: DomHandle,
        environment: StyloStyleEnvironment,
        document_context: StyleSourceDocumentContext<'_>,
        read_document: DomHandle,
        viewport: StyleViewport,
        tree_scope_versions: StyleTreeScopeVersions,
    ) -> StyleObservationSnapshot {
        computed::computed_style_snapshot_from_current_observation(
            self,
            host,
            document_url,
            handle,
            environment,
            document_context,
            read_document,
            viewport,
            tree_scope_versions,
        )
    }

    pub(crate) fn computed_style_snapshot_after_world_update(
        &self,
        host: &DomHost,
        document_url: &url::Url,
        handle: DomHandle,
        inputs: &PreparedStyleWorldUpdate,
        document_context: StyleSourceDocumentContext<'_>,
        read_document: DomHandle,
    ) -> Option<StyloComputedStyleSnapshot> {
        computed::computed_style_snapshot_after_world_update(
            self,
            host,
            document_url,
            handle,
            inputs,
            document_context,
            read_document,
        )
    }

    pub(crate) fn computed_pseudo_style_snapshot_from_current_observation(
        &self,
        host: &DomHost,
        document_url: &url::Url,
        handle: DomHandle,
        pseudo_element: &str,
        document_context: StyleSourceDocumentContext<'_>,
        read_document: DomHandle,
    ) -> Option<StyloComputedStyleSnapshot> {
        computed::computed_pseudo_style_snapshot_from_current_observation(
            self,
            host,
            document_url,
            handle,
            pseudo_element,
            document_context,
            read_document,
        )
    }
}

fn retain_owner_documents_except_document(
    owner_documents: &mut HashMap<DomHandle, DomHandle>,
    document: DomHandle,
) {
    owner_documents.retain(|_, owner_document| *owner_document != document);
}
