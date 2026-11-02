//! Native inspection capabilities used by owner-thread embedder tasks.

use super::{focus, rendered_state, styles};
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;

pub(crate) fn is_disabled_form_control(runtime: &JsContextHost, handle: DomHandle) -> bool {
    focus::is_disabled_form_control(runtime, handle)
}

pub(crate) fn is_focusable(runtime: &JsContextHost, handle: DomHandle) -> bool {
    focus::is_focusable(runtime, handle)
}

pub(crate) fn style_property(read: &styles::ComputedStyleRead<'_>, name: &str) -> String {
    read.property(name)
}

pub(crate) fn node_has_screenshot_layout(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let mut scope = styles::StyleObservation::new(runtime);
    rendered_state::ElementRenderedState::read_in_scope(&mut scope, handle).box_state()
        == rendered_state::ElementBoxState::Box
}
