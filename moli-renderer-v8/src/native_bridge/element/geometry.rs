mod layout;
mod metrics;
mod provider;
mod rects;

pub use layout::ClientRect;
pub(crate) use layout::{
    compute_mock_client_rect, compute_mock_intersection_client_rect,
    compute_mock_intersection_scrollport_client_rect,
};
pub(crate) use metrics::{
    apply_scroll_observable_effects, perform_wheel_scroll_default_action,
    queue_scroll_observable_effects, scroll_node_into_view_at_start,
    scroll_node_into_view_if_needed,
};
pub(in crate::native_bridge) use metrics::{
    node_client_height_getter_function, node_client_left_getter_function,
    node_client_top_getter_function, node_client_width_getter_function,
    node_offset_height_getter_function, node_offset_left_getter_function,
    node_offset_parent_getter_function, node_offset_top_getter_function,
    node_offset_width_getter_function, node_scroll_by_callback, node_scroll_height_getter_function,
    node_scroll_into_view_callback, node_scroll_into_view_if_needed_callback,
    node_scroll_left_getter_function, node_scroll_left_setter_function, node_scroll_to_callback,
    node_scroll_top_getter_function, node_scroll_top_setter_function,
    node_scroll_width_getter_function,
};
pub(crate) use provider::{
    observable_bounding_client_rect, observable_bounding_client_rects, observable_caret_position,
    observable_client_rects, observable_deep_hit_test, observable_element_metrics,
    observable_event_offset, observable_geometry_batch, observable_hit_test_all,
    observable_input_hit_test, observable_scroll_adjusted_client_rect,
    observable_scroll_into_view_geometry, observable_sources_with_fragments,
};
pub(in crate::native_bridge) use rects::{
    node_get_bounding_client_rect_callback, node_get_client_rects_callback,
};
