use moli_layout::{LayoutError, LayoutPoint};

use super::{InputDocument, InputPosition, current_selection_state};
use crate::{
    document_runtime::DomHandle,
    dom::{forms::InputType, native::Node},
    native_bridge::{
        PageScriptHost,
        element::{
            is_disabled_form_control, text_control_set_selection_range_with_direction_internal,
            text_control_value,
        },
    },
};

#[derive(Clone, Copy)]
pub(in crate::script_vm) struct TextControlMouseSelection {
    control: DomHandle,
    document: InputDocument,
    anchor: u32,
    press_point: LayoutPoint,
    collapse_on_release: bool,
}

impl TextControlMouseSelection {
    pub(super) fn begin(
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut PageScriptHost,
        control: DomHandle,
        position: InputPosition,
        extend: bool,
    ) -> Result<Option<Self>, LayoutError> {
        let runtime = unsafe { &*host };
        if !is_active_text_editor(runtime, control) {
            return Ok(None);
        }
        let Some(document) = InputDocument::for_target(runtime, control) else {
            return Ok(None);
        };
        let Some(offset) = caret_offset(runtime, control, document, position)? else {
            return Ok(None);
        };
        let (start, end, direction) = current_selection_state(runtime, control);
        let anchor_at_end = if cfg!(target_os = "macos") || direction == "none" {
            offset.saturating_sub(start) < end.saturating_sub(offset)
        } else {
            direction == "backward"
        };
        let anchor = if !extend {
            offset
        } else if anchor_at_end {
            end
        } else {
            start
        };
        // A press on an existing selection keeps the range until the gesture
        // becomes a click or starts extending the selection.
        let collapse_on_release = !extend && start < end && (start..=end).contains(&offset);
        if !collapse_on_release {
            set_user_selection(scope, host, control, anchor, offset);
        }
        Ok(Some(Self {
            control,
            document,
            anchor,
            press_point: position.layout_point(),
            collapse_on_release,
        }))
    }

    pub(super) fn extend(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut PageScriptHost,
        position: InputPosition,
    ) -> Result<bool, LayoutError> {
        let runtime = unsafe { &*host };
        if !self.is_active(runtime) {
            return Ok(false);
        }
        let Some(focus) = caret_offset(runtime, self.control, self.document, position)? else {
            return Ok(false);
        };
        if !self.collapse_on_release || focus != self.anchor {
            self.collapse_on_release = false;
            set_user_selection(scope, host, self.control, self.anchor, focus);
        }
        Ok(true)
    }

    pub(super) fn release(
        self,
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut PageScriptHost,
        position: InputPosition,
    ) {
        let point = position.layout_point();
        if self.collapse_on_release
            && point.x.floor() == self.press_point.x.floor()
            && point.y.floor() == self.press_point.y.floor()
            && self.is_active(unsafe { &*host })
        {
            set_user_selection(scope, host, self.control, self.anchor, self.anchor);
        }
    }

    fn is_active(self, host: &PageScriptHost) -> bool {
        self.document.contains(host, self.control) && is_active_text_editor(host, self.control)
    }
}

fn is_active_text_editor(host: &PageScriptHost, control: DomHandle) -> bool {
    host.active_element_handle() == Some(control)
        && host
            .dom_host()
            .node(control)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element.is_html_textarea()
                    || (element.is_html_input()
                        && (element.input_type().supports_variable_length_selection()
                            || matches!(
                                element.input_type(),
                                InputType::Email | InputType::Number
                            )))
            })
        && !is_disabled_form_control(host, control)
}

fn caret_offset(
    host: &PageScriptHost,
    control: DomHandle,
    document: InputDocument,
    position: InputPosition,
) -> Result<Option<u32>, LayoutError> {
    if !host.layout_policy().uses_real_layout() {
        return Ok(None);
    }
    if text_control_value(host, control).is_empty() {
        return Ok(Some(0));
    }
    host.ensure_observable_layout()?;
    let Some(transform) = host.input_frame_transform(document.handle, None) else {
        return Ok(None);
    };
    let point = transform.map_point(position.layout_point());
    Ok(host
        .with_latest_layout_tree_for_document(document.handle, |tree| {
            tree.text_control_caret_offset(control, point)
        })
        .flatten()
        .map(|offset| offset as u32))
}

fn set_user_selection(
    scope: &mut v8::PinScope<'_, '_>,
    host: *mut PageScriptHost,
    control: DomHandle,
    anchor: u32,
    focus: u32,
) {
    let direction = if cfg!(target_os = "macos") || focus == anchor {
        "none"
    } else if focus < anchor {
        "backward"
    } else {
        "forward"
    };
    text_control_set_selection_range_with_direction_internal(
        scope,
        host,
        control,
        anchor.min(focus),
        anchor.max(focus),
        direction,
    );
}
