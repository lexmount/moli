#[cfg(test)]
use std::sync::Arc as StdArc;
use std::sync::Once;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::{document_runtime::DomHandle, dom::native::DomHost};
use dom::ElementState as StyloElementState;
#[cfg(test)]
use indexmap::IndexSet;
use moli_selector::StyloDomStyleAdapter;
#[cfg(test)]
use moli_selector::StyloRetainedStyleInvalidationQuery as RetainedStyleInvalidationQuery;
#[cfg(test)]
use moli_selector::StyloStyleSourceScope as StyleSourceScope;
#[cfg(test)]
use moli_selector::stylo_element_dependency_snapshot as style_element_dependency_snapshot;
#[cfg(test)]
use moli_selector::{
    StyloStylesheetSourceScopeFallbackInput, stylo_stylesheet_source_scope_fallback_roots,
};
#[cfg(test)]
use style::Atom;

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
mod document_world;
mod drain;
mod eligibility;
mod fallback;
mod invalidation;
mod lazy_invalidation;
pub(crate) mod media_list;
mod mutation_effect;
mod outcome;
mod pending_invalidation;
mod pending_mutation;
mod planner;
mod property_metadata;
mod query;
mod registered_properties;
mod request;
mod retained;
pub(crate) use stylesheet::{
    NativeStylesheetFontFaceProjection, NativeStylesheetFontFaceRuleProjection,
    StylesheetFontFaceProjection, StylesheetFontFaceRuleProjection,
    native_font_face_projection_for_stylesheet, native_font_face_rules_for_stylesheet,
};
#[cfg(test)]
pub(crate) use stylesheet::{
    author_source_text_parse_count_for_test, reset_author_source_text_parse_count_for_test,
};
mod retained_plan;
mod runtime_invalidation;
mod schedule;
mod scope;
mod shadow_scopes;
mod snapshot;
mod source;
mod source_cascade;
#[cfg(test)]
pub(crate) use source_cascade::{
    reset_source_cascade_rebuild_count_for_test, source_cascade_rebuild_count_for_test,
};
mod source_dirty;
mod source_document;
mod source_id;
mod source_key;
mod source_lifecycle;
mod source_owner;
mod source_owner_text;
mod source_record;
mod source_scope_plan;
mod state;
#[cfg(debug_assertions)]
pub(crate) use state::ComputedStyleObservationInputEpochs;
mod stylesheet;
mod stylesheet_resources;
mod target_plan;
mod target_queries;
mod target_result;
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
    ComputedTextWrapModeKind, ComputedWhiteSpaceCollapseKind, StyleObservationSnapshot,
    StyloAnonymousBoxKind, StyloComputedStyleSnapshot,
};
use document_world::{DocumentStyleWorld, DocumentStyleWorlds};
pub(in crate::style_engine) use drain::StyleInvalidationDrainBoundary;
pub(crate) use drain::StyleInvalidationTurnExitBoundary;
use drain::drain_style_invalidations;
#[cfg(test)]
use moli_selector::StyloSourceDependencySummary;
pub(crate) use mutation_effect::{
    StyleAttributeImpact, StyleMutationEffect, normalized_style_attribute_name,
};
pub(crate) use property_metadata::{
    computed_longhand_count, computed_longhand_first_vendor_index, computed_longhand_name_at,
    computed_property_is_queryable,
};
pub(crate) use registered_properties::{
    CssCustomPropertyRegistration, CssCustomPropertyRegistrationError,
    CssCustomPropertyRegistrationRecord,
};
#[cfg(test)]
use scope::style_source_scope_for_mutation_effects;
pub(crate) use source::adopted::AdoptedStyleSheetInstallation;
pub(crate) use source::imports::stylesheet_top_level_import_urls;
pub(crate) use source::inline::InlineStyleCspState;
pub(crate) use source::store::{
    OwnerStyleSheetSource, StylesheetFontFaceDescriptor, StyloStylesheetSource,
};
pub(crate) use source_id::StyleSourceId;
#[cfg(test)]
use source_id::StyleSourceKind;
use source_id::{StyleInvalidationSourceTarget, StyleScopeId};
pub(crate) use source_lifecycle::OwnedStyleSourceDocumentContext;
use source_lifecycle::StyleSourceDocumentContext;
pub(crate) use source_owner::{
    link_rel_qualifies_as_stylesheet, stylesheet_owner_type_is_supported,
};
pub(crate) use stylesheet_resources::{StylesheetResourceGeneration, StylesheetResourceSnapshot};
#[cfg(test)]
use stylesheet_resources::{
    reset_stylesheet_resource_manifest_build_count_for_test,
    stylesheet_resource_manifest_build_count_for_test,
};
#[cfg(test)]
use target_queries::PendingStyleInvalidationTargetQueries;
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
            dom_adapter: StyloDomStyleAdapter::new(),
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

    pub(crate) fn computed_style_observation_generations(
        &self,
        documents: impl IntoIterator<Item = DomHandle>,
    ) -> Vec<(DomHandle, u64, u64, u64)> {
        self.document_worlds.observation_generations(documents)
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
            .bump_target_context_epoch();
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
            .lazy_invalidation_roots
            .root_count()
    }

    #[cfg(test)]
    pub(crate) fn style_invalidation_generation_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.world_for_document(document)
            .document_state
            .lazy_invalidation_roots
            .generation()
    }

    #[cfg(test)]
    pub(crate) fn style_invalidation_path_node_visit_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> u64 {
        self.world_for_document(document)
            .document_state
            .lazy_invalidation_roots
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
    pub(crate) fn invalidation_clear_all_fallback_reasons_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> Vec<moli_selector::StyloSourceInvalidationFallbackReason> {
        self.world_for_document(document)
            .document_state
            .invalidation_clear_all_fallback_reasons_for_test()
    }

    #[cfg(test)]
    pub(crate) fn clear_retained_style_system_for_document_for_test(&self, document: DomHandle) {
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
        world.pending_invalidations.work_item_count_for_test()
            + world
                .pending_structural_mutations
                .work_item_count_for_test()
    }

    #[cfg(test)]
    pub(crate) fn pending_style_invalidation_work_kind_names_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> Vec<&'static str> {
        let world = self.world_for_document(document);
        let mut names = world.pending_invalidations.work_kind_names_for_test();
        names.extend(
            world
                .pending_structural_mutations
                .work_kind_names_for_test(),
        );
        names
    }

    #[cfg(test)]
    pub(crate) fn pending_structural_style_mutation_effect_count_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> usize {
        self.world_for_document(document)
            .pending_structural_mutations
            .effect_count_for_test()
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
        self.dom_adapter
            .clear_inline_style_attributes_for_document(document);
        self.dom_adapter
            .clear_shadow_cascade_data_for_document(document);
        self.document_worlds.retire_document(document)
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
    pub(crate) fn retire_computed_style_for_inactive_handle(&self, handle: DomHandle) {
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
                document_context,
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
                document_context,
                boundary.into(),
            );
        }
    }

    fn drain_pending_style_invalidations_for_document_and_boundary(
        &self,
        host: &DomHost,
        document: DomHandle,
        document_context: StyleSourceDocumentContext<'_>,
        boundary: StyleInvalidationDrainBoundary,
    ) {
        let world = self.world_for_document(document);
        self.flush_pending_structural_mutations_for_world(host, document, &world);
        let pending = world.pending_invalidations.take();
        let source_stores = world.borrow_source_stores();
        drain_style_invalidations(
            &self.dom_adapter,
            &world.document_state,
            self.invalidation_cleanup_for_world(&world),
            host,
            &source_stores,
            document_context,
            document,
            pending,
            boundary,
        );
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
            StyleSourceDocumentContext::for_root_document(document),
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

    #[cfg(test)]
    fn test_author_sources_have_relative_selector_dependency_for_document(
        &self,
        host: &DomHost,
        document: DomHandle,
        source_scope: &StyleSourceScope,
        emulated_media: &crate::protocol_types::EmulatedMediaOverrides,
    ) -> bool {
        self.test_author_sources_match_dependency_summary_for_document(
            host,
            document,
            source_scope,
            emulated_media,
            StyloSourceDependencySummary::has_relative_selector_dependency,
        )
    }

    #[cfg(test)]
    fn test_author_sources_match_dependency_summary_for_document(
        &self,
        host: &DomHost,
        document: DomHandle,
        source_scope: &StyleSourceScope,
        emulated_media: &crate::protocol_types::EmulatedMediaOverrides,
        summary_predicate: impl Fn(&StyloSourceDependencySummary) -> bool + Copy,
    ) -> bool {
        let world = self.world_for_document(document);
        let source_stores = world.borrow_source_stores();
        source_stores
            .matching_dependency_sources(
                host,
                source_scope,
                emulated_media,
                StyleViewport::default(),
            )
            .iter()
            .any(|source| source.dependency_summary_matches_for_test(summary_predicate))
    }

    #[cfg(test)]
    fn test_author_sources_have_attribute_dependency_for_document(
        &self,
        host: &DomHost,
        document: DomHandle,
        element: DomHandle,
        attribute_name: &str,
    ) -> bool {
        let world = self.world_for_document(document);
        let source_stores = world.borrow_source_stores();
        let mut queries = IndexSet::new();
        queries.insert(RetainedStyleInvalidationQuery::attribute(
            element,
            attribute_name.to_owned(),
        ));
        let request_plan = request::RetainedSourceDependencyRequestPlan::exact(queries);
        source_stores.has_dependency_match_for_request_plan(host, &request_plan)
    }

    #[cfg(test)]
    pub(crate) fn retained_stylesheet_source_ids_for_document_for_test(
        &self,
        host: &DomHost,
        document: DomHandle,
    ) -> Vec<StyleSourceId> {
        let world = self.world_for_document(document);
        let document_context = StyleSourceDocumentContext::for_root_document(document);
        let source_stores = world.borrow_source_stores();
        let source_lifecycle = source_stores.source_lifecycle_report(host, document_context);
        source_stores
            .retained_source_records_for_lifecycle(host, &source_lifecycle)
            .into_iter()
            .map(|record| record.id().clone())
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn matching_dependency_source_ids_for_document_for_test(
        &self,
        host: &DomHost,
        document: DomHandle,
        source_scope: &StyleSourceScope,
        emulated_media: &crate::protocol_types::EmulatedMediaOverrides,
    ) -> Vec<(StyleSourceId, IndexSet<DomHandle>)> {
        self.matching_dependency_source_targets_for_document_for_test(
            host,
            document,
            source_scope,
            emulated_media,
        )
        .into_iter()
        .filter_map(|(target, fallback_roots)| {
            Some((target.stylesheet_source_id()?.clone(), fallback_roots))
        })
        .collect()
    }

    #[cfg(test)]
    fn matching_dependency_source_targets_for_document_for_test(
        &self,
        host: &DomHost,
        document: DomHandle,
        source_scope: &StyleSourceScope,
        emulated_media: &crate::protocol_types::EmulatedMediaOverrides,
    ) -> Vec<(StyleInvalidationSourceTarget, IndexSet<DomHandle>)> {
        let world = self.world_for_document(document);
        let source_stores = world.borrow_source_stores();
        let sources = source_stores.matching_dependency_sources(
            host,
            source_scope,
            emulated_media,
            StyleViewport::default(),
        );
        sources
            .into_iter()
            .map(|source| {
                let (target, fallback_roots) = source.into_target_and_fallback_roots_for_test();
                (target, fallback_roots.into_iter().collect())
            })
            .collect()
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

    pub(crate) fn computed_anonymous_style_snapshot_from_current_observation(
        &self,
        host: &DomHost,
        document_url: &url::Url,
        owner: DomHandle,
        parent_style: &style::properties::ComputedValues,
        anonymous_kind: StyloAnonymousBoxKind,
        document_context: StyleSourceDocumentContext<'_>,
        read_document: DomHandle,
    ) -> Option<StyloComputedStyleSnapshot> {
        computed::computed_anonymous_style_snapshot_from_current_observation(
            self,
            host,
            document_url,
            owner,
            parent_style,
            anonymous_kind,
            document_context,
            read_document,
        )
    }
}

pub(crate) fn ensure_stylo_browser_compat_prefs() {
    static ENABLE: Once = Once::new();
    ENABLE.call_once(|| {
        stylo_static_prefs::set_pref!("layout.container-queries.enabled", true);
        stylo_static_prefs::set_pref!("layout.columns.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.at-scope.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.attr.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.content.alt-text.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.margin-rules.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.scroll-driven-animations.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.style-queries.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.tree-counting-functions.enabled", true);
        stylo_static_prefs::set_pref!("layout.css.zoom.enabled", true);
        stylo_static_prefs::set_pref!("layout.grid.enabled", true);
        // Stylo exposes the experimental CSS Sizing `fit-content(<length>)`
        // form behind a Servo pref. Chromium 147 rejects that function for
        // width/height while still accepting the bare `fit-content` keyword
        // and grid track `fit-content()`. Keep declaration parsing, CSSOM and
        // CSS.supports on the Chromium surface; the grid parser is separate.
        stylo_static_prefs::set_pref!("layout.css.fit-content-function.enabled", false);
        // Blitz d788124a enables Stylo's omnibus Servo gate before creating
        // its Stylist. CSS masking is implemented in this pinned Stylo world
        // but remains grouped behind that gate, so paint cannot receive the
        // computed mask longhands without matching the upstream setup.
        stylo_static_prefs::set_pref!("layout.unimplemented", true);
        stylo_static_prefs::set_pref!("layout.writing-mode.enabled", true);
    });
}

#[cfg(test)]
fn stylo_source_metadata_for_css_text(
    css_text: &str,
    base_url: &url::Url,
) -> source::store::StyleSourceMetadata {
    stylesheet::style_source_metadata_for_css_text(css_text, base_url)
}

fn retain_owner_documents_except_document(
    owner_documents: &mut HashMap<DomHandle, DomHandle>,
    document: DomHandle,
) {
    owner_documents.retain(|_, owner_document| *owner_document != document);
}
