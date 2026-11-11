use super::*;
use crate::native_bridge::element::{
    activate_handle_via_click_with_detail_and_modifiers, form_associated_form_owner,
    is_valid_submit_button, submit_form_with_submit_event,
};
use std::collections::HashSet;

#[cfg(test)]
mod tests;

impl ScriptVm {
    /// Replaces the complete value of a text control or root editing host
    /// through the same trusted editing primitives used by real input.
    pub(crate) fn replace_control_text(&mut self, handle: DomHandle, text: &str) -> Result<bool> {
        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            update_focus(scope, runtime_ptr, Some(handle));
            let runtime = unsafe { &*runtime_ptr };
            if crate::native_bridge::element::is_text_control(runtime, handle) {
                let value_len = utf16_len(&text_control_value(runtime, handle)) as u32;
                let _ = text_control_set_selection_range_internal(
                    scope,
                    runtime_ptr,
                    handle,
                    0,
                    value_len,
                );
                return Ok(replace_text_control_selection(
                    scope,
                    runtime_ptr,
                    handle,
                    text,
                    TextEditInputType::InsertText,
                ));
            }
            let is_native_control = runtime
                .dom_host()
                .node(handle)
                .and_then(Node::as_element)
                .is_some_and(|element| element.is_html_input() || element.is_html_textarea());
            if is_native_control {
                let runtime = unsafe { &mut *runtime_ptr };
                let changed = runtime.set_input_value(handle, text);
                if changed {
                    if let Some(event) = construct_simple_event(scope, "input", true, false, false)
                    {
                        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                    }
                    if let Some(event) = construct_simple_event(scope, "change", true, false, false)
                    {
                        let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                    }
                }
                return Ok(true);
            }
            if contenteditable_editing_host(runtime, handle) != Some(handle) {
                return Ok(false);
            }
            let _ = select_contenteditable_contents(scope, handle);
            Ok(replace_contenteditable_selection(
                scope,
                runtime_ptr,
                handle,
                text,
                TextEditInputType::InsertText,
            ))
        });
        self.finish_input_command_checkpoint(result)
    }

    /// Explicitly requests submission of a form or a control's form. Only a
    /// supplied submit button is the submitter; no default button is clicked.
    /// Enter-key implicit submission stays in the native keyboard path.
    pub(crate) fn submit_control_form(
        &mut self,
        handle: DomHandle,
    ) -> Result<Option<RendererInputDispatchOutcome>> {
        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            let runtime = unsafe { &*runtime_ptr };
            let form = if runtime.dom_host().is_html_element_named(handle, "form") {
                Some(handle)
            } else {
                form_associated_form_owner(runtime, handle)
            };
            let Some(form) = form else {
                return Ok(None);
            };
            let submitter = is_valid_submit_button(runtime, handle).then_some(handle);
            let submitted =
                submit_form_with_submit_event(scope, runtime_ptr, form, submitter, true);
            Ok(Some(input_dispatch_outcome(submitted)))
        });
        self.finish_input_event_dispatch_turn(result)
    }

    /// Applies a complete native-select value set atomically and emits the
    /// trusted input/change pair only after validation has succeeded.
    pub(crate) fn select_control_values(
        &mut self,
        handle: DomHandle,
        values: &[String],
    ) -> Result<bool> {
        let selected = values.iter().map(String::as_str).collect::<HashSet<_>>();
        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            let runtime = unsafe { &mut *runtime_ptr };
            let select = runtime.dom_host().node(handle).and_then(Node::as_element);
            anyhow::ensure!(
                select.is_some_and(|element| element.is_html_element("select")),
                "Node is not a select element"
            );
            let multiple = select.unwrap().has_attribute("multiple");
            anyhow::ensure!(
                multiple || selected.len() <= 1,
                "Single select cannot select multiple values"
            );
            let options = runtime.dom_host().select_option_elements(handle);
            let mut unmatched = selected.clone();
            let mut selected_single_option = false;
            let selection = options
                .into_iter()
                .map(|option| {
                    let value = runtime
                        .dom_host()
                        .node(option)
                        .and_then(Node::as_element)
                        .map(|element| element.option_value(runtime.dom_host().dom(), option))
                        .unwrap_or_default();
                    unmatched.remove(value.as_str());
                    let should_select =
                        selected.contains(value.as_str()) && (multiple || !selected_single_option);
                    selected_single_option |= should_select && !multiple;
                    (option, should_select)
                })
                .collect::<Vec<_>>();
            anyhow::ensure!(unmatched.is_empty(), "Select option value not found");
            let before = runtime.dom_host().select_selected_option_elements(handle);
            let _ =
                runtime.set_select_explicit_none(scope, runtime_ptr, handle, selected.is_empty());
            for (option, should_select) in selection {
                let _ = runtime.set_selected_state(scope, runtime_ptr, option, should_select);
            }
            let after = runtime.dom_host().select_selected_option_elements(handle);
            if before != after {
                if let Some(event) = construct_simple_event(scope, "input", true, false, false) {
                    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                }
                if let Some(event) = construct_simple_event(scope, "change", true, false, false) {
                    let _ = dispatch_public_event(scope, runtime_ptr, handle, event);
                }
            }
            Ok(true)
        });
        self.finish_input_command_checkpoint(result)
    }

    /// Dispatches the trusted click sequence directly to a retained node. The caller chooses whether geometric hit testing is required.
    pub(crate) fn activate_control(
        &mut self,
        handle: DomHandle,
        x: f64,
        y: f64,
        modifiers: u8,
    ) -> Result<RendererInputDispatchOutcome> {
        let _current_input_event = CurrentInputEventScope::enter(
            Rc::clone(&self._context_host),
            CurrentInputEvent::mouse("click", 0, modifiers),
        );
        let result = self.with_default_context_scope(|scope, runtime_ptr| {
            Ok(activate_handle_via_click_with_detail_and_modifiers(
                scope,
                runtime_ptr,
                handle,
                x,
                y,
                0,
                0,
                1,
                modifiers,
            ))
        });
        self.finish_input_event_dispatch_turn(result)
    }
}
