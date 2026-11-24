use std::collections::{HashMap, HashSet};

use crate::{
    document_runtime::DomHandle, native_bridge::PageScriptHost,
    style_engine::StyloComputedStyleSnapshot,
};

use super::style_resolver::layout_style_read_scope;

pub(super) type DocumentScreenStyles = HashMap<DomHandle, StyloComputedStyleSnapshot>;
pub(crate) type ScreenStyleBaseline = HashMap<DomHandle, DocumentScreenStyles>;

pub(crate) fn refreshable_screen_styles(
    runtime: &PageScriptHost,
    document: DomHandle,
    roots: Vec<DomHandle>,
    baseline: &ScreenStyleBaseline,
) -> Option<Vec<(DomHandle, StyloComputedStyleSnapshot)>> {
    let published = baseline.get(&document)?;
    let host = runtime.dom_host();
    // Affected subtrees may cross slot inheritance and have lazy pseudos
    // with their own style inputs. Check only subtrees with complete coverage.
    if roots.iter().any(|&root| {
        !host.is_connected(root)
            || host.owner_document_handle(root) != Some(document)
            || host.containing_shadow_root(root).is_some()
    }) {
        return None;
    }
    let viewport = runtime.layout_viewport_for_document(document);
    let mut observation = layout_style_read_scope(runtime, document, viewport, None);
    let mut visited = HashSet::new();
    let refreshed = observation.with_resolution_pass(document, |pass| {
        let mut remaining = roots;
        let mut paint_updates = Vec::new();
        while let Some(handle) = remaining.pop() {
            if !visited.insert(handle) {
                continue;
            }
            let previous = published.get(&handle)?;
            if host.shadow_root_handle(handle).is_some()
                || host.assigned_slot_for_node(handle).is_some()
            {
                return None;
            }
            let current = pass.element_styles(handle)?;
            if !previous.has_same_rendering_as(&current) {
                if !previous.only_background_color_changed(&current) {
                    return None;
                }
                paint_updates.push((handle, current));
            }
            remaining.extend(
                host.child_handles(handle)
                    .filter(|&child| host.node(child).is_some_and(|node| node.is_element())),
            );
        }
        Some(paint_updates)
    });
    if moli_trace::cpu_profile_enabled() {
        tracing::info!(
            target: "moli_cpu_profile",
            stage = "screen_style_comparison",
            document = document.index_u32(),
            compared_nodes = visited.len(),
            reusable_geometry = refreshed.is_some(),
            paint_updates = refreshed.as_ref().map_or(0, Vec::len),
        );
    }
    refreshed
}
