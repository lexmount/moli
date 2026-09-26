use super::*;

mod composed;
mod dispatch;
mod editing_caret;
mod focus;
mod text;
mod values;

pub(super) use composed::selection_composed_boundary_order;
pub(super) use dispatch::{boundary_order, selection_dispatch_change};
pub(crate) use focus::focus_element_selection;
pub(super) use text::{first_text_descendant, last_text_descendant, text_length};
pub(super) use values::{
    SelectionRangeUpdateState, new_selection_runtime_object, selection_anchor_node,
    selection_anchor_offset, selection_bind_owner_document, selection_clear,
    selection_composed_end_node, selection_composed_end_offset, selection_composed_start_node,
    selection_composed_start_offset, selection_direction, selection_focus_node,
    selection_focus_offset, selection_has_range, selection_is_collapsed_internal,
    selection_owner_document, selection_range, selection_range_update_state,
    selection_set_collapsed, selection_store, selection_store_with_composed_boundaries,
    selection_sync_associated_range,
};
