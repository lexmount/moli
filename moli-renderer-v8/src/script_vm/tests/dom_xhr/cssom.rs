use super::*;

fn cssom_element_handle_by_id(vm: &ScriptVm, id: &str) -> DomHandle {
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
        .unwrap_or_else(|| panic!("element #{id} should have a native handle"))
}

fn cssom_owner_document_handle_for_element_id(vm: &ScriptVm, id: &str) -> DomHandle {
    let element = cssom_element_handle_by_id(vm, id);
    vm.document_runtime
        .dom_host()
        .owner_document_handle(element)
        .unwrap_or_else(|| panic!("element #{id} should have an owner document"))
}
mod extracted;
