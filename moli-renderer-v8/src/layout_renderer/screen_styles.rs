use std::collections::HashMap;

use style::servo::restyle_damage::ServoRestyleDamage;

use crate::{
    document_runtime::DomHandle, native_bridge::PageScriptHost,
    style_engine::StyleInvalidationScope,
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
            let Some(current_scope) = pass.resolve_screen_style_target(handle, after, supported)
            else {
                return false;
            };
            if scope.max(current_scope) == StyleInvalidationScope::Subtree {
                remaining.extend(host.child_handles(handle).filter_map(|child| {
                    host.node(child)
                        .is_some_and(|node| node.is_element())
                        .then_some((child, StyleInvalidationScope::Subtree))
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
