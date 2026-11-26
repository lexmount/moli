use std::time::Instant;

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, DomMutationEffects, Node},
    native_bridge::PageScriptHost,
    observer_runtime,
    style_engine::StyleMutationEffect,
};

pub(crate) struct MutationNotificationTimings {
    pub(super) style_effect_count: usize,
    pub(super) style_effects_us: u128,
    pub(super) style_invalidation_us: u128,
    pub(super) observer_us: u128,
}

/// Notifications derived entirely from the committed DOM changes. This layer
/// does not need a DocumentRuntime or prepare scripts for any document owner.
pub(crate) fn notify_dom_mutation(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    dom_host: &DomHost,
    effects: &DomMutationEffects,
    state_preserving_move: bool,
) -> MutationNotificationTimings {
    if !state_preserving_move {
        observer_runtime::discard_realtime_resize_observer_tree_boxes(
            unsafe { &*host_ptr },
            dom_host,
            effects,
        );
    }
    crate::native_bridge::document::detached_record_native_tree_mutations(
        scope, host_ptr, dom_host, effects,
    );
    let timings = notify_native_dom_mutation(host_ptr, dom_host, effects);
    let host = unsafe { &mut *host_ptr };
    if host.emulated_media().os_text_scale_percent.is_some() {
        let documents = documents_with_changed_text_scale_meta(dom_host, effects);
        if !documents.is_empty() {
            host.note_text_scale_meta_change();
            for document in documents {
                crate::native_bridge::document::sync_document_fonts_for_handle(
                    scope, host, document,
                );
            }
        }
    }
    crate::context_bootstrap::web_mcp::note_mutation(
        scope,
        unsafe { &mut *host_ptr },
        dom_host,
        effects,
    );
    let profile = moli_trace::cpu_profile_enabled();
    let started = profile.then(Instant::now);
    observer_runtime::queue_mutation_records(scope, host_ptr, dom_host, effects);
    let observer_us = started
        .map(|started| started.elapsed().as_micros())
        .unwrap_or_default();
    MutationNotificationTimings {
        observer_us,
        ..timings
    }
}

fn documents_with_changed_text_scale_meta(
    host: &DomHost,
    effects: &DomMutationEffects,
) -> Vec<DomHandle> {
    let style = effects.style();
    let attribute_changes = style
        .attribute_mutations()
        .iter()
        .filter(|change| {
            matches!(change.local_name(), "name" | "content")
                && change.namespace().is_none()
                && host.is_html_element_named(change.target(), "meta")
        })
        .map(|change| change.target());
    let tree_changes = style
        .child_list_mutations()
        .iter()
        .filter(|change| {
            change
                .added_nodes()
                .iter()
                .chain(change.removed_nodes())
                .any(|root| {
                    host.is_html_element_named(*root, "meta")
                        || !host
                            .html_elements_by_local_name_in_shadow_including_subtree(*root, "meta")
                            .is_empty()
                })
        })
        .map(|change| change.target());
    let mut documents: Vec<_> = attribute_changes
        .chain(tree_changes)
        .filter_map(|target| host.root_node_handle(target))
        .filter(|root| host.node(*root).is_some_and(Node::is_document))
        .collect();
    documents.sort_unstable_by_key(|document| document.index());
    documents.dedup();
    documents
}

/// Style and resource identity notifications do not require a Window realm.
/// No wrappers, MutationObservers or WebMCP scripts exist before that realm.
pub(crate) fn notify_native_dom_mutation(
    host_ptr: *mut PageScriptHost,
    dom_host: &DomHost,
    effects: &DomMutationEffects,
) -> MutationNotificationTimings {
    let host = unsafe { &mut *host_ptr };
    host.refresh_mobile_viewport_after_mutation(effects);
    if !effects.stylesheet_owners().changes().is_empty() {
        host.apply_stylesheet_owner_changes(effects.stylesheet_owners().changes());
    }
    host.note_app_manifest_link_mutation(dom_host, effects);

    let profile = moli_trace::cpu_profile_enabled();
    let started = profile.then(Instant::now);
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(effects);
    let style_effect_count = style_effects.len();
    let style_effects_us = started
        .map(|started| started.elapsed().as_micros())
        .unwrap_or_default();
    let started = profile.then(Instant::now);
    if !style_effects.is_empty() {
        host.note_style_mutation_effects(&style_effects);
    }
    let style_invalidation_us = started
        .map(|started| started.elapsed().as_micros())
        .unwrap_or_default();

    host.request_realtime_rendering_opportunity();
    MutationNotificationTimings {
        style_effect_count,
        style_effects_us,
        style_invalidation_us,
        observer_us: 0,
    }
}
