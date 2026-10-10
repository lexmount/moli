use super::shared::*;
use crate::{
    native_bridge::JsContextHost,
    util::{
        context_host_ptr_from_context_slot, context_host_ptr_from_global_bridge,
        context_host_ptr_from_window_object,
    },
};

mod child_context;
mod cross_origin_cache;
mod document;
mod helpers;
mod interceptors;
mod surface;

use super::CHILD_BROWSING_CONTEXT_HANDLE_SLOT as WINDOW_CHILD_CONTEXT_HANDLE_SLOT;

const WINDOW_FRAME_ELEMENT_SLOT: &str = "__moliWindowFrameElement";

pub(crate) use child_context::window_length_getter;
pub(super) use child_context::{
    window_credentialless_getter, window_cross_origin_isolated_getter, window_frame_element_getter,
};
pub(crate) use cross_origin_cache::cached_cross_origin_window_surface;
pub(super) use cross_origin_cache::initialize_cross_origin_window_cache;
pub(super) use document::{WINDOW_DOCUMENT_SLOT, window_document_getter_template};
pub(crate) use document::{
    bind_current_child_window_document, retain_window_document_in_retired_realm,
};
pub(super) use helpers::window_child_context_handle;
pub(crate) use helpers::{current_window_style_viewport, window_host_ptr};
pub(super) use interceptors::{
    window_indexed_property_definer, window_indexed_property_deleter,
    window_indexed_property_descriptor, window_indexed_property_enumerator,
    window_indexed_property_getter, window_indexed_property_query, window_indexed_property_setter,
    window_named_property_getter, window_named_property_query,
};
pub(crate) use surface::{
    window_closed_getter, window_opener_getter, window_parent_getter, window_top_getter,
};
pub(super) use surface::{
    window_custom_elements_getter, window_device_pixel_ratio_getter, window_frames_getter,
    window_inner_height_getter, window_inner_surface_height, window_inner_surface_width,
    window_inner_width_getter, window_navigator_getter, window_opener_setter,
    window_outer_height_getter, window_outer_width_getter, window_performance_getter,
    window_screen_getter, window_scroll_x_getter, window_scroll_y_getter, window_self_getter,
    window_speech_synthesis_getter, window_visual_viewport_getter, window_window_getter,
};
