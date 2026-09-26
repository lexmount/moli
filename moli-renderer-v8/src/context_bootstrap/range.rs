use super::*;
pub(super) use crate::native_bridge::RangeBoundarySide;

mod arguments;
mod exceptions;
mod object;
mod static_storage;
mod validation;

pub(super) use arguments::webidl_node_arg;
pub(super) use exceptions::throw_named_dom_exception;
pub(super) use object::{
    RANGE_WRAPPER_INTERNAL_FIELD_COUNT, current_document_object, initialize_range_object,
    native_range_boundary_handles, native_range_boundary_point, new_range_for_document,
    range_boundary_container_object, range_boundary_offset, range_is_collapsed,
    range_native_record_handle, set_range_boundary,
};
pub(super) use static_storage::initialize_static_range_object;
pub(super) use validation::{child_index, range_node_length, range_validate_boundary_point};
