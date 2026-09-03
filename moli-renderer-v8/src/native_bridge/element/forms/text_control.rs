use super::*;

mod events;
mod selection;
mod value;

pub(crate) use events::{
    dispatch_text_control_event, queue_text_control_document_selection_change_event,
};
pub(crate) use selection::{
    replace_text_control_selection, text_control_set_selection_range_internal,
    text_control_set_selection_range_with_direction_internal,
};
pub(in crate::native_bridge) use selection::{
    text_control_select_callback, text_control_selection_direction_getter_function,
    text_control_selection_direction_setter_function, text_control_selection_end_getter_function,
    text_control_selection_end_setter_function, text_control_selection_start_getter_function,
    text_control_selection_start_setter_function, text_control_set_range_text_callback,
    text_control_set_selection_range_callback,
};
pub(in crate::native_bridge) use value::normalize_textarea_api_value;
pub(crate) use value::{is_text_control, text_control_value};
pub(in crate::native_bridge) use value::{
    textarea_value_getter_function, textarea_value_setter_function,
};
