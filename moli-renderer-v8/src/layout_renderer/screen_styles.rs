use std::collections::{HashMap, HashSet};

use crate::{
    document_runtime::DomHandle, native_bridge::PageScriptHost,
    style_engine::StyloComputedStyleSnapshot,
};

use super::style_resolver::layout_style_read_scope;

pub(super) type DocumentScreenStyles = HashMap<DomHandle, StyloComputedStyleSnapshot>;
pub(crate) type ScreenStyleBaseline = HashMap<DomHandle, DocumentScreenStyles>;

pub(crate) fn screen_styles_are_equivalent(
    runtime: &PageScriptHost,
    document: DomHandle,
    roots: Vec<DomHandle>,
    baseline: &ScreenStyleBaseline,
) -> bool {
    let Some(published) = baseline.get(&document) else {
        return false;
    };
    let host = runtime.dom_host();
    // Affected subtrees may cross slot inheritance and have lazy pseudos
    // with their own style inputs. Check only subtrees with complete coverage.
    if roots.iter().any(|&root| {
        !host.is_connected(root)
            || host.owner_document_handle(root) != Some(document)
            || host.containing_shadow_root(root).is_some()
    }) {
        return false;
    }
    let viewport = runtime.layout_viewport_for_document(document);
    let mut observation = layout_style_read_scope(runtime, document, viewport, None);
    let mut visited = HashSet::new();
    let unchanged = observation.with_resolution_pass(document, |pass| {
        let mut remaining = roots;
        while let Some(handle) = remaining.pop() {
            if !visited.insert(handle) {
                continue;
            }
            let Some(previous) = published.get(&handle) else {
                return false;
            };
            if host.shadow_root_handle(handle).is_some()
                || host.assigned_slot_for_node(handle).is_some()
                || pass
                    .element_styles(handle)
                    .is_none_or(|current| !previous.has_same_rendering_as(&current))
            {
                return false;
            }
            remaining.extend(
                host.child_handles(handle)
                    .filter(|&child| host.node(child).is_some_and(|node| node.is_element())),
            );
        }
        true
    });
    if moli_trace::cpu_profile_enabled() {
        tracing::info!(
            target: "moli_cpu_profile",
            stage = "screen_style_comparison",
            document = document.index_u32(),
            compared_nodes = visited.len(),
            unchanged,
        );
    }
    unchanged
}
