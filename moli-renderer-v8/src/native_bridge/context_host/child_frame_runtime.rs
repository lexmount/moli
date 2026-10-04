mod document;
pub(in crate::native_bridge::context_host) mod document_slots;
mod isolated_world;
mod realm_state;
mod sync;
mod window;

pub(super) use crate::context_bootstrap::WINDOW_EVENT_HANDLER_PROPERTIES;
pub(in crate::native_bridge::context_host) use window::ChildWindowProxyRecords;
pub(crate) use window::{
    cross_origin_lightweight_popup_id, install_child_window_proxy_access_check_handlers,
    is_cross_origin_location_proxy, is_cross_origin_top_window_proxy,
    throw_cross_origin_location_security_error, throw_cross_origin_type_error,
};
