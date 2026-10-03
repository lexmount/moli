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

/// Notifications derived entirely from the committed DOM changes. This layer
/// does not need a DocumentRuntime or prepare scripts for any document owner.
pub(crate) fn notify_dom_mutation(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    dom_host: &DomHost,
    effects: &DomMutationEffects,
) -> MutationNotificationTimings {
    crate::native_bridge::document::detached_record_native_tree_mutations(
        scope, host_ptr, dom_host, effects,
    );
    let host = unsafe { &mut *host_ptr };
    if !effects.stylesheet_owners().changes().is_empty() {
        host.apply_stylesheet_owner_changes(effects.stylesheet_owners().changes());
    }
    host.note_app_manifest_link_mutation(dom_host, effects);
    crate::context_bootstrap::web_mcp::note_mutation(scope, host, dom_host, effects);

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
