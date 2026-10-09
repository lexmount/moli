use super::super::{BridgeHandle, ComputedStyleDescriptor, DomTokenListKind, JsContextHost};
use super::NativeDomBridge;
use crate::document_runtime::DomHandle;

#[derive(Clone, Copy, PartialEq, Eq)]
enum BridgeWrapperIdentity {
    Canonical,
    NewObject,
}

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
            .existing_reflector_id(&BridgeHandle::Node(handle))?;
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
        self.materialize_bridge_wrapper(scope, host_ptr, handle, BridgeWrapperIdentity::Canonical)
    }

    fn materialize_bridge_wrapper<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        host_ptr: *mut JsContextHost,
        handle: BridgeHandle,
        identity: BridgeWrapperIdentity,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let current = scope.get_current_context();
        let current_host = crate::util::context_host_ptr_from_context_slot(current)?;
        let foreign_host = current_host != host_ptr;
        let producer = if handle.node_handle().is_some() {
            self.identity.node_owner_context(scope, host_ptr)
        } else {
            unsafe { &*host_ptr }.page_default_context(scope)
        };
        if identity == BridgeWrapperIdentity::Canonical
            && foreign_host
            && let Some(producer) = producer
            && super::super::identity::contexts_share_wrapper_world(current, producer)
        {
            // One-field wrappers resolve their native host from the creation
            // context. Canonical wrappers retain their producer realm.
            let scope = &mut v8::ContextScope::new(scope, producer);
            return self.materialize_bridge_wrapper(scope, host_ptr, handle, identity);
        }
        let native_owner = if foreign_host
            && (identity == BridgeWrapperIdentity::NewObject || handle.node_handle().is_some())
        {
            let producer = producer?;
            let scope = &mut v8::ContextScope::new(scope, producer);
            Some(v8::Global::new(scope, v8::Object::new(scope)))
        } else {
            None
        };
        // A reflector describes the native read source. NewObject declarations
        // can share that descriptor while their JS objects remain independent.
        let reflector_id = self.identity.reflector_id(&handle);
        if identity == BridgeWrapperIdentity::Canonical
            && let Some(wrapper) = self.identity.cached_wrapper(scope, reflector_id)
        {
            return Some(wrapper);
        }

        let wrapper =
            self.bindings
                .instantiate_wrapper(scope, host_ptr, handle.clone(), reflector_id);
        if let Some(node) = handle.node_handle() {
            self.identity
                .node_ownership(scope, host_ptr, node)
                .bind(scope, wrapper);
        }
        if let Some(owner) = native_owner {
            let owner = v8::Local::new(scope, owner);
            crate::util::set_private_value(
                scope,
                wrapper,
                super::super::helpers::NATIVE_BRIDGE_OWNER_SLOT,
                owner.into(),
            );
        }
        if identity == BridgeWrapperIdentity::Canonical {
            self.identity.cache_wrapper(scope, reflector_id, wrapper);
        }
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
        self.wrap_dom_token_list(scope, runtime_ptr, handle, DomTokenListKind::Class)
    }

    pub(crate) fn wrap_part_list<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        runtime_ptr: *mut JsContextHost,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.wrap_dom_token_list(scope, runtime_ptr, handle, DomTokenListKind::Part)
    }

    pub(in crate::native_bridge) fn wrap_dom_token_list<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        runtime_ptr: *mut JsContextHost,
        handle: DomHandle,
        kind: DomTokenListKind,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.wrap_bridge_handle(scope, runtime_ptr, BridgeHandle::ClassList(handle, kind))
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

    pub(crate) fn create_computed_style<'s, 'i>(
        &mut self,
        scope: &mut v8::PinScope<'s, 'i>,
        runtime_ptr: *mut JsContextHost,
        handle: DomHandle,
        descriptor: ComputedStyleDescriptor,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.materialize_bridge_wrapper(
            scope,
            runtime_ptr,
            BridgeHandle::ComputedStyle(handle, std::rc::Rc::new(descriptor)),
            BridgeWrapperIdentity::NewObject,
        )
    }
}
