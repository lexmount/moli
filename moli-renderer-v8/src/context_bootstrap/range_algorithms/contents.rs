use super::*;
use crate::native_bridge::throw_dom_exception;

pub(in crate::context_bootstrap) fn range_delete_contents<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let _ = process_contents(scope, RangeContentsAction::Delete, range)?;
    Some(())
}

pub(in crate::context_bootstrap) fn range_extract_contents<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    process_contents(scope, RangeContentsAction::Extract, range)?
}

pub(in crate::context_bootstrap) fn range_clone_contents<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    process_contents(scope, RangeContentsAction::Clone, range)?
}

pub(in crate::context_bootstrap) fn range_surround_contents<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    new_parent: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let snapshot = range_boundary_snapshot(scope, range)?;
    if range_partially_contains_non_text_node(scope, &snapshot) {
        throw_surround_invalid_state_error(scope);
        return None;
    }
    if surround_new_parent_type_is_invalid(scope, new_parent) {
        throw_surround_invalid_node_type_error(scope);
        return None;
    }

    let fragment = range_extract_contents(scope, range)?;
    let fragment_handle = node_handle_for_tree_op(scope, fragment)?;
    let document_handle = document_handle_for_node_handle_or_self(scope, snapshot.start_container);
    let Some(new_parent_handle) = node_handle_for_range_insert(scope, document_handle, new_parent)
    else {
        crate::util::throw_type_error(scope, "Range.surroundContents requires a Node");
        return None;
    };

    remove_all_children(scope, new_parent_handle)?;
    let start_container = range_boundary_container_object(scope, range, RangeBoundarySide::Start)?;
    let start_container_handle = node_handle_for_tree_op(scope, start_container)?;
    let start_offset = range_boundary_offset(scope, range, RangeBoundarySide::Start) as u32;
    range_insert_node_handle_at_boundary(
        scope,
        range,
        start_container_handle,
        start_offset,
        new_parent_handle,
    )?;
    validate_pre_insert_internal_handle(scope, new_parent_handle, fragment_handle, None, &[])?;
    append_child_internal_handle(scope, new_parent_handle, fragment_handle).or_else(|| {
        throw_contents_hierarchy_request_error(scope);
        None
    })?;
    range_select_node_handle(scope, range, new_parent_handle)?;
    Some(())
}

#[derive(Clone, Copy)]
enum RangeContentsAction {
    Clone,
    Extract,
    Delete,
}

impl RangeContentsAction {
    fn produces_fragment(self) -> bool {
        matches!(self, Self::Clone | Self::Extract)
    }

    fn mutates_range_contents(self) -> bool {
        matches!(self, Self::Extract | Self::Delete)
    }
}

struct RangeBoundarySnapshot<'s> {
    start_container_wrapper: Option<v8::Local<'s, v8::Object>>,
    start_container: DomHandle,
    start_offset: u32,
    end_container_wrapper: Option<v8::Local<'s, v8::Object>>,
    end_container: DomHandle,
    end_offset: u32,
}

fn process_contents<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    action: RangeContentsAction,
    range: v8::Local<'s, v8::Object>,
) -> Option<Option<v8::Local<'s, v8::Object>>> {
    let snapshot = range_boundary_snapshot(scope, range)?;
    process_contents_from_snapshot(scope, action, range, snapshot)
}

fn range_boundary_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
) -> Option<RangeBoundarySnapshot<'s>> {
    if let Some(boundaries) = native_range_boundary_handles(scope, range) {
        return Some(RangeBoundarySnapshot {
            start_container_wrapper: None,
            start_container: boundaries.start.container,
            start_offset: boundaries.start.offset,
            end_container_wrapper: None,
            end_container: boundaries.end.container,
            end_offset: boundaries.end.offset,
        });
    }

    let start_container = range_boundary_container_object(scope, range, RangeBoundarySide::Start)?;
    let end_container = range_boundary_container_object(scope, range, RangeBoundarySide::End)?;
    Some(RangeBoundarySnapshot {
        start_container_wrapper: Some(start_container),
        start_container: node_handle_for_tree_op(scope, start_container)?,
        start_offset: range_boundary_offset(scope, range, RangeBoundarySide::Start) as u32,
        end_container_wrapper: Some(end_container),
        end_container: node_handle_for_tree_op(scope, end_container)?,
        end_offset: range_boundary_offset(scope, range, RangeBoundarySide::End) as u32,
    })
}

fn process_contents_from_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    action: RangeContentsAction,
    range: v8::Local<'s, v8::Object>,
    snapshot: RangeBoundarySnapshot<'s>,
) -> Option<Option<v8::Local<'s, v8::Object>>> {
    let document_handle = document_handle_for_node_handle_or_self(scope, snapshot.start_container);
    let fragment = if action.produces_fragment() {
        Some(create_document_fragment_handle(scope, document_handle)?)
    } else {
        None
    };

    if snapshot.start_container == snapshot.end_container
        && snapshot.start_offset == snapshot.end_offset
    {
        return finish_fragment(scope, range, fragment);
    }

    if snapshot.start_container == snapshot.end_container {
        process_contents_between_offsets(
            scope,
            action,
            fragment,
            snapshot.start_container,
            snapshot.start_offset,
            snapshot.end_offset,
        )?;
        if action.mutates_range_contents() {
            collapse_range_to_handle(
                scope,
                range,
                snapshot.start_container,
                snapshot.start_offset,
                Some(&snapshot),
            )?;
        }
        return finish_fragment(scope, range, fragment);
    }

    let common_root =
        common_ancestor_container_handle(scope, snapshot.start_container, snapshot.end_container)?;
    let partial_start =
        highest_ancestor_under_common_root(scope, snapshot.start_container, common_root);
    let partial_end =
        highest_ancestor_under_common_root(scope, snapshot.end_container, common_root);
    let mutation_collapse_target = if action.mutates_range_contents() {
        Some(contents_mutation_collapse_target(
            scope,
            snapshot.start_container,
            snapshot.start_offset,
            snapshot.end_container,
        )?)
    } else {
        None
    };

    let left_contents = if snapshot.start_container == common_root {
        None
    } else {
        let start_length = range_node_length_handle(scope, snapshot.start_container)?;
        let contents = process_contents_between_offsets(
            scope,
            action,
            None,
            snapshot.start_container,
            snapshot.start_offset,
            start_length,
        )?;
        process_ancestors_and_their_siblings(
            scope,
            action,
            snapshot.start_container,
            ContentsProcessDirection::Forward,
            contents,
            common_root,
        )?
    };

    let right_contents = if snapshot.end_container == common_root {
        None
    } else {
        let contents = process_contents_between_offsets(
            scope,
            action,
            None,
            snapshot.end_container,
            0,
            snapshot.end_offset,
        )?;
        process_ancestors_and_their_siblings(
            scope,
            action,
            snapshot.end_container,
            ContentsProcessDirection::Backward,
            contents,
            common_root,
        )?
    };

    let mut process_start = child_of_common_root_before_offset(
        scope,
        snapshot.start_container,
        snapshot.start_offset,
        common_root,
    )?;
    if snapshot.start_container != common_root {
        process_start = process_start.and_then(|node| next_sibling_handle(scope, node));
    }
    let process_end = child_of_common_root_before_offset(
        scope,
        snapshot.end_container,
        snapshot.end_offset,
        common_root,
    )?;

    if action.mutates_range_contents() {
        collapse_range_after_contents_mutation(
            scope,
            range,
            common_root,
            partial_start,
            partial_end,
            &snapshot,
        )?;
    }

    if let (Some(fragment), Some(left_contents)) = (fragment, left_contents) {
        let _ = append_child_internal_handle(scope, fragment, left_contents);
    }

    if let Some(process_start) = process_start {
        let nodes = collect_following_siblings_until(scope, process_start, process_end);
        process_nodes(scope, action, nodes, common_root, fragment)?;
    }

    if let (Some(fragment), Some(right_contents)) = (fragment, right_contents) {
        let _ = append_child_internal_handle(scope, fragment, right_contents);
    }

    if let Some((container, offset)) = mutation_collapse_target {
        collapse_range_to_handle(scope, range, container, offset, Some(&snapshot))?;
    }

    finish_fragment(scope, range, fragment)
}

fn range_partially_contains_non_text_node(
    scope: &mut v8::PinScope<'_, '_>,
    snapshot: &RangeBoundarySnapshot<'_>,
) -> bool {
    range_surround_comparison_node(scope, snapshot.start_container)
        != range_surround_comparison_node(scope, snapshot.end_container)
}

fn range_surround_comparison_node(
    scope: &mut v8::PinScope<'_, '_>,
    container: DomHandle,
) -> Option<DomHandle> {
    match node_type_for_handle(scope, container) {
        Some(NodeType::Text | NodeType::CDataSection) => parent_handle(scope, container),
        Some(_) => Some(container),
        None => None,
    }
}

fn surround_new_parent_type_is_invalid<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    new_parent: v8::Local<'s, v8::Object>,
) -> bool {
    matches!(
        object_number_property(scope, new_parent, "nodeType").map(|value| value as u32),
        Some(2 | 9 | 10 | 11)
    )
}

fn remove_all_children(scope: &mut v8::PinScope<'_, '_>, parent: DomHandle) -> Option<()> {
    while let Some(child) = child_handle_at_offset_optional(scope, parent, 0)? {
        remove_child_internal_handle(scope, parent, child)?;
    }
    Some(())
}

fn process_contents_between_offsets(
    scope: &mut v8::PinScope<'_, '_>,
    action: RangeContentsAction,
    result_container: Option<DomHandle>,
    container: DomHandle,
    start_offset: u32,
    end_offset: u32,
) -> Option<Option<DomHandle>> {
    match node_type_for_handle(scope, container)? {
        NodeType::Text
        | NodeType::CDataSection
        | NodeType::ProcessingInstruction
        | NodeType::Comment => {
            let length = range_node_length_handle(scope, container)?;
            let start_offset = start_offset.min(length);
            let end_offset = end_offset.min(length);
            let mut result = None;
            if action.produces_fragment() {
                let cloned =
                    range_clone_character_data(scope, container, start_offset, end_offset)?;
                if let Some(result_container) = result_container {
                    let _ = append_child_internal_handle(scope, result_container, cloned);
                    result = Some(result_container);
                } else {
                    result = Some(cloned);
                }
            }
            if action.mutates_range_contents() {
                range_delete_character_data(scope, container, start_offset, end_offset)?;
            }
            Some(result)
        }
        _ => {
            let result_container = if action.produces_fragment() {
                Some(match result_container {
                    Some(result_container) => result_container,
                    None => clone_node_internal_handle(scope, container, false)?,
                })
            } else {
                None
            };
            let nodes = child_handles_between_offsets(scope, container, start_offset, end_offset)?;
            if action.produces_fragment() && contains_document_type(scope, &nodes) {
                throw_contents_hierarchy_request_error(scope);
                return None;
            }
            process_nodes(scope, action, nodes, container, result_container)?;
            Some(result_container)
        }
    }
}

fn process_nodes(
    scope: &mut v8::PinScope<'_, '_>,
    action: RangeContentsAction,
    nodes: Vec<DomHandle>,
    old_container: DomHandle,
    new_container: Option<DomHandle>,
) -> Option<()> {
    if action.produces_fragment() && contains_document_type(scope, &nodes) {
        throw_contents_hierarchy_request_error(scope);
        return None;
    }

    match action {
        RangeContentsAction::Clone => {
            let new_container = new_container?;
            for node in nodes {
                let clone = clone_node_internal_handle(scope, node, true)?;
                let _ = append_child_internal_handle(scope, new_container, clone);
            }
            Some(())
        }
        RangeContentsAction::Extract => {
            let new_container = new_container?;
            for node in nodes {
                let _ = append_child_internal_handle(scope, new_container, node);
            }
            Some(())
        }
        RangeContentsAction::Delete => {
            for node in nodes {
                let _ = remove_child_internal_handle(scope, old_container, node);
            }
            Some(())
        }
    }
}

#[derive(Clone, Copy)]
enum ContentsProcessDirection {
    Forward,
    Backward,
}

fn process_ancestors_and_their_siblings(
    scope: &mut v8::PinScope<'_, '_>,
    action: RangeContentsAction,
    container: DomHandle,
    direction: ContentsProcessDirection,
    mut cloned_container: Option<DomHandle>,
    common_root: DomHandle,
) -> Option<Option<DomHandle>> {
    let ancestors = ancestor_chain_below_common_root(scope, container, common_root)?;
    let mut cloned_ancestors = Vec::with_capacity(ancestors.len());
    if action.produces_fragment() {
        for ancestor in ancestors.iter().rev() {
            cloned_ancestors.push(clone_node_internal_handle(scope, *ancestor, false)?);
        }
        cloned_ancestors.reverse();
    }

    let mut first_sibling_to_process = match direction {
        ContentsProcessDirection::Forward => next_sibling_handle(scope, container),
        ContentsProcessDirection::Backward => previous_sibling_handle(scope, container),
    };

    for (index, ancestor) in ancestors.into_iter().enumerate() {
        if action.produces_fragment() {
            let cloned_ancestor = cloned_ancestors[index];
            let child = cloned_container?;
            let _ = append_child_internal_handle(scope, cloned_ancestor, child);
            cloned_container = Some(cloned_ancestor);
        }

        process_siblings(
            scope,
            action,
            cloned_container,
            ancestor,
            first_sibling_to_process,
            direction,
        )?;

        first_sibling_to_process = match direction {
            ContentsProcessDirection::Forward => next_sibling_handle(scope, ancestor),
            ContentsProcessDirection::Backward => previous_sibling_handle(scope, ancestor),
        };
    }

    Some(cloned_container)
}

fn process_siblings(
    scope: &mut v8::PinScope<'_, '_>,
    action: RangeContentsAction,
    container: Option<DomHandle>,
    old_container: DomHandle,
    first_sibling: Option<DomHandle>,
    direction: ContentsProcessDirection,
) -> Option<()> {
    let mut cursor = first_sibling;
    while let Some(sibling) = cursor {
        cursor = match direction {
            ContentsProcessDirection::Forward => next_sibling_handle(scope, sibling),
            ContentsProcessDirection::Backward => previous_sibling_handle(scope, sibling),
        };
        match action {
            RangeContentsAction::Clone => {
                let container = container?;
                let clone = clone_node_internal_handle(scope, sibling, true)?;
                match direction {
                    ContentsProcessDirection::Forward => {
                        let _ = append_child_internal_handle(scope, container, clone);
                    }
                    ContentsProcessDirection::Backward => {
                        let reference = child_handle_at_offset_optional(scope, container, 0)?;
                        let _ = insert_before_internal_handle(scope, container, clone, reference);
                    }
                }
            }
            RangeContentsAction::Extract => {
                let container = container?;
                match direction {
                    ContentsProcessDirection::Forward => {
                        let _ = append_child_internal_handle(scope, container, sibling);
                    }
                    ContentsProcessDirection::Backward => {
                        let reference = child_handle_at_offset_optional(scope, container, 0)?;
                        let _ = insert_before_internal_handle(scope, container, sibling, reference);
                    }
                }
            }
            RangeContentsAction::Delete => {
                if parent_handle(scope, sibling).is_some_and(|parent| parent == old_container) {
                    let _ = remove_child_internal_handle(scope, old_container, sibling);
                }
            }
        }
    }
    Some(())
}

fn contains_document_type(scope: &mut v8::PinScope<'_, '_>, nodes: &[DomHandle]) -> bool {
    for node in nodes {
        if node_type_for_handle(scope, *node) == Some(NodeType::DocumentType) {
            return true;
        }
    }
    false
}

fn throw_contents_hierarchy_request_error(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(scope, "HierarchyRequestError", 3, "Hierarchy Error");
}

fn throw_surround_invalid_state_error(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "The Range has partially selected a non-Text node.",
    );
}

fn throw_surround_invalid_node_type_error(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "InvalidNodeTypeError",
        24,
        "The node provided is not a valid Range.surroundContents parent.",
    );
}

fn ancestor_chain_below_common_root(
    scope: &mut v8::PinScope<'_, '_>,
    container: DomHandle,
    common_root: DomHandle,
) -> Option<Vec<DomHandle>> {
    let mut ancestors = Vec::new();
    let mut current = container;
    while let Some(parent) = parent_handle(scope, current) {
        if parent == common_root {
            break;
        }
        ancestors.push(parent);
        current = parent;
    }
    Some(ancestors)
}

fn child_of_common_root_before_offset(
    scope: &mut v8::PinScope<'_, '_>,
    container: DomHandle,
    offset: u32,
    common_root: DomHandle,
) -> Option<Option<DomHandle>> {
    if container == common_root {
        return child_handle_at_offset_optional(scope, common_root, offset);
    }

    let mut child = container;
    while let Some(parent) = parent_handle(scope, child) {
        if parent == common_root {
            return Some(Some(child));
        }
        child = parent;
    }
    None
}

fn collect_following_siblings_until(
    scope: &mut v8::PinScope<'_, '_>,
    start: DomHandle,
    stop_before: Option<DomHandle>,
) -> Vec<DomHandle> {
    let mut nodes = Vec::new();
    let mut cursor = Some(start);
    while let Some(node) = cursor {
        if stop_before.is_some_and(|stop| node == stop) {
            break;
        }
        cursor = next_sibling_handle(scope, node);
        nodes.push(node);
    }
    nodes
}

fn highest_ancestor_under_common_root(
    scope: &mut v8::PinScope<'_, '_>,
    node: DomHandle,
    common_root: DomHandle,
) -> Option<DomHandle> {
    if node == common_root {
        return None;
    }

    let mut current = node;
    while let Some(parent) = parent_handle(scope, current) {
        if parent == common_root {
            return Some(current);
        }
        current = parent;
    }
    None
}

fn common_ancestor_container_handle(
    scope: &mut v8::PinScope<'_, '_>,
    start_container: DomHandle,
    end_container: DomHandle,
) -> Option<DomHandle> {
    let mut start_ancestors = Vec::new();
    let mut current = Some(start_container);
    while let Some(node) = current {
        start_ancestors.push(node);
        current = parent_handle(scope, node);
    }

    let mut current = Some(end_container);
    while let Some(node) = current {
        if start_ancestors.contains(&node) {
            return Some(node);
        }
        current = parent_handle(scope, node);
    }
    None
}

fn collapse_range_after_contents_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    common_root: DomHandle,
    partial_start: Option<DomHandle>,
    partial_end: Option<DomHandle>,
    snapshot: &RangeBoundarySnapshot<'s>,
) -> Option<()> {
    if let Some(partial_start) = partial_start
        && node_is_still_below_common_root(scope, partial_start, common_root)
        && let Some(parent) = parent_handle(scope, partial_start)
        && let Some(index) = child_index_handle(scope, parent, partial_start)
    {
        collapse_range_to_handle(
            scope,
            range,
            parent,
            index.saturating_add(1),
            Some(snapshot),
        )?;
        return Some(());
    }

    if let Some(partial_end) = partial_end
        && node_is_still_below_common_root(scope, partial_end, common_root)
        && let Some(parent) = parent_handle(scope, partial_end)
        && let Some(index) = child_index_handle(scope, parent, partial_end)
    {
        collapse_range_to_handle(scope, range, parent, index, Some(snapshot))?;
    }
    Some(())
}

fn node_is_still_below_common_root(
    scope: &mut v8::PinScope<'_, '_>,
    node: DomHandle,
    common_root: DomHandle,
) -> bool {
    node == common_root || node_contains_handle(scope, common_root, node)
}

fn contents_mutation_collapse_target(
    scope: &mut v8::PinScope<'_, '_>,
    start_container: DomHandle,
    start_offset: u32,
    end_container: DomHandle,
) -> Option<(DomHandle, u32)> {
    if node_is_ancestor_container(scope, start_container, end_container) {
        return Some((start_container, start_offset));
    }

    let mut reference = start_container;
    while let Some(parent) = parent_handle(scope, reference) {
        if node_is_ancestor_container(scope, parent, end_container) {
            let offset = child_index_handle(scope, parent, reference)?.saturating_add(1);
            return Some((parent, offset));
        }
        reference = parent;
    }
    None
}

fn node_is_ancestor_container(
    scope: &mut v8::PinScope<'_, '_>,
    ancestor: DomHandle,
    node: DomHandle,
) -> bool {
    let mut current = Some(node);
    while let Some(node) = current {
        if ancestor == node {
            return true;
        }
        current = parent_handle(scope, node);
    }
    false
}

fn collapse_range_to_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    container: DomHandle,
    offset: u32,
    snapshot: Option<&RangeBoundarySnapshot<'s>>,
) -> Option<()> {
    let container = boundary_wrapper_for_handle(scope, container, snapshot)?;
    set_range_boundary(scope, range, RangeBoundarySide::Start, container, offset);
    set_range_boundary(scope, range, RangeBoundarySide::End, container, offset);
    Some(())
}

fn boundary_wrapper_for_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: DomHandle,
    snapshot: Option<&RangeBoundarySnapshot<'s>>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Some(snapshot) = snapshot {
        if handle == snapshot.start_container
            && let Some(wrapper) = snapshot.start_container_wrapper
        {
            return Some(wrapper);
        }
        if handle == snapshot.end_container
            && let Some(wrapper) = snapshot.end_container_wrapper
        {
            return Some(wrapper);
        }
    }
    node_wrapper_for_handle_prefer_paired(scope, handle)
}

fn range_delete_character_data(
    scope: &mut v8::PinScope<'_, '_>,
    node: DomHandle,
    start_offset: u32,
    end_offset: u32,
) -> Option<()> {
    if end_offset <= start_offset {
        return Some(());
    }
    let mut units = character_data_utf16_units_handle(scope, node)?;
    let data_len = units.len() as u32;
    let start_offset = start_offset.min(data_len);
    let end_offset = end_offset.min(data_len);
    let removed_count = end_offset.saturating_sub(start_offset);
    if removed_count == 0 {
        return Some(());
    }
    let start = start_offset as usize;
    let end = start + removed_count as usize;
    units.drain(start..end);
    let changed = set_character_data_utf16_units_handle(scope, node, &units)?;
    if changed {
        crate::context_bootstrap::live_ranges_character_data_edit(
            scope,
            node,
            start_offset,
            removed_count,
            0,
        );
    }
    changed.then_some(())
}

fn range_clone_character_data(
    scope: &mut v8::PinScope<'_, '_>,
    node: DomHandle,
    start_offset: u32,
    end_offset: u32,
) -> Option<DomHandle> {
    let units = character_data_utf16_units_handle(scope, node)?;
    let start = (start_offset as usize).min(units.len());
    let end = (end_offset as usize).min(units.len()).max(start);
    let clone = clone_node_internal_handle(scope, node, false)?;
    let _ = set_character_data_utf16_units_handle(scope, clone, &units[start..end])?;
    Some(clone)
}

fn finish_fragment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    fragment: Option<DomHandle>,
) -> Option<Option<v8::Local<'s, v8::Object>>> {
    match fragment {
        Some(fragment) => {
            let context = range.get_creation_context(scope)?;
            let scope = &mut v8::ContextScope::new(scope, context);
            Some(Some(node_wrapper_for_handle(scope, fragment)?))
        }
        None => Some(None),
    }
}
