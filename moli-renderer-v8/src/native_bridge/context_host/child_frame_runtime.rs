mod cross_origin_property_descriptor_map;
mod cross_origin_window_intrinsics;
mod document;
pub(in crate::native_bridge::context_host) mod document_slots;
mod isolated_world;
mod realm_state;
mod sync;
mod window;

pub(super) use crate::context_bootstrap::WINDOW_EVENT_HANDLER_PROPERTIES;
pub(crate) use cross_origin_window_intrinsics::install_cross_origin_window_internal_method_intrinsics;
pub(in crate::native_bridge::context_host) use window::ChildWindowProxyRecords;
pub(crate) use window::{
    cross_origin_remote_frame_window_target, cross_origin_remote_top_window_target,
    cross_origin_window_target_host_ptr, install_child_window_proxy_access_check_handlers,
    is_cross_origin_location_proxy, is_cross_origin_related_top_window_proxy,
    is_cross_origin_remote_frame_window_proxy, is_cross_origin_top_window_proxy,
    throw_cross_origin_location_security_error, throw_cross_origin_type_error,
    top_level_window_proxy_is_finally_closed,
};
