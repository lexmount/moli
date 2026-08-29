//! Planning and application of on-demand updates to retained Document worlds.

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use indexmap::IndexSet;
use moli_selector::StyloDomStyleAdapter;
use style::context::QuirksMode;

use super::{
    FullStyleWorldSnapshot, StyleTreeScopeVersions, StyleWorldEnvironment,
    cleanup::StyleInvalidationCleanup,
    retained::{
        RetainedStyleInvalidations, build_retained_style_system,
        retained_style_system_matches_full_snapshot, update_retained_style_system,
        update_retained_style_system_incrementally,
    },
    source_document::DocumentStyleSourceStores,
    source_lifecycle::StyleSourceDocumentContext,
    state::StyleDocumentState,
    world_key::StyleWorldKey,
    world_trace::{RetainedStyleSystemChangeKind, trace_retained_style_system_change},
    world_update::{
        IncrementalStyleWorldUpdate, IncrementalStyleWorldUpdatePlan, StyleWorldUpdatePlan,
    },
};

pub(super) fn retained_style_world_update_plan(
    host: &DomHost,
    document_state: &StyleDocumentState,
    document: DomHandle,
    quirks_mode: QuirksMode,
    tree_scope_versions: StyleTreeScopeVersions,
) -> StyleWorldUpdatePlan {
    let source_dirty_scope = document_state.source_dirty_scope_snapshot();
    let Some((replace_world, tree_scopes_changed, retained_shadow_roots)) = document_state
        .try_with_retained_style_system(|retained| {
            (
                retained
                    .key
                    .requires_replacement_for_observation(quirks_mode),
                retained.key.tree_scope_versions != Some(tree_scope_versions),
                retained
                    .shadow_scopes
                    .iter()
                    .map(|scope| scope.root())
                    .collect::<Vec<_>>(),
            )
        })
    else {
        return StyleWorldUpdatePlan::Full;
    };
    if replace_world {
        return StyleWorldUpdatePlan::Full;
    }

    let connected_shadow_roots = tree_scopes_changed.then(|| {
        let mut roots = host
            .snapshot_connected_shadow_roots()
            .into_iter()
            .filter(|root| host.owner_document_handle(*root) == Some(document))
            .collect::<Vec<_>>();
        roots.sort_by_key(|root| root.index());
        roots
    });
    let mut shadow_stylesheet_roots = source_dirty_scope.dirty_shadow_roots();
    if let Some(connected_roots) = connected_shadow_roots.as_ref() {
        shadow_stylesheet_roots.extend(
            connected_roots
                .iter()
                .copied()
                .filter(|root| !retained_shadow_roots.contains(root)),
        );
    }

    StyleWorldUpdatePlan::Incremental(IncrementalStyleWorldUpdatePlan::new(
        source_dirty_scope.refreshes_document_stylesheets(document),
        shadow_stylesheet_roots.into_iter().collect(),
        connected_shadow_roots,
        source_dirty_scope.refreshes_custom_property_registrations(document),
    ))
}

pub(super) fn ensure_retained_style_system(
    host: &DomHost,
    dom_adapter: &StyloDomStyleAdapter,
    document_state: &StyleDocumentState,
    source_stores: &DocumentStyleSourceStores<'_>,
    document_context: StyleSourceDocumentContext<'_>,
    retained_document: DomHandle,
    invalidation_cleanup: StyleInvalidationCleanup<'_>,
    key: &StyleWorldKey,
    inputs: &FullStyleWorldSnapshot,
) {
    let source_dirty_scope = document_state.source_dirty_scope_snapshot();
    if document_state
        .try_with_retained_style_system(|retained| {
            retained_style_system_matches_full_snapshot(retained, key, inputs)
        })
        .unwrap_or(false)
    {
        if source_dirty_scope.requires_retained_style_update() {
            document_state.clear_source_dirty_scopes();
        }
        return;
    }
    let trace_enabled = moli_trace::style_invalidation_trace_enabled();
    let key_mismatch_trace =
        document_state.try_with_retained_style_system(|retained| retained.key.mismatch_trace(key));
    let trace_documents = trace_enabled.then(|| vec![retained_document]);

    let source_lifecycle = source_stores.source_lifecycle_report(host, document_context);
    let retained_source_records =
        source_stores.retained_source_records_for_lifecycle(host, &source_lifecycle);
    let shared_lock = dom_adapter.shared_lock().clone();
    let custom_property_history_is_append_only = document_state
        .try_with_retained_style_system(|retained| {
            retained.script_custom_property_registrations.len()
                <= inputs.script_custom_property_registrations.len()
                && retained.script_custom_property_registrations
                    == inputs.script_custom_property_registrations
                        [..retained.script_custom_property_registrations.len()]
        })
        .unwrap_or(true);
    let can_update_in_place = key_mismatch_trace
        .as_ref()
        .is_some_and(|trace| !trace.requires_style_system_replacement())
        && custom_property_history_is_append_only;
    if can_update_in_place {
        let invalidations = document_state.update_retained_style_system_with_result(|retained| {
            update_retained_style_system(
                retained,
                host,
                key.clone(),
                inputs,
                &shared_lock,
                &retained_source_records,
            )
        });
        apply_retained_stylesheet_invalidations(
            host,
            dom_adapter,
            retained_document,
            &invalidation_cleanup,
            invalidations,
        );
        document_state.clear_source_dirty_scopes();
        if trace_enabled {
            document_state.with_retained_style_system(|retained| {
                trace_retained_style_system_change(
                    inputs,
                    &source_lifecycle,
                    &source_dirty_scope,
                    document_state.source_set_generation(),
                    retained,
                    RetainedStyleSystemChangeKind::IncrementalUpdate,
                    custom_property_history_is_append_only,
                    key_mismatch_trace.as_ref(),
                    trace_documents.as_deref().unwrap_or_default(),
                );
            });
        }
        return;
    }

    // Replacing the document style world is the exceptional path. Incremental
    // updates above must preserve canonical ElementData so Stylo can apply the
    // invalidation set returned by flush().
    invalidation_cleanup.clear_for_retained_style_system_rebuild();
    document_state.clear_source_dirty_scopes();

    let retained = build_retained_style_system(
        host,
        key.clone(),
        inputs,
        &shared_lock,
        &retained_source_records,
    );
    if trace_enabled {
        trace_retained_style_system_change(
            inputs,
            &source_lifecycle,
            &source_dirty_scope,
            document_state.source_set_generation(),
            &retained,
            if key_mismatch_trace.is_some() {
                RetainedStyleSystemChangeKind::Replacement
            } else {
                RetainedStyleSystemChangeKind::InitialBuild
            },
            custom_property_history_is_append_only,
            key_mismatch_trace.as_ref(),
            trace_documents.as_deref().unwrap_or_default(),
        );
    }
    document_state.set_retained_style_system(retained);
}

pub(super) fn ensure_retained_style_system_incrementally(
    host: &DomHost,
    dom_adapter: &StyloDomStyleAdapter,
    document_state: &StyleDocumentState,
    source_stores: &DocumentStyleSourceStores<'_>,
    document_context: StyleSourceDocumentContext<'_>,
    retained_document: DomHandle,
    invalidation_cleanup: StyleInvalidationCleanup<'_>,
    environment: &StyleWorldEnvironment,
    update: &IncrementalStyleWorldUpdate,
) {
    let source_dirty_scope = document_state.source_dirty_scope_snapshot();
    let Some((next_key, device_changed)) =
        document_state.try_with_retained_style_system(|retained| {
            (
                retained.key.updated_for_observation(environment),
                retained
                    .key
                    .device_differs_from_observation(environment.viewport, environment.media),
            )
        })
    else {
        // Planning guarantees an initial observation is full. Keep this guard
        // explicit so a stale or incorrectly reused plan cannot create a
        // partial Document world.
        debug_assert!(false, "incremental style update requires a retained world");
        return;
    };

    let dirty_source_ids = source_dirty_scope
        .source_ids_vec()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let full_source_projection_scopes = source_dirty_scope
        .full_source_projection_scope_ids()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let source_lifecycle = if device_changed || !full_source_projection_scopes.is_empty() {
        source_stores.source_lifecycle_report(host, document_context)
    } else {
        source_stores.source_lifecycle_report_for_source_ids(
            host,
            document_context,
            dirty_source_ids.iter().cloned(),
        )
    };
    let retained_source_records =
        source_stores.retained_source_records_for_lifecycle(host, &source_lifecycle);
    let shared_lock = dom_adapter.shared_lock().clone();
    let invalidations = document_state.update_retained_style_system_with_result(|retained| {
        update_retained_style_system_incrementally(
            retained,
            host,
            retained_document,
            next_key,
            update,
            &shared_lock,
            &retained_source_records,
            &dirty_source_ids,
            &full_source_projection_scopes,
        )
    });
    apply_retained_stylesheet_invalidations(
        host,
        dom_adapter,
        retained_document,
        &invalidation_cleanup,
        invalidations,
    );
    document_state.clear_source_dirty_scopes();

    if moli_trace::style_invalidation_trace_enabled() {
        tracing::info!(
            retained_document = ?retained_document,
            document_scope_materialized = update.document_stylesheet_sources.is_some(),
            shadow_scopes_materialized = ?update
                .shadow_stylesheet_sources
                .iter()
                .map(|(root, _)| *root)
                .collect::<Vec<_>>(),
            tree_scope_membership_materialized = update.connected_shadow_roots.is_some(),
            custom_properties_materialized = update
                .script_custom_property_registrations
                .is_some(),
            source_dirty_ids = ?source_dirty_scope.source_ids_vec(),
            source_dirty_scope_ids = ?source_dirty_scope.scope_ids_vec(),
            source_dirty_reasons = ?source_dirty_scope.reasons_vec(),
            "incremental retained style world update"
        );
    }
}

fn apply_retained_stylesheet_invalidations(
    host: &DomHost,
    dom_adapter: &StyloDomStyleAdapter,
    document: DomHandle,
    invalidation_cleanup: &StyleInvalidationCleanup<'_>,
    invalidations: RetainedStyleInvalidations,
) {
    let RetainedStyleInvalidations {
        document: document_invalidations,
        document_scope_fallback,
        shadow_scopes,
        shadow_scope_fallbacks,
        removed_shadow_scopes,
        viewport_size_changed,
        used_color_scheme_changed,
    } = invalidations;
    let mut stylesheet_invalidation_roots = IndexSet::new();
    if let Some(invalidations) = document_invalidations {
        // HTML documents normally have one document element. The native DOM
        // also permits fragment-like test documents with several top-level
        // elements, and every one of those roots belongs to the document
        // stylesheet scope.
        let mut roots = host
            .child_handles(document)
            .filter(|handle| {
                host.node(*handle)
                    .is_some_and(|node| node.as_element().is_some())
            })
            .collect::<Vec<_>>();
        if roots.is_empty()
            && let Some(root) = host.document_element_handle_for_document(document)
        {
            roots.push(root);
        }
        stylesheet_invalidation_roots.extend(dom_adapter.process_stylesheet_invalidations(
            host,
            &roots,
            &invalidations,
        ));
    }
    for (shadow_root, invalidations) in shadow_scopes {
        let mut roots = host
            .child_handles(shadow_root)
            .filter(|handle| {
                host.node(*handle)
                    .is_some_and(|node| node.as_element().is_some())
            })
            .collect::<Vec<_>>();
        if let Some(shadow_host) = host.shadow_root_host(shadow_root) {
            roots.push(shadow_host);
        }
        stylesheet_invalidation_roots.extend(dom_adapter.process_stylesheet_invalidations(
            host,
            &roots,
            &invalidations,
        ));
    }
    if viewport_size_changed {
        stylesheet_invalidation_roots.extend(invalidate_viewport_unit_styles(
            host,
            dom_adapter,
            document,
        ));
    }
    if used_color_scheme_changed {
        invalidation_cleanup.invalidate_subtrees(host, [document]);
    }
    invalidation_cleanup.retain_stylesheet_invalidation_roots(host, stylesheet_invalidation_roots);
    if document_scope_fallback {
        invalidation_cleanup.invalidate_subtrees(host, [document]);
    }
    for shadow_root in shadow_scope_fallbacks {
        let mut roots = vec![shadow_root];
        roots.extend(host.shadow_root_host(shadow_root));
        invalidation_cleanup.invalidate_subtrees(host, roots);
    }
    invalidation_cleanup.invalidate_subtrees(host, removed_shadow_scopes);
}

fn invalidate_viewport_unit_styles(
    host: &DomHost,
    dom_adapter: &StyloDomStyleAdapter,
    document: DomHandle,
) -> Vec<DomHandle> {
    let mut roots = host
        .child_handles(document)
        .filter(|handle| {
            host.node(*handle)
                .is_some_and(|node| node.as_element().is_some())
        })
        .collect::<Vec<_>>();
    for shadow_root in host
        .snapshot_connected_shadow_roots()
        .into_iter()
        .filter(|root| host.owner_document_handle(*root) == Some(document))
    {
        roots.extend(host.child_handles(shadow_root).filter(|handle| {
            host.node(*handle)
                .is_some_and(|node| node.as_element().is_some())
        }));
    }
    let changed_roots = dom_adapter.with_bound_host(host, |binding| {
        roots
            .iter()
            .copied()
            .filter(|root| {
                binding
                    .element(host, *root)
                    .is_some_and(style::invalidation::viewport_units::invalidate)
            })
            .collect::<Vec<_>>()
    });
    dom_adapter.collect_dirty_style_roots(host, &changed_roots)
}
