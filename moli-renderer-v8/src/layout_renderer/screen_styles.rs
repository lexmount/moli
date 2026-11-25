use std::collections::HashMap;

use style::{
    invalidation::element::restyle_hints::RestyleHint, servo::restyle_damage::ServoRestyleDamage,
};

use crate::{
    document_runtime::DomHandle,
    native_bridge::PageScriptHost,
    style_engine::{ScreenStyleCoverage, StyleInvalidationScope, computed_style_values_damage},
};

use super::style_resolver::layout_style_read_scope;

/// Resolve pending targets through the canonical style commit. Unsupported
/// scopes and damage are left for a full screen layout.
pub(crate) fn resolve_screen_style_targets(
    runtime: &PageScriptHost,
    document: DomHandle,
    targets: Vec<(DomHandle, StyleInvalidationScope)>,
    after: u64,
    supported: ServoRestyleDamage,
) -> bool {
    let host = runtime.dom_host();
    // Affected subtrees may cross slot inheritance and have lazy pseudos
    // with their own style inputs. Check only subtrees with complete coverage.
    if targets.iter().any(|&(root, _)| {
        !host.is_connected(root)
            || host.owner_document_handle(root) != Some(document)
            || host.containing_shadow_root(root).is_some()
    }) {
        return false;
    }
    let viewport = runtime.layout_viewport_for_document(document);
    let mut observation = layout_style_read_scope(runtime, document, viewport, None);
    let mut visited = HashMap::new();
    let resolved = observation.with_resolution_pass(document, |pass| {
        let mut remaining = targets;
        while let Some((handle, scope)) = remaining.pop() {
            if visited.get(&handle).is_some_and(|&seen| seen >= scope) {
                continue;
            }
            visited.insert(handle, scope);
            if host.shadow_root_handle(handle).is_some()
                || host.assigned_slot_for_node(handle).is_some()
            {
                return false;
            }
            let Some(child_hint) = pass.resolve_screen_style_target(handle, after, supported)
            else {
                return false;
            };
            let subtree =
                scope == StyleInvalidationScope::Subtree || child_hint.will_recascade_subtree();
            if subtree || !child_hint.is_empty() {
                let child_scope = if subtree {
                    StyleInvalidationScope::Subtree
                } else {
                    StyleInvalidationScope::Element
                };
                let only_reset = !subtree
                    && child_hint.contains(RestyleHint::RECASCADE_SELF_IF_INHERIT_RESET_STYLE);
                remaining.extend(host.child_handles(handle).filter_map(|child| {
                    (host.node(child).is_some_and(|node| node.is_element())
                        && (!only_reset || pass.may_inherit_reset_style(child)))
                    .then_some((child, child_scope))
                }));
            }
        }
        true
    });
    if moli_trace::cpu_profile_enabled() {
        tracing::info!(
            target: "moli_cpu_profile",
            stage = "screen_style_validation",
            document = document.index_u32(),
            visited_nodes = visited.len(),
            resolved,
        );
    }
    resolved
}

/// Validate the sources consumed by the publication. A broad Document target
/// is a scope boundary rather than an element, and projected controls can
/// suppress descendants that were never used by layout.
pub(crate) fn validate_region_external_styles(
    runtime: &PageScriptHost,
    document: DomHandle,
    coverage: &ScreenStyleCoverage,
    include: impl Fn(DomHandle) -> bool,
) -> bool {
    let started = moli_trace::cpu_profile_enabled().then(std::time::Instant::now);
    let host = runtime.dom_host();
    let viewport = runtime.layout_viewport_for_document(document);
    let mut observation = layout_style_read_scope(runtime, document, viewport, None);
    let mut visited = 0usize;
    let valid = observation.with_resolution_pass(document, |pass| {
        for &handle in &coverage.sources {
            if !include(handle)
                || !host.is_connected(handle)
                || host.owner_document_handle(handle) != Some(document)
            {
                continue;
            }
            visited += 1;
            if pass.element_styles(handle).is_none() {
                return false;
            }
            if let Some(previous) = coverage.markers.get(&handle) {
                let current = pass.marker_for_generation(handle);
                if !computed_style_values_damage(previous.as_ref(), current.as_ref()).is_empty() {
                    return false;
                }
            }
            if let Some(previous) = coverage.placeholders.get(&handle) {
                let current = pass.pseudo_style(handle, "placeholder");
                if !computed_style_values_damage(previous.as_ref(), current.as_ref()).is_empty() {
                    return false;
                }
            }
        }
        true
    });
    if let Some(started) = started {
        tracing::info!(target: "moli_cpu_profile", stage = "isolated_external_style_validation",
            document = document.index_u32(), compared_nodes = visited,
            total_us = started.elapsed().as_micros(), valid);
    }
    valid
}
