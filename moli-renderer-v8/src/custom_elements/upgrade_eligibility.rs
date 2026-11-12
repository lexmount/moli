use super::super::{document_runtime::DomHandle, native_bridge::PageScriptHost};
use super::definition_name_for_handle;

pub(super) fn can_upgrade_handle(host_ptr: *mut PageScriptHost, handle: DomHandle) -> bool {
    // A pending reaction does not change the element's state. A nested reaction
    // scope must be able to enqueue it again and perform the upgrade immediately.
    let host = unsafe { &*host_ptr };
    if host
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| {
            store.is_upgraded_handle(handle) || store.is_pending_construction_handle(handle)
        })
    {
        return false;
    }
    let Some(definition_name) = definition_name_for_handle(host_ptr, handle) else {
        return false;
    };
    host.custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.has_definition(&definition_name))
}
