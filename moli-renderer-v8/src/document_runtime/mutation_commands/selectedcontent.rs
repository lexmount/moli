use crate::{
    custom_elements,
    document_runtime::{DocumentRuntime, DomHandle},
    dom::native::Node,
    native_bridge::JsContextHost,
};

impl DocumentRuntime {
    pub(crate) fn sync_selectedcontents_for_select_in_reaction_scope(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        select: DomHandle,
    ) -> bool {
        custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
            self.sync_selectedcontents_for_select_appending_to_current_reaction_queue(
                scope, host_ptr, select,
            )
        })
    }

    pub(crate) fn sync_selectedcontents_after_parser_option_finished(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        option: DomHandle,
    ) -> bool {
        let Some(select) = self.dom_host.option_nearest_ancestor_select(option) else {
            return false;
        };
        if self
            .dom_host
            .select_selected_option_elements(select)
            .first()
            .copied()
            != Some(option)
        {
            return false;
        }
        self.sync_selectedcontents_for_select_appending_to_current_reaction_queue(
            scope, host_ptr, select,
        )
    }

    pub(crate) fn sync_selectedcontents_for_select_appending_to_current_reaction_queue(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        select: DomHandle,
    ) -> bool {
        let Some(select_element) = self.dom_host.node(select).and_then(Node::as_element) else {
            return false;
        };
        if !select_element.is_html_select() || select_element.has_attribute("multiple") {
            return false;
        }

        let selected_option = self
            .dom_host
            .select_selected_option_elements(select)
            .first()
            .copied();
        let targets = self.dom_host.select_selectedcontent_elements(select);
        let mut changed = false;
        for target in targets {
            changed |= self.clone_selected_option_contents_into_selectedcontent(
                scope,
                host_ptr,
                target,
                selected_option,
            );
        }
        changed
    }

    fn clone_selected_option_contents_into_selectedcontent(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        selectedcontent: DomHandle,
        selected_option: Option<DomHandle>,
    ) -> bool {
        let Some(document) = self.dom_host.owner_document_handle(selectedcontent) else {
            return false;
        };
        let fragment = self.create_document_fragment_for_document(document);
        if let Some(option) = selected_option {
            let source_children = self.dom_host.child_handles(option).collect::<Vec<_>>();
            for source_child in source_children {
                let Some(clone) = self.clone_node(scope, host_ptr, source_child, true) else {
                    return false;
                };
                if !self
                    .dom_host
                    .append_child_without_mutation_effects(fragment, clone)
                {
                    return false;
                }
            }
        }

        let existing_children = self
            .dom_host
            .child_handles(selectedcontent)
            .collect::<Vec<_>>();
        self.replace_all_children_with_fragment_appending_to_current_reaction_queue(
            scope,
            host_ptr,
            selectedcontent,
            fragment,
            &existing_children,
        )
    }
}
