use super::*;

mod browser_font_preferences;
mod child_list;
mod image_presentation_hints;

fn inspector_active_child_window_scope_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let active = crate::native_bridge::active_child_window_handle(scope).is_some();
    rv.set(v8::Boolean::new(scope, active).into());
}

fn child_document_handle_for_frame_id(vm: &ScriptVm, frame_id: &str) -> DomHandle {
    let frame = element_handle_by_id(vm, frame_id);
    vm._context_host
        .borrow()
        .child_browsing_context_document_handle(frame)
        .expect("iframe should have a child document handle")
}

fn element_handle_by_id(vm: &ScriptVm, id: &str) -> DomHandle {
    vm.document_runtime
        .dom_host()
        .dom()
        .nodes()
        .iter()
        .enumerate()
        .find_map(|(index, node)| {
            let element = node.as_element()?;
            (element.attribute("id") == Some(id)).then_some(DomHandle::new(index))
        })
        .unwrap_or_else(|| panic!("detached element #{id} should have a native handle"))
}

fn owner_document_handle_for_element_id(vm: &ScriptVm, id: &str) -> DomHandle {
    let element = element_handle_by_id(vm, id);
    vm.document_runtime
        .dom_host()
        .owner_document_handle(element)
        .unwrap_or_else(|| panic!("detached element #{id} should have an owner document"))
}

fn computed_style_cache_entry_count_for_document(vm: &ScriptVm, document: DomHandle) -> usize {
    vm._context_host
        .borrow()
        .computed_style_cache_entry_count_for_document_for_test(document)
}

fn registered_custom_property_for_document(vm: &ScriptVm, document: DomHandle, name: &str) -> bool {
    vm._context_host
        .borrow()
        .registered_css_custom_property_registration(document, name)
        .is_some()
}

mod advanced_style_values;
mod computed_style_access;
mod content_and_invalidation;
mod cross_document_and_animations;
mod nested_document_invalidation;
mod properties_and_selectors;
mod stylesheet_and_document_lifecycle;
