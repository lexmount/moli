use super::{JsContextHost, TextControlSelection, text_control_value};
use crate::document_runtime::DomHandle;
use crate::dom::native::DomMutationEffects;

impl JsContextHost {
    pub(crate) fn reconcile_text_control_selection_dom_mutations(
        &mut self,
        effects: &DomMutationEffects,
    ) -> Vec<(DomHandle, bool)> {
        for mutation in effects.style().attribute_mutations() {
            self.reconcile_text_control_selection_value(mutation.target());
        }
        let mut changed = Vec::new();
        for change in effects.textarea_values() {
            let control = change.target();
            let Some(selection) = self.text_control_selection(control) else {
                continue;
            };
            let mut selection = selection.clamp(change.length());
            selection.direction = self.text_control_selection_direction(control, "none");
            let selection_changed = self.set_text_control_selection(
                control,
                selection.start,
                selection.end,
                selection.direction,
            );
            let focused = self.active_element_handle() == Some(control);
            let document = self.dom_host().owner_document_handle(control);
            if let Some(selected) = document
                .and_then(|document| {
                    self.selection_record_registry
                        .text_controls
                        .get_mut(&document)
                })
                .filter(|selected| selected.control == control)
            {
                selected.selection = if focused {
                    selection
                } else {
                    TextControlSelection::caret(0)
                };
            }
            if let Some((_, changed_selection)) =
                changed.iter_mut().find(|(handle, _)| *handle == control)
            {
                *changed_selection |= selection_changed;
            } else {
                changed.push((control, selection_changed));
            }
        }
        for &(control, _) in &changed {
            let value = text_control_value(self, control);
            let document = self.dom_host().owner_document_handle(control);
            if let Some(selected) = document
                .and_then(|document| {
                    self.selection_record_registry
                        .text_controls
                        .get_mut(&document)
                })
                .filter(|selected| selected.control == control)
            {
                selected.value = value;
            }
        }
        changed
    }
}
