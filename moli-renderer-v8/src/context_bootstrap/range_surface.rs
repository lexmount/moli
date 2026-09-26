use super::range::{
    RANGE_WRAPPER_INTERNAL_FIELD_COUNT, RangeBoundarySide, child_index, current_document_object,
    initialize_range_object, initialize_static_range_object, new_range_for_document,
    range_boundary_container_object, range_boundary_offset, range_is_collapsed, range_node_length,
    range_validate_boundary_point, set_range_boundary, throw_named_dom_exception, webidl_node_arg,
};
use super::range_algorithms::{
    create_contextual_fragment_internal, native_boundary_point_from_node,
    native_boundary_point_from_range_boundary, native_boundary_point_is_doctype,
    native_boundary_point_is_valid, native_boundary_point_order, native_boundary_points_share_root,
    new_dom_rect_zero, point_order, range_clone_contents, range_common_ancestor_container,
    range_compare_point_internal, range_delete_contents, range_extract_contents,
    range_geometry_client_rects, range_geometry_dom_rect, range_insert_node_at_boundary,
    range_intersects_node_native, range_string_contents, range_surround_contents, root_handle,
};
use super::*;

mod accessors;
mod boundaries;
mod comparison;
mod construction;
mod content;
mod geometry;
mod install;
mod template;

pub(super) use install::install_range_template_bindings;
pub(super) use template::{
    build_abstract_range_template, build_range_constructor_template,
    build_static_range_constructor_template,
};
