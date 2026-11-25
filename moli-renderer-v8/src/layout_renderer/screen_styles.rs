use std::collections::{HashMap, HashSet};

use style::servo::restyle_damage::ServoRestyleDamage;

use crate::{
    document_runtime::DomHandle, native_bridge::PageScriptHost,
    style_engine::StyloComputedStyleSnapshot,
};

use super::style_resolver::layout_style_read_scope;

pub(super) type DocumentScreenStyles = HashMap<DomHandle, StyloComputedStyleSnapshot>;
pub(crate) type ScreenStyleBaseline = HashMap<DomHandle, DocumentScreenStyles>;

/// Returns the observed damage. Once it exceeds the consumer's capacity the
/// remaining subtrees can be left for layout to resolve.
pub(crate) fn screen_styles_damage(
    runtime: &PageScriptHost,
    document: DomHandle,
    roots: Vec<DomHandle>,
    baseline: &ScreenStyleBaseline,
    supported: ServoRestyleDamage,
) -> ServoRestyleDamage {
    let Some(published) = baseline.get(&document) else {
        return ServoRestyleDamage::reconstruct();
    };
    let host = runtime.dom_host();
    // Affected subtrees may cross slot inheritance and have lazy pseudos
    // with their own style inputs. Check only subtrees with complete coverage.
    if roots.iter().any(|&root| {
        !host.is_connected(root)
            || host.owner_document_handle(root) != Some(document)
            || host.containing_shadow_root(root).is_some()
    }) {
        return ServoRestyleDamage::reconstruct();
    }
    let viewport = runtime.layout_viewport_for_document(document);
    let mut observation = layout_style_read_scope(runtime, document, viewport, None);
    let mut visited = HashSet::new();
    let damage = observation.with_resolution_pass(document, |pass| {
        let mut remaining = roots;
        let mut damage = ServoRestyleDamage::empty();
        while let Some(handle) = remaining.pop() {
            if !visited.insert(handle) {
                continue;
            }
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
            damage |= previous.rendering_damage(&current);
            if !supported.contains(damage) {
                return damage;
            }
            remaining.extend(
                host.child_handles(handle)
                    .filter(|&child| host.node(child).is_some_and(|node| node.is_element())),
            );
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
