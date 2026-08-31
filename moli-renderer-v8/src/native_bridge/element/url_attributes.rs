mod helpers;
mod iframe;

pub(super) use self::helpers::{
    default_port_for_scheme, normalize_url_default_port, parsed_url_like_attribute,
    resolve_url_like_attribute, set_resolved_url_attribute,
    should_block_dangling_markup_subresource,
};
pub(super) use self::iframe::{
    disconnected_iframe_can_materialize_detached_content, iframe_has_inactive_child_context,
    iframe_is_in_own_child_document, iframe_is_inside_its_own_child_context_document,
    iframe_uses_detached_content_cache,
};
pub(in crate::native_bridge) use self::iframe::{
    live_frame_owner_content_window_for_handle, update_iframe_snapshot_navigation,
};
