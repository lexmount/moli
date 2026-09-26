use crate::document_runtime::DomHandle;
use crate::native_bridge::{JsContextHost, SequentialFocusStartingPoint};
use std::collections::HashSet;

// The HTML sequential navigation search uses shadow-including DOM order when
// its starting point is not an item in the tabindex-ordered navigation scope.
// In particular, a removed or disabled control must not restart at the page's
// first positive tabindex.
pub(super) fn sequential_target_in_dom_order(
    runtime: &JsContextHost,
    point: SequentialFocusStartingPoint,
    candidates: &[DomHandle],
    reverse: bool,
) -> Option<DomHandle> {
    let dom = runtime.dom_host();
    let (anchor, after_subtree, skip_start) = match point {
        SequentialFocusStartingPoint::Element(element) => (element, false, true),
        SequentialFocusStartingPoint::Position(mut boundary) => {
            let container = boundary.container();
            let offset = boundary.offset(dom)? as usize;
            match dom.nth_child(container, offset) {
                Some(next) => (next, false, false),
                None => (container, true, false),
            }
        }
    };
    let mut nodes = Vec::new();
    let mut position = None;
    let mut stack = vec![(runtime.document_handle(), false)];
    while let Some((node, exiting)) = stack.pop() {
        if node == anchor && exiting == after_subtree {
            position = Some(nodes.len());
        }
        if exiting {
            continue;
        }
        nodes.push(node);
        stack.push((node, true));
        let mut child = dom.node(node).and_then(|node| node.last_child());
        while let Some(handle) = child {
            stack.push((handle, false));
            child = dom.node(handle).and_then(|node| node.prev_sibling());
        }
        if let Some(shadow) = dom.shadow_root_handle(node) {
            stack.push((shadow, false));
        }
    }
    let position = position?;
    let candidates = candidates.iter().copied().collect::<HashSet<_>>();
    if reverse {
        nodes[..position]
            .iter()
            .rev()
            .find(|node| candidates.contains(node))
            .copied()
    } else {
        nodes[position + usize::from(skip_start)..]
            .iter()
            .find(|node| candidates.contains(node))
            .copied()
    }
}
