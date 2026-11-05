use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(super) struct CustomElementReactionGuard {
    host_ptr: *mut JsContextHost,
}

struct CustomElementReactionScope<'scope, 'pin, 'host> {
    scope: &'host mut v8::PinScope<'scope, 'pin>,
    host_ptr: *mut JsContextHost,
}

pub(super) struct DynamicMarkupInsertionGuard {
    host_ptr: *mut JsContextHost,
    document: Option<DomHandle>,
}

impl Drop for CustomElementReactionGuard {
    fn drop(&mut self) {
        unsafe { &mut *self.host_ptr }.exit_custom_element_reaction();
    }
}

impl Drop for CustomElementReactionScope<'_, '_, '_> {
    fn drop(&mut self) {
        unsafe { &mut *self.host_ptr }
            .native_bridge_mut()
            .synchronize_custom_element_registry_references(self.scope, self.host_ptr);
        super::reactions::flush_current_custom_element_reaction_queue(self.scope, self.host_ptr);
    }
}

impl Drop for DynamicMarkupInsertionGuard {
    fn drop(&mut self) {
        if let Some(document) = self.document {
            unsafe { &mut *self.host_ptr }.exit_throw_on_dynamic_markup_insertion(document);
        }
    }
}

pub(super) fn enter_custom_element_reaction(
    host_ptr: *mut JsContextHost,
) -> CustomElementReactionGuard {
    unsafe { &mut *host_ptr }.enter_custom_element_reaction();
    CustomElementReactionGuard { host_ptr }
}

pub(super) fn enter_upgrade_dynamic_markup_insertion(
    host_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> DynamicMarkupInsertionGuard {
    let document = unsafe { &*host_ptr }
        .dom_host()
        .owner_document_handle(handle);
    if let Some(document) = document {
        unsafe { &mut *host_ptr }.enter_throw_on_dynamic_markup_insertion(document);
    }
    DynamicMarkupInsertionGuard { host_ptr, document }
}

impl<'scope, 'pin, 'host> CustomElementReactionScope<'scope, 'pin, 'host> {
    fn enter(scope: &'host mut v8::PinScope<'scope, 'pin>, host_ptr: *mut JsContextHost) -> Self {
        unsafe { &mut *host_ptr }
            .custom_element_reactions_mut()
            .push_element_queue();
        Self { scope, host_ptr }
    }

    fn scope(&mut self) -> &mut v8::PinScope<'scope, 'pin> {
        self.scope
    }
}

pub(crate) fn with_custom_element_reaction_scope<'scope, 'pin, R>(
    scope: &mut v8::PinScope<'scope, 'pin>,
    host_ptr: *mut JsContextHost,
    op: impl FnOnce(&mut v8::PinScope<'scope, 'pin>) -> R,
) -> R {
    let mut reaction_scope = CustomElementReactionScope::enter(scope, host_ptr);
    op(reaction_scope.scope())
}
