use super::shared::append_console_message;
use super::*;
use crate::{document_runtime::EventTargetHandle, native_bridge::JsContextHost};

mod accessors;
mod console;
mod error;
mod install;
mod promise;

pub(crate) const WINDOW_EVENT_HANDLER_PROPERTIES: &[&str] = &[
    "onabort",
    "onafterprint",
    "onbeforeprint",
    "onbeforeunload",
    "onblur",
    "oncancel",
    "oncanplay",
    "oncanplaythrough",
    "onchange",
    "onclick",
    "onclose",
    "oncontextmenu",
    "ondblclick",
    "ondrag",
    "ondragend",
    "ondragenter",
    "ondragleave",
    "ondragover",
    "ondragstart",
    "ondrop",
    "ondurationchange",
    "onemptied",
    "onended",
    "onerror",
    "onfocus",
    "onhashchange",
    "oninput",
    "oninvalid",
    "onkeydown",
    "onkeypress",
    "onkeyup",
    "onload",
    "onloadeddata",
    "onloadedmetadata",
    "onloadstart",
    "onmessage",
    "onmousedown",
    "onmousemove",
    "onmouseout",
    "onmouseover",
    "onmouseup",
    "onmousewheel",
    "onoffline",
    "ononline",
    "onpagehide",
    "onpageshow",
    "onpause",
    "onplay",
    "onplaying",
    "onpopstate",
    "onprogress",
    "onratechange",
    "onreset",
    "onresize",
    "onscroll",
    "onseeked",
    "onseeking",
    "onselect",
    "onstalled",
    "onstorage",
    "onsubmit",
    "onsuspend",
    "ontimeupdate",
    "ontouchcancel",
    "ontouchend",
    "ontouchmove",
    "ontouchstart",
    "onunhandledrejection",
    "onunload",
    "onvolumechange",
    "onwaiting",
    "onrejectionhandled",
];

pub(crate) use accessors::{
    set_window_body_onerror_handler_compiled, set_window_onerror_handler_value,
    window_body_onerror_handler_is_compiled,
};
pub(super) use accessors::{
    window_console_getter, window_event_getter, window_event_setter,
    window_onerror_getter_function, window_onerror_setter_function,
    window_onrejectionhandled_getter_function, window_onrejectionhandled_setter_function,
    window_onunhandledrejection_getter_function, window_onunhandledrejection_setter_function,
};
pub(super) use console::{
    console_assert_callback, console_debug_callback, console_error_callback,
    console_group_callback, console_group_collapsed_callback, console_info_callback,
    console_log_callback, console_noop_callback, console_profile_callback,
    console_profile_end_callback, console_table_callback, console_trace_callback,
    console_warn_callback,
};
pub(super) use error::window_report_error_callback;
pub(crate) use error::{
    dispatch_window_error_event_with_details, dispatch_window_report_error_message,
};
pub(super) use install::install_window_global_accessors;
pub(crate) use promise::dispatch_window_promise_rejection_event;
