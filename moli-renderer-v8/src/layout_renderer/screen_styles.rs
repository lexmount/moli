use std::collections::HashMap;

use style::{
    invalidation::element::restyle_hints::RestyleHint, servo::restyle_damage::ServoRestyleDamage,
};

use crate::{
    document_runtime::DomHandle, native_bridge::PageScriptHost,
    style_engine::StyleInvalidationScope,
};

use super::style_resolver::layout_style_read_scope;

/// Settle outstanding selector work while the current screen layout may still
/// be reused. The caller checks canonical paint damage after settling the paths.
pub(crate) fn screen_targets_allow_layout_reuse(
    runtime: &PageScriptHost,
    document: DomHandle,
    targets: Vec<(DomHandle, StyleInvalidationScope)>,
    after: u64,
    supported_damage: ServoRestyleDamage,
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
            let Some(child_hint) =
                pass.resolve_screen_target_for_reuse(handle, after, supported_damage)
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
