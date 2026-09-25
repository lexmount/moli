use std::time::Instant;

use crate::{
    dom::native::{DomHost, DomMutationEffects},
    native_bridge::JsContextHost,
    observer_runtime,
    style_engine::StyleMutationEffect,
};

pub(crate) struct MutationNotificationTimings {
    pub(super) style_effect_count: usize,
    pub(super) style_effects_us: u128,
    pub(super) style_invalidation_us: u128,
    pub(super) observer_us: u128,
}

/// Notifications derived entirely from committed DOM changes, shared by the
/// parser and DOM APIs. No author scripts execute at this notification boundary.
pub(crate) fn notify_dom_mutation(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    dom_host: &DomHost,
    effects: &DomMutationEffects,
) -> MutationNotificationTimings {
    let host = unsafe { &mut *host_ptr };
    if !effects.tree().disconnected_roots().is_empty() {
        host.clear_disconnected_document_focus();
    }
    host.note_inserted_autofocus_candidates(effects.tree().connected_roots());
    if !effects.stylesheet_owners().changes().is_empty() {
        host.apply_stylesheet_owner_changes(effects.stylesheet_owners().changes());
    }
    host.note_app_manifest_link_mutation(dom_host, effects);

    let profile = moli_trace::cpu_profile_enabled();
    let started = profile.then(Instant::now);
    let style_effects = StyleMutationEffect::from_dom_mutation_effects(dom_host, effects);
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

    let started = profile.then(Instant::now);
    observer_runtime::queue_mutation_records(scope, host_ptr, dom_host, effects);
    let observer_us = started
        .map(|started| started.elapsed().as_micros())
        .unwrap_or_default();
    MutationNotificationTimings {
        style_effect_count,
        style_effects_us,
        style_invalidation_us,
        observer_us,
    }
}
