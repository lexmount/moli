use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use style::{
    author_styles::AuthorStyles,
    invalidation::stylesheets::StylesheetInvalidationSet,
    servo_arc::Arc as ServoArc,
    shared_lock::{SharedRwLock, StylesheetGuards},
    stylesheets::{CustomMediaMap, DocumentStyleSheet, Origin, OriginSet, UrlExtraData},
    stylist::Stylist,
};

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::{
    CssCustomPropertyRegistrationRecord, FullStyleWorldSnapshot,
    active_stylesheets::{notify_document_stylesheet_rule_changes, update_document_stylesheet_set},
    shadow_scopes::{ShadowScopeStyles, reconcile_dirty_shadow_scopes, reconcile_shadow_scopes},
    source::store::StyloStylesheetSource,
    source_cascade::{build_source_cascade_data, update_source_cascade_data_for_scopes},
    source_id::{StyleScopeId, StyleSourceId},
    source_record::RetainedStylesheetSourceRecord,
    state::RetainedStyleSystem,
    stylesheet::{
        append_stylesheet_to_stylist, install_active_stylesheet, install_active_stylesheets,
        moli_ua_stylesheet_base_url, new_style_device_with_viewport_bits,
        new_stylist_with_viewport_bits,
    },
    stylesheet_resources::StylesheetResourceManifest,
    ua::HTML_STYLESHEET as MOLI_UA_STYLESHEET,
    world_key::StyleWorldKey,
    world_update::IncrementalStyleWorldUpdate,
};

static NEXT_STYLIST_IDENTITY: AtomicU64 = AtomicU64::new(1);

pub(super) struct RetainedStyleInvalidations {
    pub(super) document: Option<StylesheetInvalidationSet>,
    pub(super) document_scope_fallback: bool,
    pub(super) shadow_scopes: Vec<(DomHandle, StylesheetInvalidationSet)>,
    pub(super) shadow_scope_fallbacks: Vec<DomHandle>,
    pub(super) removed_shadow_scopes: Vec<DomHandle>,
    pub(super) viewport_size_changed: bool,
    pub(super) used_color_scheme_changed: bool,
}

/// Compares an explicit full-world snapshot with the canonical retained
/// collections. This is used only by the low-level full-snapshot API and the
/// fresh-world test oracle; normal observations are driven by dirty state and
/// never materialize these vectors merely to check freshness.
pub(super) fn retained_style_system_matches_full_snapshot(
    retained: &RetainedStyleSystem,
    key: &StyleWorldKey,
    inputs: &FullStyleWorldSnapshot,
) -> bool {
    retained.key == *key
        && retained
            .document_stylesheets
            .matches_sources(&inputs.document_stylesheet_sources)
        && retained.script_custom_property_registrations
            == inputs.script_custom_property_registrations
        && retained.shadow_scopes.len() == inputs.shadow_stylesheet_sources.len()
        && retained
            .shadow_scopes
            .iter()
            .zip(&inputs.shadow_stylesheet_sources)
            .all(|(scope, (root, sources))| {
                scope.root() == *root && scope.active_stylesheets().matches_sources(sources)
            })
}

impl RetainedStyleInvalidations {
    fn new() -> Self {
        Self {
            document: None,
            document_scope_fallback: false,
            shadow_scopes: Vec::new(),
            shadow_scope_fallbacks: Vec::new(),
            removed_shadow_scopes: Vec::new(),
            viewport_size_changed: false,
            used_color_scheme_changed: false,
        }
    }
}

pub(super) fn build_retained_style_system(
    host: &DomHost,
    key: StyleWorldKey,
    inputs: &FullStyleWorldSnapshot,
    shared_lock: &SharedRwLock,
    retained_source_records: &[RetainedStylesheetSourceRecord<'_>],
) -> RetainedStyleSystem {
    let mut stylist = new_stylist_with_viewport_bits(
        key.viewport_width_bits,
        key.viewport_height_bits,
        key.screen_width_bits,
        key.screen_height_bits,
        key.environment,
        key.quirks_mode,
    );
    register_script_custom_properties(&mut stylist, inputs);
    append_stylesheet_to_stylist(
        &mut stylist,
        shared_lock,
        MOLI_UA_STYLESHEET,
        moli_ua_stylesheet_base_url(),
        Origin::UserAgent,
        key.quirks_mode,
    );
    let document_stylesheets = install_active_stylesheets(
        host,
        shared_lock,
        &inputs.document_stylesheet_sources,
        key.quirks_mode,
    );
    {
        let guard = shared_lock.read();
        for stylesheet in document_stylesheets.cascade_stylesheets() {
            stylist.append_stylesheet(stylesheet, &guard);
        }
    }
    let mut shadow_scopes = Vec::new();
    let mut shadow_cascade_data = Vec::new();
    for (root, sources) in &inputs.shadow_stylesheet_sources {
        let active_stylesheets =
            install_active_stylesheets(host, shared_lock, sources, key.quirks_mode);
        let mut author_styles = AuthorStyles::<DocumentStyleSheet>::new();
        let custom_media = CustomMediaMap::default();
        let guard = shared_lock.read();
        for stylesheet in active_stylesheets.cascade_stylesheets() {
            author_styles
                .stylesheets
                .append_stylesheet(None, &custom_media, stylesheet, &guard);
        }
        author_styles.flush(&mut stylist, &guard);
        shadow_cascade_data.push((*root, author_styles.data.clone()));
        shadow_scopes.push(ShadowScopeStyles::new(
            *root,
            active_stylesheets,
            author_styles,
        ));
    }
    let (source_cascade_data, source_cascade_keys) = build_source_cascade_data(
        &mut stylist,
        shared_lock,
        &document_stylesheets,
        &shadow_scopes,
        retained_source_records,
        None,
        |source| install_active_stylesheet(host, shared_lock, source, key.quirks_mode),
    );
    let guard = shared_lock.read();
    stylist.flush(&StylesheetGuards::same(&guard));
    let user_agent_cascade_data = ServoArc::new(
        stylist
            .cascade_data()
            .borrow_for_origin(Origin::UserAgent)
            .clone(),
    );
    let stylesheet_resources = StylesheetResourceManifest::from_active_stylesheets(
        &stylist,
        &document_stylesheets,
        &shadow_scopes,
    );

    RetainedStyleSystem {
        stylist_identity: NEXT_STYLIST_IDENTITY.fetch_add(1, Ordering::Relaxed),
        key,
        stylist,
        document_stylesheets,
        shadow_scopes,
        stylesheet_resources,
        // `StyleDocumentState::set_retained_style_system` assigns the
        // document-monotonic revision after comparing any replaced world.
        stylesheet_resource_revision: 0,
        user_agent_cascade_data,
        shadow_cascade_data,
        source_cascade_data,
        source_cascade_keys,
        script_custom_property_registrations: inputs.script_custom_property_registrations.clone(),
    }
}

/// Updates the persistent document style world without replacing its Stylist.
///
/// This full-snapshot reconciliation is kept for the explicit style-engine API
/// and the fresh-world test oracle. Normal CSSOM/layout observations use
/// `update_retained_style_system_incrementally` and materialize only the dirty
/// collections named by their update plan.
pub(super) fn update_retained_style_system(
    retained: &mut RetainedStyleSystem,
    host: &DomHost,
    key: StyleWorldKey,
    inputs: &FullStyleWorldSnapshot,
    shared_lock: &SharedRwLock,
    retained_source_records: &[RetainedStylesheetSourceRecord<'_>],
) -> RetainedStyleInvalidations {
    let mut invalidations = RetainedStyleInvalidations::new();
    let document_update = update_document_scope(
        retained,
        host,
        &key,
        shared_lock,
        Some(&inputs.document_stylesheet_sources),
        Some(&inputs.script_custom_property_registrations),
    );
    invalidations.document = document_update.invalidations;
    invalidations.document_scope_fallback = document_update.scope_fallback;
    invalidations.viewport_size_changed = document_update.viewport_size_changed;
    invalidations.used_color_scheme_changed = document_update.used_color_scheme_changed;

    let shadow_reconciliation = reconcile_shadow_scopes(
        retained,
        shared_lock,
        &inputs.shadow_stylesheet_sources,
        document_update.device_changed,
        |source| install_active_stylesheet(host, shared_lock, source, key.quirks_mode),
    );
    invalidations.shadow_scopes = shadow_reconciliation.invalidations;
    invalidations.shadow_scope_fallbacks = shadow_reconciliation.scope_fallbacks;
    invalidations.removed_shadow_scopes = shadow_reconciliation.removed_roots;

    let previous_source_cascade_data = std::mem::take(&mut retained.source_cascade_data);
    let previous_source_cascade_keys = std::mem::take(&mut retained.source_cascade_keys);
    let (source_cascade_data, source_cascade_keys) = build_source_cascade_data(
        &mut retained.stylist,
        shared_lock,
        &retained.document_stylesheets,
        &retained.shadow_scopes,
        retained_source_records,
        Some((&previous_source_cascade_data, &previous_source_cascade_keys)),
        |source| install_active_stylesheet(host, shared_lock, source, key.quirks_mode),
    );
    retained.source_cascade_data = source_cascade_data;
    retained.source_cascade_keys = source_cascade_keys;
    refresh_retained_derived_state(
        retained,
        document_update.device_changed,
        document_update.stylesheets_changed || shadow_reconciliation.collections_changed,
    );
    retained.key = key;
    invalidations
}

/// Applies a dirty-plan update without reconstructing clean TreeScope input
/// vectors. Parsed stylesheets and AuthorStyles for untouched scopes stay in
/// place.
pub(super) fn update_retained_style_system_incrementally(
    retained: &mut RetainedStyleSystem,
    host: &DomHost,
    document: DomHandle,
    key: StyleWorldKey,
    update: &IncrementalStyleWorldUpdate,
    shared_lock: &SharedRwLock,
    retained_source_records: &[RetainedStylesheetSourceRecord<'_>],
    dirty_source_ids: &HashSet<StyleSourceId>,
    full_source_projection_scopes: &HashSet<StyleScopeId>,
) -> RetainedStyleInvalidations {
    let mut invalidations = RetainedStyleInvalidations::new();
    let document_update = update_document_scope(
        retained,
        host,
        &key,
        shared_lock,
        update.document_stylesheet_sources.as_deref(),
        update.script_custom_property_registrations.as_deref(),
    );
    invalidations.document = document_update.invalidations;
    invalidations.document_scope_fallback = document_update.scope_fallback;
    invalidations.viewport_size_changed = document_update.viewport_size_changed;
    invalidations.used_color_scheme_changed = document_update.used_color_scheme_changed;

    let shadow_reconciliation = reconcile_dirty_shadow_scopes(
        retained,
        shared_lock,
        &update.shadow_stylesheet_sources,
        update.connected_shadow_roots.as_deref(),
        document_update.device_changed,
        |source| install_active_stylesheet(host, shared_lock, source, key.quirks_mode),
    );
    invalidations.shadow_scopes = shadow_reconciliation.invalidations;
    invalidations.shadow_scope_fallbacks = shadow_reconciliation.scope_fallbacks;
    invalidations.removed_shadow_scopes = shadow_reconciliation.removed_roots;

    let mut dirty_source_scopes = full_source_projection_scopes.clone();
    if document_update
        .device_affected_origins
        .contains(OriginSet::ORIGIN_AUTHOR)
    {
        dirty_source_scopes.insert(StyleScopeId::Document(document));
    }
    dirty_source_scopes.extend(
        shadow_reconciliation
            .device_affected_roots
            .iter()
            .copied()
            .map(StyleScopeId::ShadowRoot),
    );
    dirty_source_scopes.extend(
        invalidations
            .removed_shadow_scopes
            .iter()
            .copied()
            .map(StyleScopeId::ShadowRoot),
    );
    update_source_cascade_data_for_scopes(
        retained,
        shared_lock,
        retained_source_records,
        dirty_source_ids,
        &dirty_source_scopes,
        document_update.device_changed,
        |source| install_active_stylesheet(host, shared_lock, source, key.quirks_mode),
    );
    refresh_retained_derived_state(
        retained,
        document_update.device_changed,
        document_update.stylesheets_changed || shadow_reconciliation.collections_changed,
    );
    retained.key = key;
    invalidations
}

struct DocumentScopeUpdate {
    invalidations: Option<StylesheetInvalidationSet>,
    scope_fallback: bool,
    device_changed: bool,
    viewport_size_changed: bool,
    used_color_scheme_changed: bool,
    device_affected_origins: OriginSet,
    stylesheets_changed: bool,
}

fn update_document_scope(
    retained: &mut RetainedStyleSystem,
    host: &DomHost,
    key: &StyleWorldKey,
    shared_lock: &SharedRwLock,
    stylesheet_sources: Option<&[StyloStylesheetSource]>,
    custom_property_registrations: Option<&[CssCustomPropertyRegistrationRecord]>,
) -> DocumentScopeUpdate {
    let custom_properties_changed = custom_property_registrations.is_some_and(|registrations| {
        retained.script_custom_property_registrations.as_slice() != registrations
    });
    if let Some(registrations) = custom_property_registrations.filter(|_| custom_properties_changed)
    {
        debug_assert!(
            retained.script_custom_property_registrations.len() <= registrations.len()
                && retained.script_custom_property_registrations
                    == registrations[..retained.script_custom_property_registrations.len()],
            "registered custom properties must be append-only within a retained style world"
        );
        for record in registrations
            .iter()
            .skip(retained.script_custom_property_registrations.len())
        {
            let registration = &record.registration;
            let url_data = UrlExtraData::from(record.base_url.clone());
            let _ = retained.stylist.register_custom_property(
                &url_data,
                &registration.name,
                &registration.syntax,
                registration.inherits,
                registration.initial_value.as_deref(),
            );
        }
        retained.script_custom_property_registrations = registrations.to_vec();
        retained
            .stylist
            .force_stylesheet_origins_dirty(OriginSet::ORIGIN_AUTHOR);
    }

    let viewport_size_changed = retained.key.viewport_width_bits != key.viewport_width_bits
        || retained.key.viewport_height_bits != key.viewport_height_bits;
    let used_color_scheme_changed = retained.key.environment.stylo_prefers_color_scheme()
        != key.environment.stylo_prefers_color_scheme()
        || retained.key.environment.stylo_page_color_schemes()
            != key.environment.stylo_page_color_schemes();
    let device_changed = viewport_size_changed
        || retained.key.screen_width_bits != key.screen_width_bits
        || retained.key.screen_height_bits != key.screen_height_bits
        || retained.key.environment != key.environment;
    let mut device_affected_origins = OriginSet::empty();
    if device_changed {
        let device = new_style_device_with_viewport_bits(
            key.viewport_width_bits,
            key.viewport_height_bits,
            key.screen_width_bits,
            key.screen_height_bits,
            key.environment,
            key.quirks_mode,
        );
        let guard = shared_lock.read();
        let guards = StylesheetGuards::same(&guard);
        device_affected_origins = retained.stylist.set_device(device, &guards);
        retained
            .stylist
            .force_stylesheet_origins_dirty(device_affected_origins);
    }

    let stylesheet_reconciliation = stylesheet_sources.and_then(|sources| {
        retained.document_stylesheets.reconcile(sources, |source| {
            install_active_stylesheet(host, shared_lock, source, key.quirks_mode)
        })
    });
    let mut scope_fallback = false;
    if let Some(reconciliation) = stylesheet_reconciliation.as_ref() {
        let guard = shared_lock.read();
        scope_fallback |= reconciliation.stylesheet_removed();
        if reconciliation.stylesheet_set_changed() {
            let next_stylesheets = retained.document_stylesheets.cascade_stylesheets();
            update_document_stylesheet_set(
                &mut retained.stylist,
                reconciliation.previous_stylesheets(),
                &next_stylesheets,
                &guard,
            );
        }
        if notify_document_stylesheet_rule_changes(&mut retained.stylist, reconciliation, &guard) {
            // Media/disabled/import-descendant changes and whole-sheet
            // replacement are explicit journal fallbacks.
            scope_fallback = true;
            retained
                .stylist
                .force_stylesheet_origins_dirty(OriginSet::ORIGIN_AUTHOR);
        }
    }

    let stylesheets_changed = stylesheet_reconciliation.is_some();
    let must_flush =
        !device_affected_origins.is_empty() || stylesheets_changed || custom_properties_changed;
    let invalidations = must_flush.then(|| {
        let guard = shared_lock.read();
        retained.stylist.flush(&StylesheetGuards::same(&guard))
    });
    DocumentScopeUpdate {
        invalidations,
        scope_fallback,
        device_changed,
        viewport_size_changed,
        used_color_scheme_changed,
        device_affected_origins,
        stylesheets_changed,
    }
}

fn refresh_retained_derived_state(
    retained: &mut RetainedStyleSystem,
    device_changed: bool,
    stylesheet_collections_changed: bool,
) {
    if device_changed {
        retained.user_agent_cascade_data = ServoArc::new(
            retained
                .stylist
                .cascade_data()
                .borrow_for_origin(Origin::UserAgent)
                .clone(),
        );
    }
    if !device_changed && !stylesheet_collections_changed {
        return;
    }
    let stylesheet_resources = StylesheetResourceManifest::from_active_stylesheets(
        &retained.stylist,
        &retained.document_stylesheets,
        &retained.shadow_scopes,
    );
    if retained.stylesheet_resources != stylesheet_resources {
        retained.stylesheet_resources = stylesheet_resources;
        retained.stylesheet_resource_revision =
            retained.stylesheet_resource_revision.saturating_add(1);
    }
}

fn register_script_custom_properties(stylist: &mut Stylist, inputs: &FullStyleWorldSnapshot) {
    for record in &inputs.script_custom_property_registrations {
        let registration = &record.registration;
        let url_data = UrlExtraData::from(record.base_url.clone());
        let _ = stylist.register_custom_property(
            &url_data,
            &registration.name,
            &registration.syntax,
            registration.inherits,
            registration.initial_value.as_deref(),
        );
    }
}
