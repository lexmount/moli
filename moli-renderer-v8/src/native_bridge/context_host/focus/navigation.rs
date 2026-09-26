use super::JsContextHost;
use crate::document_runtime::DomHandle;
use crate::range_boundary::RangeBoundaryPoint;

#[derive(Clone, Copy)]
pub(crate) enum SequentialFocusStartingPoint {
    Element(DomHandle),
    Position(RangeBoundaryPoint),
}

impl SequentialFocusStartingPoint {
    pub(crate) fn element(self) -> Option<DomHandle> {
        match self {
            Self::Element(element) => Some(element),
            Self::Position(_) => None,
        }
    }

    pub(super) fn container(self) -> DomHandle {
        match self {
            Self::Element(element) => element,
            Self::Position(point) => point.container(),
        }
    }
}

impl JsContextHost {
    pub(crate) fn has_sequential_focus_starting_points(&self) -> bool {
        self.document_focus_changes
            .values()
            .any(|state| state.sequential_starting_point.is_some())
    }

    pub(crate) fn update_sequential_focus_starting_points_for_text_split(
        &mut self,
        original: DomHandle,
        new_text: DomHandle,
        offset: u32,
    ) {
        let dom = unsafe { &*self.runtime }.dom_host();
        for state in self.document_focus_changes.values_mut() {
            if let Some(SequentialFocusStartingPoint::Position(point)) =
                &mut state.sequential_starting_point
            {
                point.update_for_text_split(dom, original, new_text, offset);
            }
        }
    }

    pub(crate) fn update_sequential_focus_starting_points_for_child_removal(
        &mut self,
        parent: DomHandle,
        removed_child: DomHandle,
        previous_sibling: Option<DomHandle>,
    ) {
        let dom = unsafe { &*self.runtime }.dom_host();
        for state in self.document_focus_changes.values_mut() {
            let Some(start) = &mut state.sequential_starting_point else {
                continue;
            };
            let mut ancestor = Some(start.container());
            let mut removed = false;
            while let Some(node) = ancestor {
                if node == removed_child {
                    removed = true;
                    break;
                }
                ancestor = dom.parent_node(node).or_else(|| dom.shadow_root_host(node));
            }
            if removed {
                // A removed element leaves a position, not the parent element:
                // backward navigation must include the previous sibling's last
                // descendant. This position never changes the DOM Selection.
                if let Some(mut point) = RangeBoundaryPoint::set_to_start_of_node(dom, parent) {
                    point.set_child_before_boundary(dom, previous_sibling);
                    *start = SequentialFocusStartingPoint::Position(point);
                }
            } else if let SequentialFocusStartingPoint::Position(point) = start
                && point.container() == parent
                && point.child_before() == Some(removed_child)
            {
                // The anchor can itself be removed or moved after focus was
                // cleared. Keep the gap in its original parent.
                point.set_child_before_boundary(dom, previous_sibling);
            }
        }
    }
}
