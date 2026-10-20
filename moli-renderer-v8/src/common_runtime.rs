//! Shared owner-local Web API state and its Context entry point.
//!
//! A Context locates its owner through a weak binding. Window Realm tokens and
//! Worker run state remain the authoritative execution identities; retaining a
//! Context must not independently root its host or its callback graph.

mod callback_registry;

pub(crate) use callback_registry::{
    CallbackRegistrationId, CallbackRegistrationKind, CallbackResidence,
};

use callback_registry::CallbackRegistry;

use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{
    native_bridge::JsContextHost,
    text_codec::TextCodecStore,
    util::{ContextHostLifecycle, page_context_is_disconnected},
    v8_finalizer::V8FinalizerRegistry,
    worker::WorkerGlobalState,
};

#[derive(Default)]
pub(crate) struct CommonRuntimeState {
    pub(crate) finalizers: RefCell<V8FinalizerRegistry>,
    pub(crate) text_codecs: TextCodecStore,
    pub(crate) callbacks: CallbackRegistry,
}

impl CommonRuntimeState {
    /// Resolve native components without borrowing the Window or Worker
    /// host. Retained backing state and callback execution permission remain
    /// separate: asynchronous callbacks must still revalidate their identities.
    pub(crate) fn from_scope(scope: &mut v8::PinScope<'_, '_>) -> Option<Rc<Self>> {
        Self::from_context(scope.get_current_context())
    }

    /// Native components for an explicit target Realm. Borrowed methods must
    /// resolve the receiver's owner rather than the method's current Context.
    pub(crate) fn from_context(context: v8::Local<'_, v8::Context>) -> Option<Rc<Self>> {
        let binding = context.get_slot::<CommonRuntimeBinding>()?;
        let (runtime, _) = binding.resolve_owner(context)?;
        Some(runtime)
    }

    pub(crate) fn clear_for_context_teardown(&self) {
        self.callbacks.clear_for_context_teardown();
        // Cleanup may reenter native components. Drop the component borrow
        // before executing any cleanup, just as registration releases owner
        // borrows before allocating a V8 weak handle.
        let mut finalizers = std::mem::take(&mut *self.finalizers.borrow_mut());
        finalizers.clear_for_context_teardown();
    }

    #[cfg(test)]
    pub(crate) fn finalizer_count(&self) -> usize {
        self.finalizers.borrow().len()
    }
}

pub(crate) struct CommonRuntimeBinding {
    common: Weak<CommonRuntimeState>,
    owner: RuntimeOwner,
}

enum RuntimeOwner {
    Window {
        host_ptr: *mut JsContextHost,
        lifecycle: Weak<Cell<ContextHostLifecycle>>,
    },
    Worker {
        state: Weak<RefCell<WorkerGlobalState>>,
        termination_requested: Arc<AtomicBool>,
    },
}

#[derive(Clone, Copy)]
pub(crate) enum RuntimeExecutionContext {
    Window(*mut JsContextHost),
    Worker,
}

impl CommonRuntimeBinding {
    pub(crate) fn install_window(
        context: v8::Local<'_, v8::Context>,
        common: &Rc<CommonRuntimeState>,
        host_ptr: *mut JsContextHost,
        lifecycle: &Rc<Cell<ContextHostLifecycle>>,
    ) {
        if let Some(existing) = context.get_slot::<Self>() {
            assert!(
                Weak::ptr_eq(&existing.common, &Rc::downgrade(common))
                    && matches!(existing.owner, RuntimeOwner::Window { host_ptr: installed, .. } if installed == host_ptr),
                "a Context's common runtime owner must not be rebound"
            );
            return;
        }
        let previous = context.set_slot(Rc::new(Self {
            common: Rc::downgrade(common),
            owner: RuntimeOwner::Window {
                host_ptr,
                lifecycle: Rc::downgrade(lifecycle),
            },
        }));
        debug_assert!(previous.is_none());
    }

    pub(crate) fn install_worker(
        context: v8::Local<'_, v8::Context>,
        state: &Rc<RefCell<WorkerGlobalState>>,
    ) {
        let (common, termination_requested) = {
            let state = state.borrow();
            (
                Rc::clone(&state.common),
                Arc::clone(&state.termination_requested),
            )
        };
        let previous = context.set_slot(Rc::new(Self {
            common: Rc::downgrade(&common),
            owner: RuntimeOwner::Worker {
                state: Rc::downgrade(state),
                termination_requested,
            },
        }));
        assert!(
            previous.is_none(),
            "a Worker Context must have one common runtime binding"
        );
    }

    /// Resolve the current Context and its owner once for immediate native
    /// dispatch. This result is not permission to invoke a later callback:
    /// invocation must revalidate the binding through `callback_execution_context`.
    pub(crate) fn from_scope(
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<(Rc<Self>, RuntimeExecutionContext)> {
        Self::from_context(scope.get_current_context())
    }

    pub(crate) fn from_context(
        context: v8::Local<'_, v8::Context>,
    ) -> Option<(Rc<Self>, RuntimeExecutionContext)> {
        let binding = context.get_slot::<Self>()?;
        let (_, owner) = binding.resolve_owner(context)?;
        Some((binding, owner))
    }

    pub(crate) fn runtime(&self) -> Option<Rc<CommonRuntimeState>> {
        self.common.upgrade()
    }

    fn resolve_owner(
        &self,
        context: v8::Local<'_, v8::Context>,
    ) -> Option<(Rc<CommonRuntimeState>, RuntimeExecutionContext)> {
        let runtime = self.runtime()?;
        let owner = match &self.owner {
            RuntimeOwner::Window {
                host_ptr,
                lifecycle,
            } => {
                if page_context_is_disconnected(context)
                    || lifecycle.upgrade()?.get() == ContextHostLifecycle::Destroyed
                {
                    return None;
                }
                RuntimeExecutionContext::Window(*host_ptr)
            }
            RuntimeOwner::Worker { state, .. } => {
                state.upgrade()?;
                RuntimeExecutionContext::Worker
            }
        };
        Some((runtime, owner))
    }

    pub(crate) fn execution_context(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<RuntimeExecutionContext> {
        let context = scope.get_current_context();
        let installed = context.get_slot::<Self>()?;
        if !std::ptr::eq(self, installed.as_ref()) {
            return None;
        }
        self.resolve_owner(context).map(|(_, owner)| owner)
    }

    /// Recheck the exact Context and owner immediately before author code.
    /// Worker termination is checked in the same pass as owner resolution;
    /// `close()` remains the task owner's separate delivery policy.
    pub(crate) fn callback_execution_context(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
    ) -> Option<RuntimeExecutionContext> {
        let context = self.execution_context(scope)?;
        if let RuntimeOwner::Worker {
            termination_requested,
            ..
        } = &self.owner
            && termination_requested.load(Ordering::Acquire)
        {
            return None;
        }
        Some(context)
    }

    pub(crate) fn window_host_ptr(&self) -> Option<*mut JsContextHost> {
        self.runtime()?;
        let RuntimeOwner::Window {
            host_ptr,
            lifecycle,
        } = &self.owner
        else {
            return None;
        };
        (lifecycle.upgrade()?.get() != ContextHostLifecycle::Destroyed).then_some(*host_ptr)
    }

    pub(crate) fn worker_state(&self) -> Option<Rc<RefCell<WorkerGlobalState>>> {
        self.runtime()?;
        let RuntimeOwner::Worker { state, .. } = &self.owner else {
            return None;
        };
        state.upgrade()
    }

    pub(crate) fn worker_can_deliver_task(&self) -> bool {
        self.runtime().is_some()
            && match &self.owner {
                RuntimeOwner::Worker { state, .. } => state
                    .upgrade()
                    .is_some_and(|state| state.borrow().callback_task_delivery_allowed()),
                RuntimeOwner::Window { .. } => false,
            }
    }
}

#[cfg(test)]
mod benchmarks;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_bindings_share_components_without_rooting_or_confusing_owners() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let first = v8::Context::new(scope, Default::default());
        let second = v8::Context::new(scope, Default::default());
        let other = v8::Context::new(scope, Default::default());
        let common = Rc::new(CommonRuntimeState::default());
        let other_common = Rc::new(CommonRuntimeState::default());
        let lifecycle = Rc::new(Cell::new(ContextHostLifecycle::Active));
        let other_lifecycle = Rc::new(Cell::new(ContextHostLifecycle::Active));
        // This test exercises component ownership and Context identity only.
        // The opaque host address is never dereferenced by a host API.
        let host = std::ptr::NonNull::<JsContextHost>::dangling().as_ptr();
        CommonRuntimeBinding::install_window(first, &common, host, &lifecycle);
        CommonRuntimeBinding::install_window(second, &common, host, &lifecycle);
        CommonRuntimeBinding::install_window(other, &other_common, host, &other_lifecycle);
        let first_binding = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let (binding, owner) = CommonRuntimeBinding::from_scope(scope).unwrap();
            assert!(matches!(owner, RuntimeExecutionContext::Window(ptr) if ptr == host));
            assert!(Rc::ptr_eq(&binding.runtime().unwrap(), &common));
            assert!(Rc::ptr_eq(
                &CommonRuntimeState::from_scope(scope).unwrap(),
                &common
            ));
            assert!(binding.callback_execution_context(scope).is_some());
            binding
        };
        {
            let scope = &mut v8::ContextScope::new(scope, second);
            let (binding, _) = CommonRuntimeBinding::from_scope(scope).unwrap();
            assert!(Rc::ptr_eq(&binding.runtime().unwrap(), &common));
            assert!(!Rc::ptr_eq(&binding, &first_binding));
            assert!(first_binding.execution_context(scope).is_none());
            assert!(first_binding.callback_execution_context(scope).is_none());
        }
        {
            let scope = &mut v8::ContextScope::new(scope, other);
            let (binding, _) = CommonRuntimeBinding::from_scope(scope).unwrap();
            assert!(!Rc::ptr_eq(&binding.runtime().unwrap(), &common));
            assert!(Rc::ptr_eq(
                &CommonRuntimeState::from_context(first).unwrap(),
                &common
            ));
            other_lifecycle.set(ContextHostLifecycle::Destroyed);
            assert!(CommonRuntimeBinding::from_scope(scope).is_none());
            assert!(CommonRuntimeState::from_scope(scope).is_none());
            assert!(CommonRuntimeState::from_context(other).is_none());
            // A previously resolved owner is not a cached execution permit.
            assert!(binding.callback_execution_context(scope).is_none());
        }
        drop(common);
        let scope = &mut v8::ContextScope::new(scope, first);
        assert!(CommonRuntimeBinding::from_scope(scope).is_none());
        assert!(first_binding.runtime().is_none());
        assert!(first_binding.callback_execution_context(scope).is_none());
    }

    #[test]
    fn bound_window_host_lookup_uses_native_owner_and_rejects_destroyed_owner() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let common = Rc::new(CommonRuntimeState::default());
        let lifecycle = Rc::new(Cell::new(ContextHostLifecycle::Active));
        // Owner lookup must not dereference this opaque test address.
        let host = std::ptr::NonNull::<JsContextHost>::dangling().as_ptr();
        CommonRuntimeBinding::install_window(context, &common, host, &lifecycle);
        let scope = &mut v8::ContextScope::new(scope, context);
        let source = v8::String::new(
            scope,
            "Object.defineProperty(globalThis, '__moliNativeBridge', {get() { throw new Error('author bridge getter'); }});",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        crate::script_execution::execute_compiled_script(scope, script).unwrap();
        assert_eq!(
            crate::util::context_host_ptr_from_global_bridge(scope),
            Some(host)
        );
        lifecycle.set(ContextHostLifecycle::Destroyed);
        assert!(crate::util::context_host_ptr_from_global_bridge(scope).is_none());
    }

    #[test]
    fn missing_context_binding_rejects_callback_before_reading_its_carrier() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let object = v8::Object::new(scope);
        assert!(CommonRuntimeBinding::from_scope(scope).is_none());
        assert!(
            crate::webidl_callback_execution::PreparedWebIdlCallbackExecution::from_traced(
                scope, object
            )
            .is_none()
        );
    }
}
