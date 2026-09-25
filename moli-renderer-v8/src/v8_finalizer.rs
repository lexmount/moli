pub(crate) use moli_v8_util::V8FinalizerRegistry;

/// Attach a finalizer to the page or worker context owning `object`.
///
/// Owner discovery happens before the weak handle is created. The registry is
/// then mutated only after the V8 call returns, so no `PageScriptHost` or worker
/// `RefCell` borrow is held across a potentially re-entrant V8 operation.
pub(crate) fn track_context_owned_v8_finalizer(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    cleanup: impl FnOnce() + 'static,
) {
    let common = crate::common_runtime::CommonRuntimeState::from_scope(scope)
        .expect("V8 finalizer object must belong to a page or worker context");
    V8FinalizerRegistry::track(&common.finalizers, scope, object, cleanup);
}

pub(crate) fn track_context_owned_external_finalizer(
    scope: &mut v8::PinScope<'_, '_>,
    external: v8::Local<'_, v8::External>,
    cleanup: impl FnOnce() + 'static,
) {
    let common = crate::common_runtime::CommonRuntimeState::from_scope(scope)
        .expect("V8 finalizer must belong to a page or worker context");
    V8FinalizerRegistry::track(&common.finalizers, scope, external, cleanup);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn common_runtime_teardown_releases_borrow_before_running_cleanup() {
        use crate::common_runtime::{CommonRuntimeBinding, CommonRuntimeState};
        crate::ensure_v8_for_test();
        let common = Rc::new(CommonRuntimeState::default());
        let lifecycle = Rc::new(Cell::new(crate::util::ContextHostLifecycle::Active));
        let cleanup_count = Rc::new(Cell::new(0));
        let mut isolate = v8::Isolate::new(Default::default());
        {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            // Finalizer registration uses only the shared component, so this
            // owner identity does not need to dereference a Document host.
            CommonRuntimeBinding::install_window(
                context,
                &common,
                std::ptr::NonNull::<crate::native_bridge::PageScriptHost>::dangling().as_ptr(),
                &lifecycle,
            );
            let scope = &mut v8::ContextScope::new(scope, context);
            let object = v8::Object::new(scope);
            let owner = Rc::downgrade(&common);
            let count = Rc::clone(&cleanup_count);
            track_context_owned_v8_finalizer(scope, object, move || {
                assert_eq!(owner.upgrade().unwrap().finalizer_count(), 0);
                count.set(count.get() + 1);
            });
            assert_eq!(common.finalizer_count(), 1);
        }
        common.clear_for_context_teardown();
        common.clear_for_context_teardown();
        assert_eq!(common.finalizer_count(), 0);
        assert_eq!(cleanup_count.get(), 1);
        drop(common);
        drop(isolate);
    }
}
