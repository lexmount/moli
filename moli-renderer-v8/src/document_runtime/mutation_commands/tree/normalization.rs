use crate::{
    custom_elements,
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::Node,
    native_bridge::JsContextHost,
};

impl DocumentRuntime {
    pub(crate) fn normalize(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> bool {
        let steps = self.dom_host.text_normalization_steps(handle);
        custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
            let mut changed = false;
            for step in steps {
                if let Some(target) = step.merge_into {
                    let Some(mut value) = self
                        .dom_host
                        .node(target)
                        .and_then(Node::character_data_value)
                        .cloned()
                    else {
                        continue;
                    };
                    let Some(source) = self
                        .dom_host
                        .node(step.text)
                        .and_then(Node::character_data_value)
                        .cloned()
                    else {
                        continue;
                    };
                    let offset = value.utf16_units().len() as u32;
                    if !source.is_empty() {
                        value.append(&source);
                        changed |= self.set_character_data_value_in_current_reaction_queue(
                            scope, host_ptr, target, value, false,
                        );
                    }
                    // Range endpoints move into the retained text before ordinary
                    // removal can rehome them to the parent. Empty siblings count.
                    unsafe { &mut *host_ptr }
                        .update_live_range_records_for_text_merge(scope, target, step.text, offset);
                }
                changed |= self.remove_child_appending_to_current_reaction_queue(
                    scope,
                    host_ptr,
                    step.parent,
                    step.text,
                );
            }
            changed
        })
    }
}
