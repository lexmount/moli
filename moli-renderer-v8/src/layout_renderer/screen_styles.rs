use std::collections::HashMap;

use style::servo::restyle_damage::ServoRestyleDamage;

use crate::{
    document_runtime::DomHandle,
    native_bridge::PageScriptHost,
    style_engine::{ScreenStyleScope, StyloComputedStyleSnapshot},
};

use super::style_resolver::layout_style_read_scope;

pub(super) type DocumentScreenStyles = HashMap<DomHandle, StyloComputedStyleSnapshot>;
pub(crate) type ScreenStyleBaseline = HashMap<DomHandle, DocumentScreenStyles>;

/// Returns the observed damage. Once it exceeds the consumer's capacity the
/// remaining subtrees can be left for layout to resolve.
pub(crate) fn screen_styles_damage(
    runtime: &PageScriptHost,
    document: DomHandle,
    targets: Vec<(DomHandle, ScreenStyleScope)>,
    baseline: &ScreenStyleBaseline,
    supported: ServoRestyleDamage,
    mut record_damage: impl FnMut(DomHandle, ServoRestyleDamage),
) -> ServoRestyleDamage {
    let Some(published) = baseline.get(&document) else {
        return ServoRestyleDamage::reconstruct();
    };
    let host = runtime.dom_host();
    // Affected subtrees may cross slot inheritance and have lazy pseudos
    // with their own style inputs. Check only subtrees with complete coverage.
    if targets.iter().any(|&(root, _)| {
        !host.is_connected(root)
            || host.owner_document_handle(root) != Some(document)
            || host.containing_shadow_root(root).is_some()
    }) {
        return ServoRestyleDamage::reconstruct();
    }
    let viewport = runtime.layout_viewport_for_document(document);
    let mut observation = layout_style_read_scope(runtime, document, viewport, None);
    let mut visited = HashMap::new();
    let damage = observation.with_resolution_pass(document, |pass| {
        let mut remaining = targets;
        let mut damage = ServoRestyleDamage::empty();
        while let Some((handle, scope)) = remaining.pop() {
            if visited.get(&handle).is_some_and(|&seen| seen >= scope) {
                continue;
            }
            visited.insert(handle, scope);
            let Some(previous) = published.get(&handle) else {
                return ServoRestyleDamage::reconstruct();
            };
            if host.shadow_root_handle(handle).is_some()
                || host.assigned_slot_for_node(handle).is_some()
            {
                return ServoRestyleDamage::reconstruct();
            }
            let Some(current) = pass.element_styles(handle) else {
                return ServoRestyleDamage::reconstruct();
            };
            let node_damage = previous.rendering_damage(&current);
            if !node_damage.is_empty() {
                record_damage(handle, node_damage);
                damage |= node_damage;
            }
            if !supported.contains(damage) {
                return damage;
            }
            if scope == ScreenStyleScope::Subtree || previous.descendant_inputs_changed(&current) {
                remaining.extend(
                    host.child_handles(handle)
                        .filter(|&child| host.node(child).is_some_and(|node| node.is_element()))
                        .map(|child| (child, ScreenStyleScope::Subtree)),
                );
            }
        }
        damage
    });
    if moli_trace::cpu_profile_enabled() {
        tracing::info!(
            target: "moli_cpu_profile",
            stage = "screen_style_comparison",
            document = document.index_u32(),
            compared_nodes = visited.len(),
            unchanged = damage.is_empty(),
            ?damage,
        );
    }
    damage
}
