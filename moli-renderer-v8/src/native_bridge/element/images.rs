mod attributes;
mod decode;
mod dimensions;
mod geometry;
mod lazy;
mod src;
mod state;

pub(in crate::native_bridge) use attributes::{
    image_is_map_getter_function, image_is_map_setter_function,
};
pub(in crate::native_bridge) use decode::image_decode_callback;
pub(crate) use dimensions::image_intrinsic_dimensions;
pub(in crate::native_bridge) use dimensions::{
    image_height_getter_function, image_height_setter_function,
    image_natural_height_getter_function, image_natural_width_getter_function,
    image_width_getter_function, image_width_setter_function,
};
pub(in crate::native_bridge) use geometry::{image_x_getter_function, image_y_getter_function};
pub(in crate::native_bridge) use src::image_current_src_getter_function;
pub(crate) use src::{
    apply_authorized_image_load_event_in_context, apply_image_attribute_mutation_plan,
    image_selected_request_key, image_selected_source, plan_image_attribute_mutation,
    queue_image_load_event_after_document_adoption, queue_image_load_event_for_loading_change,
    queue_image_load_event_if_needed, queue_image_load_event_if_needed_with_initiator,
    queue_image_load_network_terminal_followup, queue_revealed_lazy_image_loads,
    reset_image_load_dispatch,
};
pub(in crate::native_bridge) use state::image_complete_getter_function;
