//! Native inspection capabilities used by owner-thread embedder tasks.

use super::{focus, styles};
use crate::document_runtime::DomHandle;
use crate::native_bridge::PageScriptHost;

pub(crate) fn is_disabled_form_control(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    focus::is_disabled_form_control(runtime, handle)
}

pub(crate) fn is_focusable(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    focus::is_focusable(runtime, handle)
}

pub(crate) fn style_property(read: &styles::ComputedStyleRead<'_>, name: &str) -> String {
    read.property(name)
}

pub(crate) fn node_has_screenshot_layout(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    super::element_is_being_rendered(runtime, handle)
}
