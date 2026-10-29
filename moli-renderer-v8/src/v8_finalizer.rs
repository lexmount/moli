use std::{cell::Cell, rc::Rc};

const FINALIZER_COMPACTION_INTERVAL: usize = 64;

type FinalizerCleanup = Box<dyn FnOnce()>;
type SharedFinalizerCleanup = Rc<Cell<Option<FinalizerCleanup>>>;

/// Owns V8 weak handles for exactly one page or worker context.
///
/// rusty_v8 drains guaranteed-finalizer callbacks before C++ finishes isolate
/// teardown. A weak handle that outlives its isolate can therefore leave C++
/// pointing at Rust `WeakData` after the finalizer annex has gone away. Every
/// entry in this registry is instead released while its owning isolate is still
/// alive.
#[derive(Default)]
pub(crate) struct V8FinalizerRegistry {
    entries: Vec<ContextOwnedV8Finalizer>,
    insertions_since_compaction: usize,
}

struct ContextOwnedV8Finalizer {
    // Keep the weak before the shared cleanup so field teardown resets the V8
    // handle before releasing the final copy of the once-cell.
    weak: v8::Weak<v8::Object>,
    cleanup: SharedFinalizerCleanup,
}

impl ContextOwnedV8Finalizer {
    fn new(
        scope: &mut v8::PinScope<'_, '_>,
        object: v8::Local<'_, v8::Object>,
        cleanup: FinalizerCleanup,
    ) -> Self {
        let cleanup = Rc::new(Cell::new(Some(cleanup)));
        let callback_cleanup = Rc::clone(&cleanup);
        let weak = v8::Weak::with_guaranteed_finalizer(
            scope,
            object,
            Box::new(move || run_cleanup(&callback_cleanup)),
        );
        Self { weak, cleanup }
    }
}

impl Drop for ContextOwnedV8Finalizer {
    fn drop(&mut self) {
        // `Weak::drop` cancels its V8 callback, so explicit context teardown
        // must run the resource cleanup first. If V8 already finalized the
        // object, the shared once-cell is empty and this is a no-op.
        run_cleanup(&self.cleanup);
    }
}

impl V8FinalizerRegistry {
    fn push(&mut self, entry: ContextOwnedV8Finalizer) -> Vec<ContextOwnedV8Finalizer> {
        self.entries.push(entry);
        self.insertions_since_compaction = self.insertions_since_compaction.saturating_add(1);
        if self.insertions_since_compaction >= FINALIZER_COMPACTION_INTERVAL {
            self.insertions_since_compaction = 0;
            return self
                .entries
                .extract_if(.., |entry| entry.weak.is_empty())
                .collect();
        }
        Vec::new()
    }

    pub(crate) fn clear_for_context_teardown(&mut self) {
        self.entries.clear();
        self.insertions_since_compaction = 0;
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

fn run_cleanup(cleanup: &SharedFinalizerCleanup) {
    if let Some(cleanup) = cleanup.take() {
        cleanup();
    }
}

/// Attach a finalizer to the page or worker context owning `object`.
///
/// Owner discovery happens before the weak handle is created. The registry is
/// then mutated only after the V8 call returns, so no `JsContextHost` or worker
/// `RefCell` borrow is held across a potentially re-entrant V8 operation.
pub(crate) fn track_context_owned_v8_finalizer(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    cleanup: impl FnOnce() + 'static,
) {
    let cleanup: FinalizerCleanup = Box::new(cleanup);

    let common = crate::common_runtime::CommonRuntimeState::from_scope(scope)
        .expect("V8 finalizer object must belong to a page or worker context");
    let entry = ContextOwnedV8Finalizer::new(scope, object, cleanup);
    let retired = common.finalizers.borrow_mut().push(entry);
    // Compaction can release a pending native cleanup as well. Keep those
    // drops outside the component borrow, matching explicit teardown.
    drop(retired);
}

#[cfg(test)]
mod tests {
    use super::*;

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
                std::ptr::NonNull::<crate::native_bridge::JsContextHost>::dangling().as_ptr(),
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

    #[test]
    fn context_teardown_runs_cleanup_exactly_once_before_isolate_drop() {
        crate::ensure_v8_for_test();
        let cleanup_count = Rc::new(Cell::new(0_u32));
        let mut isolate = v8::Isolate::new(Default::default());
        let mut registry = V8FinalizerRegistry::default();
        {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let object = v8::Object::new(scope);
            let cleanup_count_for_callback = Rc::clone(&cleanup_count);
            drop(registry.push(ContextOwnedV8Finalizer::new(
                scope,
                object,
                Box::new(move || {
                    cleanup_count_for_callback
                        .set(cleanup_count_for_callback.get().saturating_add(1));
                }),
            )));
            assert_eq!(registry.len(), 1);
        }

        registry.clear_for_context_teardown();
        assert_eq!(registry.len(), 0);
        assert_eq!(cleanup_count.get(), 1);
        registry.clear_for_context_teardown();
        assert_eq!(cleanup_count.get(), 1);
        drop(registry);
        drop(isolate);
    }
}
