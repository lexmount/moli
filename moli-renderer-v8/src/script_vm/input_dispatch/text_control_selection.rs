use moli_input::InputModifiers;
use moli_layout::{LayoutError, LayoutPoint, LayoutTextControlCaret};
use unicode_segmentation::UnicodeSegmentation;

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
    anchor: (u32, u32),
    granularity: SelectionGranularity,
    press_point: LayoutPoint,
    collapse_on_release: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectionGranularity {
    Character,
    Word,
    Paragraph,
}

impl SelectionGranularity {
    fn at_point(
        self,
        host: &PageScriptHost,
        control: DomHandle,
        caret: LayoutTextControlCaret,
    ) -> (u32, u32) {
        if self == Self::Character {
            return (caret.offset as u32, caret.offset as u32);
        }
        let value = text_control_value(host, control);
        let offset = caret.source_start as u32;
        match self {
            Self::Word => text_segment_at(value.split_word_bounds(), offset),
            Self::Paragraph => text_segment_at(value.split_inclusive('\n'), offset),
            Self::Character => unreachable!("character selection is a caret"),
        }
    }
}

fn text_segment_at<'a>(segments: impl Iterator<Item = &'a str>, offset: u32) -> (u32, u32) {
    let mut start = 0;
    for segment in segments {
        let end = start + segment.encode_utf16().count() as u32;
        if offset < end {
            return (start, end);
        }
        start = end;
    }
    (start, start)
}

impl TextControlMouseSelection {
    pub(super) fn begin(
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut PageScriptHost,
        control: DomHandle,
        position: InputPosition,
        click_count: i32,
        modifiers: InputModifiers,
    ) -> Result<Option<Self>, LayoutError> {
        let runtime = unsafe { &*host };
        if !is_active_text_editor(runtime, control) {
            return Ok(None);
        }
        let Some(document) = InputDocument::for_target(runtime, control) else {
            return Ok(None);
        };
        let Some(caret) = caret_hit(runtime, control, document, position)? else {
            return Ok(None);
        };
        let (start, end, direction) = current_selection_state(runtime, control);
        let password = runtime
            .dom_host()
            .node(control)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element.is_html_input() && element.input_type() == InputType::Password
            });
        let granularity = match click_count {
            2 if !password => SelectionGranularity::Word,
            2.. => SelectionGranularity::Paragraph,
            _ => SelectionGranularity::Character,
        };
        let offset = caret.offset as u32;
        let extend = granularity == SelectionGranularity::Character
            && modifiers.contains(InputModifiers::SHIFT);
        let anchor_at_end = if cfg!(target_os = "macos") || direction == "none" {
            offset.saturating_sub(start) < end.saturating_sub(offset)
        } else {
            direction == "backward"
        };
        let anchor = match granularity {
            SelectionGranularity::Character => {
                let anchor = if !extend {
                    offset
                } else if anchor_at_end {
                    end
                } else {
                    start
                };
                (anchor, anchor)
            }
            SelectionGranularity::Word if start < end => (start, end),
            _ => granularity.at_point(runtime, control, caret),
        };
        // A press on an existing selection keeps the range until the gesture
        // becomes a click or starts extending the selection.
        let collapse_on_release = granularity == SelectionGranularity::Character
            && !extend
            && start < end
            && (start..=end).contains(&offset);
        if !collapse_on_release {
            let focus = if granularity == SelectionGranularity::Character {
                offset
            } else {
                anchor.1
            };
            set_mouse_selection(scope, host, control, anchor.0, focus);
        }
        Ok(Some(Self {
            control,
            document,
            anchor,
            granularity,
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
        let Some(caret) = caret_hit(runtime, self.control, self.document, position)? else {
            return Ok(false);
        };
        let range = self.granularity.at_point(runtime, self.control, caret);
        let (anchor, focus) = if range.0 < self.anchor.0 {
            (self.anchor.1, range.0)
        } else {
            (self.anchor.0, self.anchor.1.max(range.1))
        };
        if !self.collapse_on_release || focus != anchor {
            self.collapse_on_release = false;
            set_mouse_selection(scope, host, self.control, anchor, focus);
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
            set_mouse_selection(scope, host, self.control, self.anchor.0, self.anchor.0);
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

fn caret_hit(
    host: &PageScriptHost,
    control: DomHandle,
    document: InputDocument,
    position: InputPosition,
) -> Result<Option<LayoutTextControlCaret>, LayoutError> {
    if !host.layout_policy().uses_real_layout() {
        return Ok(None);
    }
    let value = text_control_value(host, control);
    if value.is_empty() {
        return Ok(Some(LayoutTextControlCaret {
            offset: 0,
            source_start: 0,
            at_source_start: true,
        }));
    }
    host.ensure_observable_layout()?;
    let Some(transform) = host.input_frame_transform(document.handle, None) else {
        return Ok(None);
    };
    let point = transform.map_point(position.layout_point());
    Ok(host
        .with_latest_layout_tree_for_document(document.handle, |tree| {
            tree.text_control_caret(control, point)
        })
        .flatten()
        .map(|caret| snap_caret_to_grapheme(&value, caret)))
}

fn snap_caret_to_grapheme(
    value: &str,
    mut caret: LayoutTextControlCaret,
) -> LayoutTextControlCaret {
    if value.is_ascii() {
        return caret;
    }
    // Several source fragments can share the same shaped cluster rectangle.
    // Resolve an interior offset to the grapheme edge on the hit side.
    let mut start = 0;
    for grapheme in value.graphemes(true) {
        let end = start + grapheme.encode_utf16().count();
        if start < caret.source_start && caret.source_start < end {
            caret.source_start = start;
        }
        if start < caret.offset && caret.offset < end {
            caret.offset = if caret.at_source_start { start } else { end };
        }
        if caret.offset <= end && caret.source_start <= end {
            break;
        }
        start = end;
    }
    caret
}

fn set_mouse_selection(
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyboardMovementUnit {
    Grapheme,
    Word,
    Line,
    Value,
}

pub(super) fn move_text_control_selection(
    scope: &mut v8::PinScope<'_, '_>,
    host: *mut PageScriptHost,
    control: DomHandle,
    key: &str,
    modifiers: InputModifiers,
) -> bool {
    let mac = cfg!(target_os = "macos");
    let control_key = modifiers.contains(InputModifiers::CONTROL);
    let command = mac && modifiers.contains(InputModifiers::META);
    let word = modifiers.contains(if mac {
        InputModifiers::ALT
    } else {
        InputModifiers::CONTROL
    });
    let horizontal_unit = if command {
        KeyboardMovementUnit::Line
    } else if word {
        KeyboardMovementUnit::Word
    } else {
        KeyboardMovementUnit::Grapheme
    };
    let edge_unit = if control_key && !mac {
        KeyboardMovementUnit::Value
    } else {
        KeyboardMovementUnit::Line
    };
    let (forward, unit) = match key {
        "arrowleft" | "left" => (false, horizontal_unit),
        "arrowright" | "right" => (true, horizontal_unit),
        "home" => (false, edge_unit),
        "end" => (true, edge_unit),
        "arrowup" | "up" if command => (false, KeyboardMovementUnit::Value),
        "arrowdown" | "down" if command => (true, KeyboardMovementUnit::Value),
        _ => return false,
    };
    let runtime = unsafe { &*host };
    let (start, end, direction) = current_selection_state(runtime, control);
    let shift = modifiers.contains(InputModifiers::SHIFT);
    let (anchor, focus) = if shift {
        if direction == "backward" || (direction == "none" && !forward) {
            (end, start)
        } else {
            (start, end)
        }
    } else {
        let edge = if forward { end } else { start };
        (edge, edge)
    };
    let next = if !shift && start != end && unit == KeyboardMovementUnit::Grapheme {
        focus
    } else {
        let value = text_control_value(runtime, control);
        match unit {
            KeyboardMovementUnit::Grapheme => grapheme_boundary(&value, focus, forward),
            KeyboardMovementUnit::Word => word_boundary(&value, focus, forward),
            KeyboardMovementUnit::Line => line_boundary(&value, focus, forward),
            KeyboardMovementUnit::Value if forward => value.encode_utf16().count() as u32,
            KeyboardMovementUnit::Value => 0,
        }
    };
    let crosses_anchor = (focus < anchor && next > anchor) || (focus > anchor && next < anchor);
    // Reversing a word extension on macOS first returns to the original caret.
    let next = if mac && shift && unit == KeyboardMovementUnit::Word && crosses_anchor {
        anchor
    } else {
        next
    };
    let anchor = if shift { anchor } else { next };
    let direction = match next.cmp(&anchor) {
        std::cmp::Ordering::Less => "backward",
        std::cmp::Ordering::Equal => "none",
        std::cmp::Ordering::Greater => "forward",
    };
    text_control_set_selection_range_with_direction_internal(
        scope,
        host,
        control,
        anchor.min(next),
        anchor.max(next),
        direction,
    );
    true
}

fn grapheme_boundary(value: &str, offset: u32, forward: bool) -> u32 {
    let mut end = 0;
    for grapheme in value.graphemes(true) {
        let start = end;
        end += grapheme.encode_utf16().count() as u32;
        if forward && end > offset {
            return end;
        }
        if !forward && end >= offset {
            return start;
        }
    }
    end
}

fn word_boundary(value: &str, offset: u32, forward: bool) -> u32 {
    let mut end = 0;
    let mut previous_word = 0;
    for segment in value.split_word_bounds() {
        let start = end;
        end += segment.encode_utf16().count() as u32;
        if !forward && start >= offset {
            break;
        }
        if !segment.chars().any(char::is_alphanumeric) {
            continue;
        }
        if forward {
            let boundary = if cfg!(target_os = "windows") {
                start
            } else {
                end
            };
            if boundary > offset {
                return boundary;
            }
        } else {
            previous_word = start;
        }
    }
    if forward { end } else { previous_word }
}

fn line_boundary(value: &str, offset: u32, forward: bool) -> u32 {
    let mut start = 0;
    for line in value.split('\n') {
        let end = start + line.encode_utf16().count() as u32;
        if offset <= end {
            return if forward { end } else { start };
        }
        start = end + 1;
    }
    start.saturating_sub(1)
}
