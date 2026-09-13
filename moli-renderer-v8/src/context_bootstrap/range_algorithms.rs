use super::super::util::node_wrapper_from_handle;
use super::range::{
    RangeBoundarySide, child_index, native_range_boundary_handles, native_range_boundary_point,
    range_boundary_container_object, range_boundary_offset, range_is_collapsed, set_range_boundary,
};
use super::*;
use crate::document_runtime::DomHandle;
use crate::dom::native::NodeType;
use crate::native_bridge::element::{
    ClientRect, compute_mock_client_rect, observable_geometry_batch,
};

mod contents;
mod geometry;
mod mutation;
mod ordering;
mod text;
mod tree_ops;

pub(super) use contents::{
    range_clone_contents, range_delete_contents, range_extract_contents, range_surround_contents,
};
pub(super) use geometry::{
    new_dom_rect_zero, range_geometry_client_rects, range_geometry_dom_rect,
};
pub(super) use mutation::range_insert_node_at_boundary;
pub(super) use ordering::{
    native_boundary_point_from_node, native_boundary_point_from_range_boundary,
    native_boundary_point_is_doctype, native_boundary_point_is_valid, native_boundary_point_order,
    native_boundary_points_share_root, point_order, range_common_ancestor_container,
    range_compare_point_internal, range_intersects_node_native, root_handle,
};
pub(super) use text::{
    range_clipboard_string_contents, range_selection_string_contents, range_string_contents,
};
pub(super) use tree_ops::create_contextual_fragment_internal;

pub(in crate::context_bootstrap::range_algorithms) use tree_ops::{
    append_child_internal_handle, character_data_utf16_units_handle,
    child_handle_at_offset_optional, child_handles_between_offsets, child_index_handle,
    clone_node_internal_handle, create_document_fragment_handle,
    document_handle_for_node_handle_or_self, insert_before_internal_handle, next_sibling_handle,
    node_contains_handle, node_handle_for_range_insert, node_handle_for_tree_op,
    node_type_for_handle, node_wrapper_for_handle, node_wrapper_for_handle_prefer_paired,
    parent_handle, previous_sibling_handle, prospective_child_index_after_removal_handle,
    range_insert_move_internal_handle, range_inserted_node_length_handle, range_node_length_handle,
    range_slice_utf16_string, remove_child_internal_handle, set_character_data_utf16_units_handle,
    split_text_internal_handle, validate_pre_insert_internal_handle,
};

pub(in crate::context_bootstrap::range_algorithms) use ordering::common_ancestor_handle;

pub(in crate::context_bootstrap::range_algorithms) use mutation::{
    range_insert_node_handle_at_boundary, range_select_node_handle,
};
