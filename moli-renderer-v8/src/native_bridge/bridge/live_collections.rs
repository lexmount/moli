use super::super::LiveCollectionDescriptor;
use super::NativeDomBridge;

impl NativeDomBridge {
    pub(in crate::native_bridge) fn register_live_collection(
        &mut self,
        descriptor: LiveCollectionDescriptor,
    ) -> u32 {
        self.identity.register_live_collection(descriptor)
    }

    pub(in crate::native_bridge) fn live_collection_descriptor(
        &self,
        collection_id: u32,
    ) -> Option<&LiveCollectionDescriptor> {
        self.identity.live_collection_descriptor(collection_id)
    }

    pub(in crate::native_bridge) fn cached_live_collection_wrapper<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        descriptor: &LiveCollectionDescriptor,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.identity
            .cached_live_collection_wrapper(scope, descriptor)
    }

    pub(in crate::native_bridge) fn cache_live_collection_wrapper(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        descriptor: LiveCollectionDescriptor,
        wrapper: v8::Local<'_, v8::Object>,
    ) {
        self.identity
            .cache_live_collection_wrapper(scope, descriptor, wrapper);
    }

    pub(in crate::native_bridge) fn bind_cached_live_collection_owner_realm(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut super::super::JsContextHost,
        descriptor: &LiveCollectionDescriptor,
    ) -> Option<crate::document_runtime::DomHandle> {
        self.identity
            .bind_cached_live_collection_owner_realm(scope, host_ptr, descriptor)
    }

    pub(in crate::native_bridge) fn register_static_handle_collection(
        &mut self,
        handles: Vec<crate::document_runtime::DomHandle>,
    ) -> u32 {
        self.identity.register_static_handle_collection(handles)
    }

    pub(in crate::native_bridge) fn static_handle_collection_len(
        &self,
        collection_id: u32,
    ) -> Option<usize> {
        self.identity.static_handle_collection_len(collection_id)
    }

    pub(in crate::native_bridge) fn static_handle_collection_handle_at(
        &self,
        collection_id: u32,
        index: usize,
    ) -> Option<crate::document_runtime::DomHandle> {
        self.identity
            .static_handle_collection_handle_at(collection_id, index)
    }
}
