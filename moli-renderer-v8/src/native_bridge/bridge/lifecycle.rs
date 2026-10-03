use super::super::{
    BridgeIdentityStore, JsContextHost, NativeBridgeBindings, abort, document, traversal,
};
use super::NativeDomBridge;
use anyhow::Result;
use std::ffi::c_void;

pub(crate) fn install_detached_bridge_methods(scope: &mut v8::PinScope<'_, '_>) {
    document::install_detached_bridge_methods(scope);
}

impl NativeDomBridge {
    pub(crate) fn new(bindings: NativeBridgeBindings) -> Self {
        Self {
            bindings,
            identity: BridgeIdentityStore::default(),
            abort: abort::AbortStore::default(),
            web_mcp: crate::context_bootstrap::web_mcp::ModelContextStore::default(),
            traversal: traversal::TraversalStore::default(),
        }
    }

    pub(crate) fn install_global<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        global: v8::Local<'s, v8::Object>,
        host_ptr: *mut JsContextHost,
        rc_ptr: *mut c_void,
    ) -> Result<()> {
        self.bindings
            .install_global(scope, global, host_ptr, rc_ptr)
    }
}
