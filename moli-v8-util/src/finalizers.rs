use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

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
pub struct V8FinalizerRegistry {
    entries: Vec<ContextOwnedV8Finalizer>,
    insertions_since_compaction: usize,
}

struct ContextOwnedV8Finalizer {
    // Keep the weak before the shared cleanup so field teardown resets the V8
    // handle before releasing the final copy of the once-cell.
    weak: v8::Weak<v8::Value>,
    cleanup: SharedFinalizerCleanup,
}

impl ContextOwnedV8Finalizer {
    fn new(
        scope: &mut v8::PinScope<'_, '_>,
        object: v8::Local<'_, v8::Value>,
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
    /// Register a weak handle without holding its registry borrow across V8.
    /// Retired cleanup callbacks may reenter the resource owner.
    pub fn track<'s>(
        registry: &RefCell<Self>,
        scope: &mut v8::PinScope<'_, '_>,
        object: impl Into<v8::Local<'s, v8::Value>>,
        cleanup: impl FnOnce() + 'static,
    ) {
        let entry = ContextOwnedV8Finalizer::new(scope, object.into(), Box::new(cleanup));
        let retired = registry.borrow_mut().push(entry);
        drop(retired);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

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

    pub fn clear_for_context_teardown(&mut self) {
        self.entries.clear();
        self.insertions_since_compaction = 0;
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

fn run_cleanup(cleanup: &SharedFinalizerCleanup) {
    if let Some(cleanup) = cleanup.take() {
        cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_teardown_runs_cleanup_exactly_once_before_isolate_drop() {
        moli_v8_test_util::ensure_v8();
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
                object.into(),
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
