mod helpers;
mod iframe;

pub(super) use self::helpers::{
    parsed_url_like_attribute, resolve_url_like_attribute, set_resolved_url_attribute,
    should_block_dangling_markup_subresource,
};
pub(in crate::native_bridge) use self::iframe::update_iframe_snapshot_navigation;
pub(super) use self::iframe::{
    disconnected_iframe_can_materialize_detached_content, iframe_has_inactive_child_context,
    iframe_is_in_own_child_document, iframe_is_inside_its_own_child_context_document,
    iframe_uses_detached_content_cache,
};
