use super::super::{document_runtime::DomHandle, native_bridge::JsContextHost};
use super::definition::CustomElementStore;

impl CustomElementStore {
    pub(super) fn lifecycle_callback_for_handle<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
        callback_name: &str,
    ) -> Option<v8::Local<'s, v8::Function>> {
        let definition = self.definition_for_handle(host_ptr, handle)?;
        let callback = match callback_name {
            "connectedCallback" => definition.callbacks.connected.as_ref(),
            "disconnectedCallback" => definition.callbacks.disconnected.as_ref(),
            "connectedMoveCallback" => definition.callbacks.connected_move.as_ref(),
            "adoptedCallback" => definition.callbacks.adopted.as_ref(),
            _ => None,
        }?;
        Some(v8::Local::new(scope, callback))
    }

    pub(super) fn attribute_changed_callback_for_handle<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Function>> {
        let definition = self.definition_for_handle(host_ptr, handle)?;
        let callback = definition.callbacks.attribute_changed.as_ref()?;
        Some(v8::Local::new(scope, callback))
    }

    pub(super) fn form_associated_callback_for_handle<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Function>> {
        let definition = self.definition_for_handle(host_ptr, handle)?;
        let callback = definition.callbacks.form_associated.as_ref()?;
        Some(v8::Local::new(scope, callback))
    }

    pub(super) fn form_disabled_callback_for_handle<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Function>> {
        let definition = self.definition_for_handle(host_ptr, handle)?;
        let callback = definition.callbacks.form_disabled.as_ref()?;
        Some(v8::Local::new(scope, callback))
    }

    pub(super) fn form_reset_callback_for_handle<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Function>> {
        let definition = self.definition_for_handle(host_ptr, handle)?;
        let callback = definition.callbacks.form_reset.as_ref()?;
        Some(v8::Local::new(scope, callback))
    }

    pub(super) fn form_state_restore_callback_for_handle<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Function>> {
        let definition = self.definition_for_handle(host_ptr, handle)?;
        let callback = definition.callbacks.form_state_restore.as_ref()?;
        Some(v8::Local::new(scope, callback))
    }
}
