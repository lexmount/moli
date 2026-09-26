use crate::document_runtime::DomHandle;
use crate::dom::native::{DomHost, Node, NodeType};

#[cfg(test)]
use crate::dom::native::NativeDom;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RangeBoundaryOffset {
    Valid(u32),
    #[cfg(test)]
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RangeBoundaryPoint {
    container: DomHandle,
    child_before_boundary: Option<DomHandle>,
    dom_tree_version: u64,
    offset_in_container: RangeBoundaryOffset,
}

impl RangeBoundaryPoint {
    pub(crate) fn new(dom_host: &DomHost, container: DomHandle, offset: u32) -> Option<Self> {
        dom_host.node(container)?;
        if is_character_data_container(dom_host, container) {
            return Some(Self {
                container,
                child_before_boundary: None,
                dom_tree_version: dom_host.dom_version(),
                offset_in_container: RangeBoundaryOffset::Valid(offset),
            });
        }

        let child_before_boundary = if offset == 0 {
            None
        } else {
            dom_host.nth_child(container, usize::try_from(offset - 1).ok()?)
        };
        if offset > 0 && child_before_boundary.is_none() {
            return None;
        }
        Some(Self {
            container,
            child_before_boundary,
            dom_tree_version: dom_host.dom_version(),
            offset_in_container: RangeBoundaryOffset::Valid(offset),
        })
    }

    pub(crate) fn new_for_offset_validation(
        dom_host: &DomHost,
        container: DomHandle,
        offset: u32,
    ) -> Option<Self> {
        dom_host.node(container)?;
        if is_character_data_container(dom_host, container) {
            return Self::new(dom_host, container, offset);
        }

        let child_before_boundary = if offset == 0 {
            None
        } else {
            dom_host.nth_child(container, usize::try_from(offset - 1).ok()?)
        };
        Some(Self {
            container,
            child_before_boundary,
            dom_tree_version: dom_host.dom_version(),
            offset_in_container: RangeBoundaryOffset::Valid(offset),
        })
    }

    pub(crate) fn container(&self) -> DomHandle {
        self.container
    }

    pub(crate) fn child_before(&self) -> Option<DomHandle> {
        self.child_before_boundary
    }

    pub(crate) fn update_for_character_data_edit(
        &mut self,
        dom_host: &DomHost,
        target: DomHandle,
        edit_offset: u32,
        removed_count: u32,
        inserted_count: u32,
    ) {
        if self.container != target {
            return;
        }
        let Some(current) = self.offset(dom_host) else {
            return;
        };
        let next = if current <= edit_offset {
            current
        } else if current <= edit_offset.saturating_add(removed_count) {
            edit_offset
        } else {
            current
                .saturating_sub(removed_count)
                .saturating_add(inserted_count)
        };
        if let Some(point) = Self::new(dom_host, target, next) {
            *self = point;
        }
    }

    pub(crate) fn update_for_text_split(
        &mut self,
        dom_host: &DomHost,
        original: DomHandle,
        new_text: DomHandle,
        offset: u32,
    ) {
        if self.container == original {
            if let Some(current) = self.offset(dom_host)
                && current > offset
                && let Some(point) = Self::new(dom_host, new_text, current - offset)
            {
                *self = point;
            }
        } else if self.child_before_boundary == Some(original)
            && dom_host.parent_node(original) == Some(self.container)
        {
            // A parent boundary immediately after the original text must stay
            // after both halves. Later siblings already track their own index.
            self.set_child_before_boundary(dom_host, Some(new_text));
        }
    }

    pub(crate) fn set_child_before_boundary(
        &mut self,
        dom_host: &DomHost,
        child_before: Option<DomHandle>,
    ) -> bool {
        if is_character_data_container(dom_host, self.container) {
            return child_before.is_none();
        }

        let next_offset = match child_before {
            Some(child_before) => {
                if dom_host
                    .node(child_before)
                    .and_then(|node| node.parent_node())
                    != Some(self.container)
                {
                    return false;
                }
                let Some(index) = dom_host.child_index(self.container, child_before) else {
                    return false;
                };
                let Some(offset) = u32::try_from(index + 1).ok() else {
                    return false;
                };
                RangeBoundaryOffset::Valid(offset)
            }
            None => RangeBoundaryOffset::Valid(0),
        };

        self.child_before_boundary = child_before;
        self.offset_in_container = next_offset;
        self.mark_valid(dom_host);
        true
    }

    pub(crate) fn offset(&mut self, dom_host: &DomHost) -> Option<u32> {
        self.ensure_offset_is_valid(dom_host)?;
        match self.offset_in_container {
            RangeBoundaryOffset::Valid(offset) => Some(offset),
            #[cfg(test)]
            RangeBoundaryOffset::Invalid => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_to_before_child(dom_host: &DomHost, child: DomHandle) -> Option<Self> {
        let parent = dom_host.node(child)?.parent_node()?;
        Some(Self {
            container: parent,
            child_before_boundary: dom_host.node(child)?.prev_sibling(),
            dom_tree_version: dom_host.dom_version(),
            offset_in_container: RangeBoundaryOffset::Invalid,
        })
    }

    pub(crate) fn set_to_start_of_node(dom_host: &DomHost, container: DomHandle) -> Option<Self> {
        dom_host.node(container)?;
        Some(Self {
            container,
            child_before_boundary: None,
            dom_tree_version: dom_host.dom_version(),
            offset_in_container: RangeBoundaryOffset::Valid(0),
        })
    }

    #[cfg(test)]
    pub(crate) fn set_to_end_of_node(dom_host: &DomHost, container: DomHandle) -> Option<Self> {
        let node = dom_host.node(container)?;
        if is_character_data_container(dom_host, container) {
            return Some(Self {
                container,
                child_before_boundary: None,
                dom_tree_version: dom_host.dom_version(),
                offset_in_container: RangeBoundaryOffset::Valid(
                    node.node_value()
                        .map(|value| value.encode_utf16().count() as u32)
                        .unwrap_or(0),
                ),
            });
        }
        Some(Self {
            container,
            child_before_boundary: node.last_child(),
            dom_tree_version: dom_host.dom_version(),
            offset_in_container: if node.last_child().is_some() {
                RangeBoundaryOffset::Invalid
            } else {
                RangeBoundaryOffset::Valid(0)
            },
        })
    }

    #[cfg(test)]
    pub(crate) fn child_before_will_be_removed(&mut self, dom_host: &DomHost) {
        let Some(child_before) = self.child_before_boundary else {
            return;
        };
        self.child_before_boundary = dom_host
            .node(child_before)
            .and_then(|node| node.prev_sibling());
        if !self.is_offset_valid(dom_host) {
            return;
        }
        let RangeBoundaryOffset::Valid(offset) = self.offset_in_container else {
            return;
        };
        self.offset_in_container = if self.child_before_boundary.is_none() {
            RangeBoundaryOffset::Valid(0)
        } else {
            RangeBoundaryOffset::Valid(offset.saturating_sub(1))
        };
        self.mark_valid(dom_host);
    }

    fn ensure_offset_is_valid(&mut self, dom_host: &DomHost) -> Option<()> {
        if self.is_offset_valid(dom_host) {
            return Some(());
        }
        if is_character_data_container(dom_host, self.container) {
            return Some(());
        }
        self.offset_in_container = match self.child_before_boundary {
            Some(child_before) => RangeBoundaryOffset::Valid(
                u32::try_from(dom_host.child_index(self.container, child_before)? + 1).ok()?,
            ),
            None => RangeBoundaryOffset::Valid(0),
        };
        self.mark_valid(dom_host);
        Some(())
    }

    fn is_offset_valid(&self, dom_host: &DomHost) -> bool {
        matches!(self.offset_in_container, RangeBoundaryOffset::Valid(_))
            && (self.dom_tree_version == dom_host.dom_version()
                || is_character_data_container(dom_host, self.container))
    }

    fn mark_valid(&mut self, dom_host: &DomHost) {
        self.dom_tree_version = dom_host.dom_version();
    }
}

fn is_character_data_container(dom_host: &DomHost, handle: DomHandle) -> bool {
    dom_host.node(handle).is_some_and(|node| {
        matches!(
            node.node_type(),
            NodeType::Text
                | NodeType::CDataSection
                | NodeType::ProcessingInstruction
                | NodeType::Comment
        )
    })
}

/// Compares two native DOM boundary points using the DOM Range ordering
/// algorithm. This is the shared owner for both JS Range geometry and the
/// one-shot paint selection adapter.
pub(crate) fn point_order_in_dom(
    dom_host: &DomHost,
    a_container: DomHandle,
    a_offset: u32,
    b_container: DomHandle,
    b_offset: u32,
) -> Option<std::cmp::Ordering> {
    if a_container == b_container {
        return Some(a_offset.cmp(&b_offset));
    }

    fn chain(dom_host: &DomHost, mut handle: DomHandle) -> Vec<DomHandle> {
        let mut chain = vec![handle];
        while let Some(parent) = dom_host.node(handle).and_then(Node::parent_node) {
            chain.push(parent);
            handle = parent;
        }
        chain
    }

    let a_chain = chain(dom_host, a_container);
    let b_chain = chain(dom_host, b_container);
    if a_chain.last()? != b_chain.last()? {
        return None;
    }

    for index in 1..a_chain.len() {
        if a_chain[index] == b_container {
            let child_index = dom_host.child_index(b_container, a_chain[index - 1])?;
            let child_index = u32::try_from(child_index).ok()?;
            return Some(if b_offset <= child_index {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            });
        }
    }

    for index in 1..b_chain.len() {
        if b_chain[index] == a_container {
            let child_index = dom_host.child_index(a_container, b_chain[index - 1])?;
            let child_index = u32::try_from(child_index).ok()?;
            return Some(if child_index < a_offset {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            });
        }
    }

    let mut a_index = a_chain.len();
    let mut b_index = b_chain.len();
    while a_index > 0 && b_index > 0 && a_chain[a_index - 1] == b_chain[b_index - 1] {
        a_index -= 1;
        b_index -= 1;
    }
    if a_index == 0 || b_index == 0 {
        return None;
    }
    let common_ancestor = a_chain[a_index];
    let a_child_index = dom_host.child_index(common_ancestor, a_chain[a_index - 1])?;
    let b_child_index = dom_host.child_index(common_ancestor, b_chain[b_index - 1])?;
    Some(a_child_index.cmp(&b_child_index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_host() -> DomHost {
        DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://range-boundary-point.test/").unwrap(),
        ))
    }

    fn parent_with_three_children() -> (DomHost, DomHandle, DomHandle, DomHandle, DomHandle) {
        let mut host = test_host();
        let parent = host.create_element("section");
        let first = host.create_element("a");
        let second = host.create_element("b");
        let third = host.create_element("c");
        assert!(host.append_child(parent, first));
        assert!(host.append_child(parent, second));
        assert!(host.append_child(parent, third));
        (host, parent, first, second, third)
    }

    #[test]
    fn native_boundary_point_repairs_offset_from_child_before_after_insert() {
        let (mut host, parent, first, second, _) = parent_with_three_children();
        let mut point = RangeBoundaryPoint::new(&host, parent, 2).unwrap();

        assert_eq!(point.child_before(), Some(second));
        assert_eq!(point.offset(&host), Some(2));

        let inserted = host.create_element("inserted");
        assert!(host.insert_before(parent, inserted, Some(first)));

        assert_eq!(point.child_before(), Some(second));
        assert_eq!(point.offset(&host), Some(3));
    }

    #[test]
    fn native_boundary_point_moves_child_before_to_previous_before_removal() {
        let (mut host, parent, first, second, _) = parent_with_three_children();
        let mut point = RangeBoundaryPoint::new(&host, parent, 2).unwrap();

        point.child_before_will_be_removed(&host);
        assert_eq!(point.child_before(), Some(first));
        assert!(host.remove_child(parent, second));

        assert_eq!(point.child_before(), Some(first));
        assert_eq!(point.offset(&host), Some(1));
    }

    #[test]
    fn native_boundary_point_before_child_uses_previous_sibling_anchor() {
        let (mut host, parent, first, second, _) = parent_with_three_children();
        let mut point = RangeBoundaryPoint::set_to_before_child(&host, second).unwrap();

        assert_eq!(point.container(), parent);
        assert_eq!(point.child_before(), Some(first));
        assert_eq!(point.offset(&host), Some(1));

        let inserted = host.create_element("inserted");
        assert!(host.insert_before(parent, inserted, Some(second)));

        assert_eq!(point.child_before(), Some(first));
        assert_eq!(point.offset(&host), Some(1));
    }

    #[test]
    fn native_boundary_point_end_of_character_data_uses_utf16_offset() {
        let mut host = test_host();
        let text = host.create_text_node("a\u{1f4a1}b");
        let mut point = RangeBoundaryPoint::set_to_end_of_node(&host, text).unwrap();

        assert_eq!(point.child_before(), None);
        assert_eq!(point.offset(&host), Some(4));
    }

    #[test]
    fn native_boundary_point_validation_candidate_preserves_out_of_bounds_offset() {
        let (host, parent, _, _, _) = parent_with_three_children();
        let mut point = RangeBoundaryPoint::new_for_offset_validation(&host, parent, 99).unwrap();

        assert_eq!(point.child_before(), None);
        assert_eq!(point.offset(&host), Some(99));
    }
}
