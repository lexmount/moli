#[cfg(test)]
use std::sync::Arc as StdArc;
use std::sync::Once;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use dom::ElementState as StyloElementState;
#[cfg(test)]
use style::Atom;
use style::{
    context::QuirksMode, device::servo::ServoMediaFeaturePreferences, media_queries::MediaType,
    queries::values::PrefersColorScheme, servo::media_features::PrefersContrast,
    values::specified::color::ForcedColors,
};

use crate::{document_runtime::DomHandle, dom::native::DomHost};
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

mod cache;
mod cause;
mod cleanup;
mod computed;
mod document_world;
mod drain;
mod eligibility;
mod fallback;
mod invalidation;
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
pub(crate) use retained::{
    NativeStylesheetFontFaceProjection, NativeStylesheetFontFaceRuleProjection,
    StylesheetFontFaceProjection, StylesheetFontFaceRuleProjection,
    native_font_face_projection_for_stylesheet, native_font_face_rules_for_stylesheet,
};
#[cfg(test)]
pub(crate) use retained::{
    author_source_text_parse_count_for_test, reset_author_source_text_parse_count_for_test,
};
mod retained_plan;
mod runtime_invalidation;
mod schedule;
mod scope;
mod snapshot;
mod source;
mod source_dirty;
mod source_document;
mod source_id;
mod source_lifecycle;
mod source_owner;
mod source_owner_text;
mod source_record;
mod source_scope_plan;
mod state;
mod system;
mod target_plan;
mod target_queries;
mod target_result;
#[cfg(test)]
mod tests;
mod ua;

use cache::ComputedStyleInputCacheGeneration;
use cleanup::StyleCacheCleanup;
pub(crate) use computed::{
    ComputedDisplayKind, ComputedRenderedStyleFacts, ComputedTextTransformKind,
    ComputedTextWrapModeKind, ComputedWhiteSpaceCollapseKind, StyloAnonymousBoxKind,
    StyloComputedStyleSnapshot,
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
pub(crate) use source_owner::link_rel_qualifies_as_stylesheet;
use system::StyleSystemCacheKey;
#[cfg(test)]
use target_queries::PendingStyleInvalidationTargetQueries;

/// Page-level facade for document-owned style state.
///
/// The selector/query adapter in `moli-selector` deliberately stays
/// query-only. The facade owns cross-document lookup indexes plus the Stylo
/// side-table adapter; style sources, retained state, invalidation state, and
/// computed cache live in per-document worlds.
pub(crate) struct MoliStyleEngine {
    dom_adapter: StyloDomStyleAdapter,
    document_worlds: DocumentStyleWorlds,
    owner_stylesheet_source_documents: RefCell<HashMap<DomHandle, DomHandle>>,
    linked_stylesheet_owner_documents: RefCell<HashMap<DomHandle, DomHandle>>,
    inline_style_metadata_documents: RefCell<HashMap<DomHandle, DomHandle>>,
}

#[derive(Clone, Debug)]
pub(crate) struct StyloComputedStyleInputs {
    pub(crate) document_stylesheet_sources: Vec<StyloStylesheetSource>,
    pub(crate) shadow_stylesheet_sources: Vec<(DomHandle, Vec<StyloStylesheetSource>)>,
    pub(crate) script_custom_property_registrations: Vec<CssCustomPropertyRegistration>,
    pub(crate) script_custom_property_base_url: url::Url,
    pub(crate) environment: StyloStyleEnvironment,
    pub(crate) quirks_mode: QuirksMode,
}

/// Immutable Stylo inputs paired with the exact retained-system identity they
/// determine for one document/viewport observation context.
///
/// Keeping the key beside the inputs prevents descendant reads from hashing
/// the same stylesheet source set again. The pair is generation-scoped by the
/// owning document cache, so callers must not manufacture it across style
/// lifecycle boundaries.
#[derive(Debug)]
pub(crate) struct StyloPreparedComputedStyleInputs {
    inputs: Rc<StyloComputedStyleInputs>,
    style_system_key: StyleSystemCacheKey,
}

impl StyloPreparedComputedStyleInputs {
    pub(crate) fn new(
        document_url: &url::Url,
        inputs: Rc<StyloComputedStyleInputs>,
        viewport: StyleViewport,
    ) -> Self {
        let style_system_key = StyleSystemCacheKey::new(document_url, inputs.as_ref(), viewport);
        Self {
            inputs,
            style_system_key,
        }
    }

    pub(crate) fn inputs(&self) -> &StyloComputedStyleInputs {
        self.inputs.as_ref()
    }

    pub(in crate::style_engine) fn style_system_key(&self) -> &StyleSystemCacheKey {
        &self.style_system_key
    }
}

impl Default for StyloComputedStyleInputs {
    fn default() -> Self {
        Self {
            document_stylesheet_sources: Vec::new(),
            shadow_stylesheet_sources: Vec::new(),
            script_custom_property_registrations: Vec::new(),
            script_custom_property_base_url: url::Url::parse("about:blank")
                .expect("static about:blank URL parses"),
            environment: StyloStyleEnvironment::default(),
            quirks_mode: QuirksMode::NoQuirks,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct StyleViewport {
    pub(crate) width: Option<f64>,
    pub(crate) height: Option<f64>,
    pub(crate) screen_width: Option<f64>,
    pub(crate) screen_height: Option<f64>,
}

impl StyleViewport {
    pub(crate) const fn new(width: Option<f64>, height: Option<f64>) -> Self {
        Self {
            width,
            height,
            screen_width: None,
            screen_height: None,
        }
    }

    pub(crate) const fn from_width(width: Option<f64>) -> Self {
        Self {
            width,
            height: None,
            screen_width: None,
            screen_height: None,
        }
    }

    pub(crate) const fn with_screen_size(
        self,
        screen_width: Option<f64>,
        screen_height: Option<f64>,
    ) -> Self {
        Self {
            screen_width,
            screen_height,
            ..self
        }
    }

    pub(crate) fn from_viewport_surface(surface: crate::protocol_types::ViewportSurface) -> Self {
        Self::new(
            Some(f64::from(surface.inner_width)),
            Some(f64::from(surface.inner_height)),
        )
        .with_screen_size(
            Some(f64::from(surface.screen_width)),
            Some(f64::from(surface.screen_height)),
        )
    }
}

impl From<Option<f64>> for StyleViewport {
    fn from(width: Option<f64>) -> Self {
        Self::from_width(width)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct StyloStyleEnvironment {
    media_type: StyloStyleMediaType,
    color_scheme: StyloStyleColorScheme,
    reduced_motion: StyloStyleReducedPreference,
    reduced_data: StyloStyleReducedPreference,
    reduced_transparency: StyloStyleReducedPreference,
    contrast: StyloStyleContrastPreference,
    forced_colors: StyloStyleForcedColors,
}

/// Non-generation context for one document-owned prepared-input entry.
///
/// The source document is implicit in the owning `DocumentStyleWorld`; source
/// content and tree changes are represented by the cache generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StyloDocumentComputedStyleInputCacheKey {
    read_document: Option<DomHandle>,
    tree_scope_roots: Vec<DomHandle>,
    document_url: url::Url,
    viewport_width_bits: Option<u64>,
    viewport_height_bits: Option<u64>,
    screen_width_bits: Option<u64>,
    screen_height_bits: Option<u64>,
    environment: StyloStyleEnvironment,
    script_custom_property_base_url: url::Url,
}

impl StyloDocumentComputedStyleInputCacheKey {
    pub(crate) fn new(
        read_document: Option<DomHandle>,
        tree_scope_roots: &[DomHandle],
        document_url: &url::Url,
        viewport: StyleViewport,
        environment: StyloStyleEnvironment,
        script_custom_property_base_url: &url::Url,
    ) -> Self {
        let mut document_url = document_url.clone();
        document_url.set_fragment(None);
        Self {
            read_document,
            tree_scope_roots: tree_scope_roots.to_vec(),
            document_url,
            viewport_width_bits: viewport.width.map(f64::to_bits),
            viewport_height_bits: viewport.height.map(f64::to_bits),
            screen_width_bits: viewport.screen_width.map(f64::to_bits),
            screen_height_bits: viewport.screen_height.map(f64::to_bits),
            environment,
            script_custom_property_base_url: script_custom_property_base_url.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum StyloStyleMediaType {
    #[default]
    Screen,
    Print,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum StyloStyleColorScheme {
    #[default]
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum StyloStyleReducedPreference {
    #[default]
    NoPreference,
    Reduce,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum StyloStyleContrastPreference {
    More,
    Less,
    Custom,
    #[default]
    NoPreference,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum StyloStyleForcedColors {
    #[default]
    None,
    Active,
}

impl StyloStyleEnvironment {
    pub(crate) fn from_emulated_media(
        overrides: &crate::protocol_types::EmulatedMediaOverrides,
    ) -> Self {
        Self {
            media_type: if overrides.media.as_deref() == Some("print") {
                StyloStyleMediaType::Print
            } else {
                StyloStyleMediaType::Screen
            },
            color_scheme: if overrides.color_scheme.as_deref() == Some("dark") {
                StyloStyleColorScheme::Dark
            } else {
                StyloStyleColorScheme::Light
            },
            reduced_motion: match overrides.reduced_motion.as_deref() {
                Some("reduce") => StyloStyleReducedPreference::Reduce,
                Some("no-preference") | None => StyloStyleReducedPreference::NoPreference,
                Some(_) => StyloStyleReducedPreference::NoPreference,
            },
            reduced_data: StyloStyleReducedPreference::NoPreference,
            reduced_transparency: StyloStyleReducedPreference::NoPreference,
            contrast: match overrides.contrast.as_deref() {
                Some("more") => StyloStyleContrastPreference::More,
                Some("less") => StyloStyleContrastPreference::Less,
                Some("custom") => StyloStyleContrastPreference::Custom,
                Some("no-preference") | None => StyloStyleContrastPreference::NoPreference,
                Some(_) => StyloStyleContrastPreference::NoPreference,
            },
            forced_colors: match overrides.forced_colors.as_deref() {
                Some("active") => StyloStyleForcedColors::Active,
                Some("none") | None => StyloStyleForcedColors::None,
                Some(_) => StyloStyleForcedColors::None,
            },
        }
    }

    fn stylo_media_type(self) -> MediaType {
        match self.media_type {
            StyloStyleMediaType::Screen => MediaType::screen(),
            StyloStyleMediaType::Print => MediaType::print(),
        }
    }

    fn stylo_prefers_color_scheme(self) -> PrefersColorScheme {
        match self.color_scheme {
            StyloStyleColorScheme::Light => PrefersColorScheme::Light,
            StyloStyleColorScheme::Dark => PrefersColorScheme::Dark,
        }
    }

    fn stylo_media_feature_preferences(self) -> ServoMediaFeaturePreferences {
        ServoMediaFeaturePreferences {
            prefers_reduced_motion: self.reduced_motion.prefers_reduced(),
            prefers_reduced_data: self.reduced_data.prefers_reduced(),
            prefers_reduced_transparency: self.reduced_transparency.prefers_reduced(),
            prefers_contrast: match self.contrast {
                StyloStyleContrastPreference::More => PrefersContrast::More,
                StyloStyleContrastPreference::Less => PrefersContrast::Less,
                StyloStyleContrastPreference::Custom => PrefersContrast::Custom,
                StyloStyleContrastPreference::NoPreference => PrefersContrast::NoPreference,
            },
            forced_colors: match self.forced_colors {
                StyloStyleForcedColors::None => ForcedColors::None,
                StyloStyleForcedColors::Active => ForcedColors::Active,
            },
        }
    }
}

impl StyloStyleReducedPreference {
    fn prefers_reduced(self) -> bool {
        matches!(self, Self::Reduce)
    }
}

impl Default for MoliStyleEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MoliStyleEngine {
    pub(crate) fn new() -> Self {
        ensure_stylo_browser_compat_prefs();
        Self {
            dom_adapter: StyloDomStyleAdapter::new(),
            document_worlds: DocumentStyleWorlds::new(),
            owner_stylesheet_source_documents: RefCell::new(HashMap::new()),
            linked_stylesheet_owner_documents: RefCell::new(HashMap::new()),
            inline_style_metadata_documents: RefCell::new(HashMap::new()),
        }
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

    pub(crate) fn computed_cache_generation_for_document(&self, document: DomHandle) -> u64 {
        self.world_for_document(document)
            .document_state
            .computed_cache_generation()
    }

    pub(crate) fn computed_style_observation_generations(
        &self,
        documents: impl IntoIterator<Item = DomHandle>,
    ) -> Vec<(DomHandle, u64, u64, u64)> {
        self.document_worlds.observation_generations(documents)
    }

    #[cfg(debug_assertions)]
    pub(crate) fn computed_style_read_invariant_state(&self, document: DomHandle) -> (u64, u64) {
        let world = self.world_for_document(document);
        let generations = world.document_state.generation_snapshot();
        (
            generations.source_set_generation,
            generations.retained_style_system_generation,
        )
    }

    pub(crate) fn cached_document_prepared_style_inputs(
        &self,
        document: DomHandle,
        key: &StyloDocumentComputedStyleInputCacheKey,
    ) -> Option<Rc<StyloPreparedComputedStyleInputs>> {
        let world = self.world_for_document(document);
        let generations = world.document_state.generation_snapshot();
        world.computed_style_input_cache.get(
            ComputedStyleInputCacheGeneration {
                source_set: generations.source_set_generation,
                computed_values: generations.computed_cache_generation,
                target_context: generations.target_context_epoch,
            },
            key,
        )
    }

    pub(crate) fn cache_document_prepared_style_inputs(
        &self,
        document: DomHandle,
        key: StyloDocumentComputedStyleInputCacheKey,
        inputs: Rc<StyloPreparedComputedStyleInputs>,
    ) {
        let world = self.world_for_document(document);
        let generations = world.document_state.generation_snapshot();
        world.computed_style_input_cache.insert(
            ComputedStyleInputCacheGeneration {
                source_set: generations.source_set_generation,
                computed_values: generations.computed_cache_generation,
                target_context: generations.target_context_epoch,
            },
            key,
            inputs,
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

    pub(crate) fn target_context_epoch_for_document(&self, document: DomHandle) -> u64 {
        self.world_for_document(document)
            .document_state
            .target_context_epoch()
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
        self.world_for_document(document).computed_style_cache.len()
    }

    #[cfg(test)]
    pub(crate) fn computed_style_cache_contains_handle_for_document_for_test(
        &self,
        document: DomHandle,
        handle: DomHandle,
    ) -> bool {
        self.world_for_document(document)
            .computed_style_cache
            .contains_handle_for_test(handle)
    }

    #[cfg(test)]
    pub(crate) fn computed_style_cache_entry_count_for_handle_for_document_for_test(
        &self,
        document: DomHandle,
        handle: DomHandle,
    ) -> usize {
        self.world_for_document(document)
            .computed_style_cache
            .entry_count_for_handle_for_test(handle)
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
    pub(in crate::style_engine) fn retained_style_system_matches_for_document_for_test(
        &self,
        document: DomHandle,
        key: &StyleSystemCacheKey,
    ) -> bool {
        self.world_for_document(document)
            .document_state
            .retained_style_system_matches(key)
    }

    #[cfg(test)]
    pub(crate) fn retained_style_system_is_none_for_document_for_test(
        &self,
        document: DomHandle,
    ) -> bool {
        self.world_for_document(document)
            .document_state
            .try_with_retained_style_system(|_| ())
            .is_none()
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

    pub(in crate::style_engine) fn cache_cleanup_for_world<'a>(
        &'a self,
        world: &'a DocumentStyleWorld,
    ) -> StyleCacheCleanup<'a> {
        StyleCacheCleanup::new(
            world.document,
            &self.dom_adapter,
            &world.computed_style_cache,
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
        key: StyleSystemCacheKey,
        inputs: &StyloComputedStyleInputs,
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
            self.cache_cleanup_for_world(&world),
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
        inputs: &StyloComputedStyleInputs,
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
        inputs: &StyloComputedStyleInputs,
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
        inputs: &StyloComputedStyleInputs,
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

    pub(crate) fn computed_style_snapshot_after_style_update_with_prepared_inputs(
        &self,
        host: &DomHost,
        document_url: &url::Url,
        handle: DomHandle,
        inputs: &StyloPreparedComputedStyleInputs,
        document_context: StyleSourceDocumentContext<'_>,
        read_document: DomHandle,
    ) -> Option<StyloComputedStyleSnapshot> {
        computed::computed_style_snapshot_after_style_update_with_prepared_inputs(
            self,
            host,
            document_url,
            handle,
            inputs,
            document_context,
            read_document,
        )
    }

    pub(crate) fn computed_pseudo_style_snapshot_after_style_update_with_prepared_inputs(
        &self,
        host: &DomHost,
        document_url: &url::Url,
        handle: DomHandle,
        pseudo_element: &str,
        inputs: &StyloPreparedComputedStyleInputs,
        document_context: StyleSourceDocumentContext<'_>,
        read_document: DomHandle,
    ) -> Option<StyloComputedStyleSnapshot> {
        computed::computed_pseudo_style_snapshot_after_style_update_with_prepared_inputs(
            self,
            host,
            document_url,
            handle,
            pseudo_element,
            inputs,
            document_context,
            read_document,
        )
    }

    pub(crate) fn computed_anonymous_style_snapshot_after_style_update_with_prepared_inputs(
        &self,
        host: &DomHost,
        document_url: &url::Url,
        owner: DomHandle,
        parent_style: &style::properties::ComputedValues,
        anonymous_kind: StyloAnonymousBoxKind,
        inputs: &StyloPreparedComputedStyleInputs,
        document_context: StyleSourceDocumentContext<'_>,
        read_document: DomHandle,
    ) -> Option<StyloComputedStyleSnapshot> {
        computed::computed_anonymous_style_snapshot_after_style_update_with_prepared_inputs(
            self,
            host,
            document_url,
            owner,
            parent_style,
            anonymous_kind,
            inputs,
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
    retained::style_source_metadata_for_css_text(css_text, base_url)
}

fn retain_owner_documents_except_document(
    owner_documents: &mut HashMap<DomHandle, DomHandle>,
    document: DomHandle,
) {
    owner_documents.retain(|_, owner_document| *owner_document != document);
}
