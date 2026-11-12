use super::existing_upgrade::upgrade_handle_with_immediate_form_lifecycle_if_defined;
use super::{CustomElementRegistryAssociation, CustomElementRegistryKey};
use crate::{document_runtime::DomHandle, native_bridge::PageScriptHost};

pub(crate) fn initialize_registry_for_subtree(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    root: DomHandle,
    registry_key: CustomElementRegistryKey,
) -> bool {
    initialize_registry_for_subtree_inner(scope, host_ptr, root, registry_key, false)
}

fn initialize_registry_for_subtree_inner(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    root: DomHandle,
    registry_key: CustomElementRegistryKey,
    ancestor_initialized: bool,
) -> bool {
    let mut stack = vec![(root, ancestor_initialized)];
    while let Some((handle, ancestor_initialized)) = stack.pop() {
        let explicit_registry = unsafe { &*host_ptr }.custom_element_registry_association(handle);
        match explicit_registry {
            Some(CustomElementRegistryAssociation::Registry(_)) => {}
            _ => {
                let current_registry =
                    unsafe { &*host_ptr }.effective_custom_element_registry_association(handle);
                if !ancestor_initialized {
                    match current_registry {
                        CustomElementRegistryAssociation::Null => {}
                        CustomElementRegistryAssociation::Registry(existing_key)
                            if existing_key == registry_key => {}
                        CustomElementRegistryAssociation::Registry(_) => continue,
                    }
                }
                unsafe { &mut *host_ptr }.set_custom_element_registry_association(
                    handle,
                    CustomElementRegistryAssociation::Registry(registry_key),
                );
            }
        }
        if !upgrade_handle_with_immediate_form_lifecycle_if_defined(scope, host_ptr, handle) {
            return false;
        }
        let children = unsafe { &*host_ptr }
            .dom_host()
            .child_handles(handle)
            .collect::<Vec<_>>();
        stack.extend(children.into_iter().rev().map(|child| (child, true)));
    }
    true
}
