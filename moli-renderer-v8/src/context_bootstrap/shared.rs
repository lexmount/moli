mod buffers;
mod clone;
mod console;
mod definition;
mod dom;
mod fonts;
mod object;
mod slots;
mod time;

use crate::{
    native_bridge::callback_value_dom_handle,
    util::{context_host_ptr_from_global_bridge, object_number_property, v8_string},
};

pub(in crate::context_bootstrap) use crate::host::WINDOW_EVENT_SLOT;
pub(in crate::context_bootstrap) use buffers::*;
pub(crate) use clone::*;
pub(in crate::context_bootstrap) use console::*;
pub(crate) use console::{
    console_arg_remote_object_json, current_console_stack,
    install_console_message_buffers_for_context,
    snapshot_console_message_details_for_current_context,
    snapshot_console_messages_for_current_context,
};
pub(in crate::context_bootstrap) use definition::*;
pub(in crate::context_bootstrap) use dom::*;
pub(in crate::context_bootstrap) use fonts::with_font_services;
pub(in crate::context_bootstrap) use moli_browser_profile::{
    DEFAULT_CONNECTION_DOWNLINK, DEFAULT_CONNECTION_DOWNLINK_MAX,
    DEFAULT_CONNECTION_EFFECTIVE_TYPE, DEFAULT_CONNECTION_RTT, DEFAULT_CONNECTION_SAVE_DATA,
    DEFAULT_CONNECTION_TYPE, DEFAULT_NAVIGATOR_APP_CODE_NAME, DEFAULT_NAVIGATOR_APP_NAME,
    DEFAULT_NAVIGATOR_DEVICE_MEMORY, DEFAULT_NAVIGATOR_ONLINE,
    DEFAULT_NAVIGATOR_PDF_VIEWER_ENABLED, DEFAULT_NAVIGATOR_PRODUCT, DEFAULT_NAVIGATOR_PRODUCT_SUB,
    DEFAULT_NAVIGATOR_VENDOR, DEFAULT_NAVIGATOR_VENDOR_SUB, DEFAULT_NAVIGATOR_WEBDRIVER,
    DEFAULT_WINDOW_SURFACE_PROFILE,
};
pub(in crate::context_bootstrap) use object::*;
pub(crate) use slots::*;
pub(crate) use time::*;

pub(in crate::context_bootstrap) use super::shared_installers::{
    install_abort_template_bindings, install_attr_template_bindings,
    install_constructor_constant_template_bindings,
    install_css_style_declaration_template_accessors, install_node_filter_constants,
    install_to_string_tag,
};
