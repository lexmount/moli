mod client_rect;
mod hit_test;
mod metrics;
mod mock;
mod provider;
mod rects;
mod scroll;
mod scroll_into_view;

pub use client_rect::ClientRect;
pub(crate) use hit_test::{
    observable_deep_hit_test, observable_input_hit_test, observable_scrollbar_hit_test,
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
pub(crate) use mock::{
    compute_mock_client_rect, compute_mock_intersection_client_rect,
    compute_mock_intersection_scrollport_client_rect,
};
pub(crate) use provider::{
    observable_bounding_client_rect, observable_bounding_client_rects, observable_caret_position,
    observable_client_rects, observable_element_metrics, observable_event_offset,
    observable_geometry_batch, observable_hit_test_all, observable_scroll_adjusted_client_rect,
    observable_sources_with_fragments,
};
pub(in crate::native_bridge) use rects::{
    node_get_bounding_client_rect_callback, node_get_client_rects_callback,
};
pub(crate) use scroll::{
    apply_scroll_observable_effects, perform_scrollbar_scroll_default_action,
    perform_wheel_scroll_default_action, queue_scroll_observable_effects,
};
pub(crate) use scroll_into_view::{
    scroll_node_into_view_at_start, scroll_node_into_view_if_needed,
};
