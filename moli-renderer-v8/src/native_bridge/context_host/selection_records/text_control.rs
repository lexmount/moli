use super::{JsContextHost, SelectionDirection, SelectionRecordHandle};
use crate::document_runtime::DomHandle;
use crate::dom::forms::InputType;
use crate::dom::native::{Element, Node};
use crate::native_bridge::element::text_control_value;
use icu_segmenter::GraphemeClusterSegmenter;

mod mutations;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextControlSelection {
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) direction: &'static str,
}

impl TextControlSelection {
    fn cached(element: &Element) -> Self {
        Self {
            start: element.selection_start(),
            end: element.selection_end(),
            direction: match element.selection_direction() {
                "backward" => "backward",
                "forward" => "forward",
                _ => "none",
            },
        }
    }

    fn caret(offset: u32) -> Self {
        Self {
            start: offset,
            end: offset,
            direction: "forward",
        }
    }

    fn clamp(mut self, length: u32) -> Self {
        self.end = self.end.min(length);
        self.start = self.start.min(self.end);
        self
    }
}

pub(super) struct SelectedTextControl {
    pub(super) control: DomHandle,
    selection: TextControlSelection,
    // This is the text underlying the Document's selection. Cached control
    // offsets may change independently while the control is not focused.
    value: String,
}

impl SelectedTextControl {
    fn text(&self, password: bool) -> String {
        let mut result = String::new();
        let mut previous = 0;
        let mut offset = 0;
        for boundary in GraphemeClusterSegmenter::new()
            .segment_str(&self.value)
            .skip(1)
        {
            if offset >= self.selection.end {
                break;
            }
            let cluster = &self.value[previous..boundary];
            if offset >= self.selection.start {
                if password {
                    result.push('\u{2022}');
                } else {
                    result.push_str(cluster);
                }
            }
            offset += cluster.encode_utf16().count() as u32;
            previous = boundary;
        }
        result
    }
}

impl JsContextHost {
    pub(crate) fn cached_text_control_selection(
        &self,
        control: DomHandle,
    ) -> Option<TextControlSelection> {
        let element = self.dom_host().node(control)?.as_element()?;
        let mut selection = TextControlSelection::cached(element);
        selection.direction = self.text_control_selection_direction(control, selection.direction);
        Some(selection)
    }

    pub(crate) fn set_text_control_selection(
        &mut self,
        control: DomHandle,
        start: u32,
        end: u32,
        direction: &str,
    ) -> bool {
        let before = self.cached_text_control_selection(control);
        let direction = self.text_control_selection_direction(control, direction);
        let changed = self.set_selection_range_with_direction(control, start, end, direction);
        // A real Document's default direction is already forward. Materializing
        // that default in the cache must not manufacture a selectionchange.
        changed
            && before
                != Some(TextControlSelection {
                    start,
                    end,
                    direction,
                })
    }

    pub(crate) fn text_control_selection_direction(
        &self,
        control: DomHandle,
        direction: &str,
    ) -> &'static str {
        match direction {
            "backward" => "backward",
            "forward" => "forward",
            _ if self.owner_dispatch_scope_for_node(control).is_some() => "forward",
            _ => "none",
        }
    }

    pub(crate) fn text_control_has_selection_editor(&self, control: DomHandle) -> bool {
        self.dom_host()
            .node(control)
            .and_then(Node::as_element)
            .is_some_and(|element| {
                element.is_html_textarea()
                    || element.is_html_input()
                        && matches!(
                            element.input_type(),
                            InputType::Text
                                | InputType::Search
                                | InputType::Tel
                                | InputType::Url
                                | InputType::Email
                                | InputType::Password
                                | InputType::Number
                        )
            })
    }

    pub(crate) fn note_text_control_selection(&mut self, control: DomHandle) {
        if !self.text_control_has_selection_editor(control) {
            return;
        }
        let Some(document) = self.dom_host().owner_document_handle(control) else {
            return;
        };
        let Some(element) = self.dom_host().node(control).and_then(Node::as_element) else {
            return;
        };
        let mut selection = TextControlSelection::cached(element);
        // Text field selections are directional even when the caller omitted
        // a direction. A collapsed Document Selection still reports "none".
        if selection.direction == "none" {
            selection.direction = "forward";
        }
        if let Some(selected) = self
            .selection_record_registry
            .text_controls
            .get_mut(&document)
            && selected.control == control
        {
            selected.selection = selection.clamp(selected.value.encode_utf16().count() as u32);
            return;
        }
        let value = text_control_value(self, control);
        let selection = selection.clamp(value.encode_utf16().count() as u32);
        self.selection_record_registry.text_controls.insert(
            document,
            SelectedTextControl {
                control,
                selection,
                value,
            },
        );
    }

    pub(crate) fn document_selected_text_control(&self, document: DomHandle) -> Option<DomHandle> {
        self.selected_text_control(document)
            .map(|selected| selected.control)
    }

    fn selected_text_control(&self, document: DomHandle) -> Option<&SelectedTextControl> {
        let selected = self
            .selection_record_registry
            .text_controls
            .get(&document)?;
        (self.dom_host().is_connected(selected.control)
            && self.dom_host().owner_document_handle(selected.control) == Some(document)
            && self.text_control_has_selection_editor(selected.control))
        .then_some(selected)
    }

    pub(crate) fn text_control_selection(
        &self,
        control: DomHandle,
    ) -> Option<TextControlSelection> {
        if self.active_element_handle() != Some(control) {
            return self.cached_text_control_selection(control);
        }
        let document = self.dom_host().owner_document_handle(control)?;
        if let Some(selected) = self.selected_text_control(document)
            && selected.control == control
        {
            return Some(selected.selection);
        }
        // Focusing does not make the cached range authoritative. An explicit
        // Document Selection can be empty or lie outside the focused control.
        let direction = self
            .selection_record_registry
            .records
            .values()
            .find(|record| record.owner_document == Some(document))
            .map_or("forward", |record| match record.direction {
                SelectionDirection::Backward => "backward",
                _ => "forward",
            });
        Some(TextControlSelection {
            direction,
            ..TextControlSelection::caret(0)
        })
    }

    pub(crate) fn selection_record_text_control(
        &self,
        handle: SelectionRecordHandle,
    ) -> Option<TextControlSelection> {
        let record = self.selection_record_registry.records.get(&handle)?;
        if !record.has_range() {
            return None;
        }
        Some(
            self.selected_text_control(record.owner_document?)?
                .selection,
        )
    }

    pub(crate) fn selection_record_text_control_text(
        &self,
        handle: SelectionRecordHandle,
    ) -> Option<String> {
        let record = self.selection_record_registry.records.get(&handle)?;
        if !record.has_range() {
            return None;
        }
        self.document_text_control_selection_text(record.owner_document?)
    }

    pub(crate) fn document_text_control_selection_text(
        &self,
        document: DomHandle,
    ) -> Option<String> {
        let selected = self.selected_text_control(document)?;
        let element = self.dom_host().node(selected.control)?.as_element()?;
        let password = element.is_html_input() && element.input_type() == InputType::Password;
        Some(selected.text(password))
    }

    pub(crate) fn reconcile_text_control_selection_value(&mut self, control: DomHandle) {
        let Some(document) = self.dom_host().owner_document_handle(control) else {
            return;
        };
        if !self
            .selection_record_registry
            .text_controls
            .get(&document)
            .is_some_and(|selected| selected.control == control)
        {
            return;
        }
        if !self.text_control_has_selection_editor(control) {
            self.selection_record_registry
                .text_controls
                .remove(&document);
            return;
        }
        let value = text_control_value(self, control);
        let selected = self
            .selection_record_registry
            .text_controls
            .get_mut(&document)
            .expect("selected control checked");
        if selected.value != value {
            selected.selection = TextControlSelection::caret(0);
            selected.value = value;
        }
    }
}
