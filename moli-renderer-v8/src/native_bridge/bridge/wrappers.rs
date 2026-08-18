use super::super::{
    BridgeHandle, ComputedStyleDescriptor, DomTokenListKind, JsContextHost, ReflectorId,
};
use super::NativeDomBridge;
use crate::document_runtime::DomHandle;

impl NativeDomBridge {
    pub(crate) fn install_default_world_wrapper_cache(&self, context: v8::Local<'_, v8::Context>) {
        self.identity.install_default_world_wrapper_cache(context);
    }

    pub(crate) fn wrap_handle<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        host_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.wrap_bridge_handle(scope, host_ptr, BridgeHandle::Node(handle))
    }

    pub(crate) fn cached_handle_wrapper<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let reflector_id = self
            .identity
            .existing_reflector_id(BridgeHandle::Node(handle))?;
        self.identity.cached_wrapper(scope, reflector_id)
    }

    pub(crate) fn retire_default_world_wrappers_for_realm(
        &self,
        realm_token: crate::native_bridge::RuntimeObservableContextToken,
    ) {
        self.identity
            .retire_default_world_wrappers_for_realm(realm_token);
    }

    pub(crate) fn wrap_window<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        host_ptr: *mut JsContextHost,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.wrap_bridge_handle(scope, host_ptr, BridgeHandle::Window)
    }

    pub(super) fn wrap_bridge_handle<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        host_ptr: *mut JsContextHost,
        handle: BridgeHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let reflector_id = self.identity.reflector_id(handle.clone());
        if let Some(wrapper) = self.identity.cached_wrapper(scope, reflector_id) {
            if !matches!(&handle, BridgeHandle::Window) {
                self.bindings
                    .sync_wrapper_owner_realm_prototype(scope, host_ptr, &handle, wrapper);
            }
            return Some(wrapper);
        }

        let wrapper = self
            .bindings
            .instantiate_wrapper(scope, host_ptr, handle, reflector_id);
        self.identity.cache_wrapper(scope, reflector_id, wrapper);
        Some(wrapper)
    }

    pub(crate) fn wrap_handle_for_receiver<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        host_ptr: *mut JsContextHost,
        receiver: v8::Local<'s, v8::Object>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let creation_context = receiver.get_creation_context(scope)?;
        let bridge_handle = BridgeHandle::Node(handle);
        if creation_context == scope.get_current_context() {
            return self.wrap_bridge_handle(scope, host_ptr, bridge_handle);
        }

        let wrapper = {
            let target_scope = &mut v8::ContextScope::new(scope, creation_context);
            let wrapper = self.wrap_bridge_handle(target_scope, host_ptr, bridge_handle)?;
            v8::Global::new(target_scope, wrapper)
        };
        Some(v8::Local::new(scope, &wrapper))
    }

    pub(crate) fn wrap_class_list<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        runtime_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.wrap_bridge_handle(
            scope,
            runtime_ptr,
            BridgeHandle::ClassList(handle, DomTokenListKind::Class),
        )
    }

    pub(crate) fn wrap_part_list<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        runtime_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.wrap_bridge_handle(
            scope,
            runtime_ptr,
            BridgeHandle::ClassList(handle, DomTokenListKind::Part),
        )
    }

    pub(crate) fn wrap_rel_list<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        runtime_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.wrap_bridge_handle(
            scope,
            runtime_ptr,
            BridgeHandle::ClassList(handle, DomTokenListKind::Rel),
        )
    }

    pub(crate) fn wrap_dataset<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        runtime_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.wrap_bridge_handle(scope, runtime_ptr, BridgeHandle::Dataset(handle))
    }

    pub(crate) fn wrap_style<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        runtime_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.wrap_bridge_handle(scope, runtime_ptr, BridgeHandle::Style(handle))
    }

    pub(crate) fn wrap_computed_style<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        runtime_ptr: *mut JsContextHost,
        handle: DomHandle,
        descriptor: ComputedStyleDescriptor,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.wrap_bridge_handle(
            scope,
            runtime_ptr,
            BridgeHandle::ComputedStyle(handle, descriptor),
        )
    }

    pub(crate) fn resolve_node_handle(&self, reflector_id: ReflectorId) -> Option<DomHandle> {
        match self.bridge_handle(reflector_id) {
            Some(BridgeHandle::Node(handle)) => Some(handle),
            Some(
                BridgeHandle::Window
                | BridgeHandle::ClassList(_, _)
                | BridgeHandle::Dataset(_)
                | BridgeHandle::Style(_)
                | BridgeHandle::ComputedStyle(_, _),
            )
            | None => None,
        }
    }
}
