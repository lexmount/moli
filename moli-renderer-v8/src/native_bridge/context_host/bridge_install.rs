use super::*;
use anyhow::Result;
use std::{cell::RefCell, ffi::c_void, ptr::NonNull, rc::Rc};

pub(crate) struct JsContextHostBridgeRef {
    ptr: NonNull<RefCell<JsContextHost>>,
    #[cfg(test)]
    bridge_ref_count: Rc<std::cell::Cell<usize>>,
}

impl JsContextHostBridgeRef {
    #[cfg(not(test))]
    fn new(ptr: *const RefCell<JsContextHost>) -> Self {
        Self {
            ptr: NonNull::new(ptr.cast_mut())
                .expect("V8 bridge JsContextHost Rc pointer should not be null"),
        }
    }

    #[cfg(test)]
    fn new(
        ptr: *const RefCell<JsContextHost>,
        bridge_ref_count: Rc<std::cell::Cell<usize>>,
    ) -> Self {
        bridge_ref_count.set(bridge_ref_count.get() + 1);
        Self {
            ptr: NonNull::new(ptr.cast_mut())
                .expect("V8 bridge JsContextHost Rc pointer should not be null"),
            bridge_ref_count,
        }
    }

    fn as_external_ptr(&self) -> *mut c_void {
        self.ptr.as_ptr().cast::<c_void>()
    }
}

impl Drop for JsContextHostBridgeRef {
    fn drop(&mut self) {
        // SAFETY: this token is created only from the matching Rc::into_raw in
        // `install_into_bridge`, is not Clone, and is dropped exactly once by
        // the owning ScriptVm context state.
        unsafe {
            drop(Rc::from_raw(self.ptr.as_ptr()));
        }
        #[cfg(test)]
        {
            let count = self.bridge_ref_count.get();
            self.bridge_ref_count.set(
                count
                    .checked_sub(1)
                    .expect("V8 bridge ref-count test counter should not underflow"),
            );
        }
    }
}

impl JsContextHost {
    pub(crate) fn install_default_world_wrapper_cache_for_context(
        &self,
        context: v8::Local<'_, v8::Context>,
    ) {
        self.bridge.install_default_world_wrapper_cache(context);
    }

    /// Install this host into the V8 global bridge object and return the Rust
    /// owner token for the V8-side Rc ref-count.
    pub(crate) fn install_into_bridge<'s>(
        host_rc: &Rc<RefCell<JsContextHost>>,
        scope: &mut v8::PinScope<'s, '_>,
        global: v8::Local<'s, v8::Object>,
    ) -> Result<JsContextHostBridgeRef> {
        let host_ptr: *mut JsContextHost = (*host_rc).as_ptr();
        let host_lifecycle = unsafe { &*host_ptr }.context_host_lifecycle_handle();
        #[cfg(test)]
        let bridge_ref_count = unsafe { &*host_ptr }.bridge_ref_count.clone();
        #[cfg(not(test))]
        let bridge_ref = JsContextHostBridgeRef::new(Rc::into_raw(host_rc.clone()));
        #[cfg(test)]
        let bridge_ref =
            JsContextHostBridgeRef::new(Rc::into_raw(host_rc.clone()), bridge_ref_count);
        // The actual JsContextHost pointer for runtime_ptr_from_object compatibility.
        let bridge = unsafe { &mut (*host_ptr).bridge };
        bridge.install_global(scope, global, host_ptr, bridge_ref.as_external_ptr())?;
        crate::util::install_context_host_pointer_slot(
            scope.get_current_context(),
            host_ptr,
            host_lifecycle,
        );
        Ok(bridge_ref)
    }
}
