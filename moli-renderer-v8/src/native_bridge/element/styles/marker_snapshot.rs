use std::{collections::HashSet, ops::ControlFlow};

use indexmap::IndexMap;
use moli_layout::{
    ChildBoxPolicy, LayoutDisplay, LayoutElementCategory, LayoutFormControlKind,
    LayoutInputControlKind,
};

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Node},
    dom_presentation::{
        element_child_box_policy, layout_element_classification, suppresses_noscript_box,
    },
    native_bridge::PageScriptHost,
    style_engine::StyleResolutionPass,
};

use super::{StyleObservation, ancestry::AncestorQueries};

#[derive(Default)]
struct MarkerTargets {
    elements: Vec<DomHandle>,
    child_frames: Vec<(DomHandle, DomHandle)>,
}

pub(crate) fn marker_hosts_for_document_snapshot(
    host: &PageScriptHost,
    handles: impl IntoIterator<Item = DomHandle>,
) -> HashSet<DomHandle> {
    let dom = host.dom_host();
    let root_document = host.document_handle();
    let mut flat_tree = AncestorQueries::default();
    let mut documents = IndexMap::<DomHandle, MarkerTargets>::new();
    for handle in handles {
        let Some(document) = dom
            .node(handle)
            .filter(|node| node.is_element())
            .and_then(Node::owner_document)
        else {
            continue;
        };
        if flat_tree_document(dom, &mut flat_tree, handle) == Some(document) {
            documents.entry(document).or_default().elements.push(handle);
        }
    }
    if documents.is_empty() {
        return HashSet::new();
    }

    // Include the frame owners even for a request targeting a single child
    // document. Check each owner in its parent's style pass.
    let mut index = 0;
    while let Some((&document, _)) = documents.get_index(index) {
        index += 1;
        if document != root_document
            && let Some(frame) = host.child_browsing_context_host_for_document_handle(document)
            && let Some(parent) = dom.owner_document_handle(frame)
            && flat_tree_document(dom, &mut flat_tree, frame) == Some(parent)
        {
            documents
                .entry(parent)
                .or_default()
                .child_frames
                .push((frame, document));
        }
    }

    let mut hosts = HashSet::new();
    let mut pending = vec![root_document];
    let mut children_participate = AncestorQueries::default();
    let mut observation = StyleObservation::new(host);
    while let Some(document) = pending.pop() {
        let Some(targets) = documents.get(&document) else {
            continue;
        };
        let scripting = host.document_scripting_enabled(document);
        observation.with_resolution_pass(document, |pass| {
            for &element in &targets.elements {
                let Some(style) = pass.primary_style(element) else {
                    continue;
                };
                if style.clone_display().is_list_item()
                    && participating_children(dom, element, LayoutDisplay::from(&*style), scripting)
                        == Some(ChildBoxPolicy::All)
                    && ancestors_retain_children(
                        dom,
                        &mut children_participate,
                        pass,
                        element,
                        scripting,
                    )
                    && pass.marker_for_generation(element).is_some()
                {
                    hosts.insert(element);
                }
            }
            for &(frame, child_document) in &targets.child_frames {
                let Some(style) = pass.primary_style(frame) else {
                    continue;
                };
                let display = LayoutDisplay::from(&*style);
                if display != LayoutDisplay::Contents
                    && participating_children(dom, frame, display, scripting).is_some()
                    && ancestors_retain_children(
                        dom,
                        &mut children_participate,
                        pass,
                        frame,
                        scripting,
                    )
                {
                    pending.push(child_document);
                }
            }
            if moli_trace::cpu_profile_enabled() {
                pass.trace_profile();
            }
        });
    }
    hosts
}

fn flat_tree_document(
    dom: &DomHost,
    queries: &mut AncestorQueries<Option<DomHandle>>,
    handle: DomHandle,
) -> Option<DomHandle> {
    queries.resolve(handle, |current| {
        if let Some(parent) = dom.flat_tree_parent(current) {
            return ControlFlow::Continue(parent);
        }
        ControlFlow::Break(
            dom.node(current)
                .is_some_and(Node::is_document)
                .then_some(current),
        )
    })
}

fn ancestors_retain_children(
    dom: &DomHost,
    queries: &mut AncestorQueries<bool>,
    pass: &mut StyleResolutionPass<'_, '_>,
    handle: DomHandle,
    scripting: bool,
) -> bool {
    let Some(parent) = dom.flat_tree_parent(handle) else {
        return false;
    };
    queries.resolve(parent, |ancestor| {
        if ancestor == pass.document() {
            return ControlFlow::Break(true);
        }
        if let Some(style) = pass.primary_style(ancestor)
            && participating_children(dom, ancestor, LayoutDisplay::from(&*style), scripting)
                == Some(ChildBoxPolicy::All)
            && let Some(parent) = dom.flat_tree_parent(ancestor)
        {
            return ControlFlow::Continue(parent);
        }
        ControlFlow::Break(false)
    })
}

// Leaf elements can have their own box (including an iframe viewport) while
// discarding the DOM and generated child stream.
fn participating_children(
    dom: &DomHost,
    handle: DomHandle,
    display: LayoutDisplay,
    scripting: bool,
) -> Option<ChildBoxPolicy> {
    if display == LayoutDisplay::None || suppresses_noscript_box(dom, handle, scripting) {
        return None;
    }
    let element = dom.element(handle)?;
    let (category, replaced) = layout_element_classification(element);
    if category
        == LayoutElementCategory::FormControl(LayoutFormControlKind::Input(
            LayoutInputControlKind::Hidden,
        ))
    {
        return None;
    }
    let policy = element_child_box_policy(element, category, replaced.is_some(), display);
    (display != LayoutDisplay::Contents || policy != ChildBoxPolicy::None).then_some(policy)
}
