use super::*;
use crate::util::set_private_value;
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

const XHR_CONTEXT_DESTROYED_SLOT: &str = "__lmXhrContextDestroyed";

/// Realm-owned lifecycle observers must not keep unused XHR wrappers alive.
/// Only explicit realm teardown disposes live objects; GC prunes weak metadata.
struct WindowXhrLifecycle {
    retired: bool,
    next_id: u64,
    context: Option<v8::Weak<v8::Context>>,
    objects: HashMap<u64, v8::Weak<v8::Object>>,
}

/// A non-owning observer tagged with the captured LocalWindow address. It
/// neither roots the realm nor authorizes operations on a replacement Window.
pub(crate) struct WindowXhrLifecycleObserver {
    owner: crate::native_bridge::WindowExecutionContextOwner,
    registry: Weak<RefCell<WindowXhrLifecycle>>,
}

impl WindowXhrLifecycleObserver {
    pub(crate) fn owner(&self) -> crate::native_bridge::WindowExecutionContextOwner {
        self.owner
    }

    pub(crate) fn retire(self, scope: &mut v8::PinScope<'_, '_>) {
        if let Some(registry) = self.registry.upgrade() {
            retire_window_xhr_registry(scope, &registry);
        }
    }
}

pub(super) fn register_window_xhr(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    binding: &crate::native_bridge::WindowExecutionContextBinding,
) {
    let context = scope.get_current_context();
    let registry = context
        .get_slot::<RefCell<WindowXhrLifecycle>>()
        .unwrap_or_else(|| {
            let registry = Rc::new(RefCell::new(WindowXhrLifecycle {
                retired: false,
                next_id: 0,
                context: Some(v8::Weak::new(scope, context)),
                objects: HashMap::new(),
            }));
            let _ = context.set_slot(registry.clone());
            if let Some(host) = context_host_ptr_from_global_bridge(scope) {
                // SAFETY: the constructor's captured binding belongs to this
                // entered context and its live host bridge.
                unsafe { &mut *host }.register_window_xhr_lifecycle_observer(
                    binding.realm_token(),
                    WindowXhrLifecycleObserver {
                        owner: binding.owner(),
                        registry: Rc::downgrade(&registry),
                    },
                );
            }
            registry
        });
    let id = {
        let mut state = registry.borrow_mut();
        if state.retired {
            drop(state);
            dispose_window_xhr(scope, xhr);
            return;
        }
        let id = state.next_id;
        state.next_id = id.checked_add(1).expect("Window XHR lifecycle ID overflow");
        id
    };
    let registry_weak = Rc::downgrade(&registry);
    // No registry borrow crosses a V8 operation: another weak finalizer can
    // run during GC. The finalizer itself owns neither the realm nor the XHR.
    let weak = v8::Weak::with_finalizer(
        scope,
        xhr,
        Box::new(move |_| {
            if let Some(registry) = registry_weak.upgrade() {
                registry.borrow_mut().objects.remove(&id);
            }
        }),
    );
    registry.borrow_mut().objects.insert(id, weak);
}

pub(crate) fn retire_window_xhrs_in_current_context(scope: &mut v8::PinScope<'_, '_>) {
    let Some(registry) = scope
        .get_current_context()
        .get_slot::<RefCell<WindowXhrLifecycle>>()
    else {
        return;
    };
    retire_window_xhr_registry(scope, &registry);
}

fn retire_window_xhr_registry(
    scope: &mut v8::PinScope<'_, '_>,
    registry: &Rc<RefCell<WindowXhrLifecycle>>,
) {
    let (context, objects) = {
        let mut state = registry.borrow_mut();
        state.retired = true;
        (state.context.take(), std::mem::take(&mut state.objects))
    };
    let Some(context) = context.and_then(|context| context.to_local(scope)) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    for weak in objects.into_values() {
        if let Some(xhr) = weak.to_local(scope) {
            dispose_window_xhr(scope, xhr);
        }
    }
}

fn dispose_window_xhr(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    super::delivery::clear_xhr_request_timers(scope, xhr);
    set_xhr_state_bool(scope, xhr, XHR_ABORTED_SLOT, true);
    set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, false);
    set_xhr_state_bool(scope, xhr, XHR_UPLOAD_IN_PROGRESS_SLOT, false);
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);
    set_xhr_state_string(scope, xhr, XHR_REQUEST_HEADERS_SLOT, "[]");
    // ContextDestroyed clears response state without transitioning readyState
    // or invoking author callbacks. Transport cancellation remains host-owned.
    reset_xhr_response(scope, xhr);
    let destroyed = v8::Boolean::new(scope, true);
    set_private_value(scope, xhr, XHR_CONTEXT_DESTROYED_SLOT, destroyed.into());
    if let Some(upload) = xhr_upload_object(scope, xhr) {
        set_private_value(scope, upload, XHR_CONTEXT_DESTROYED_SLOT, destroyed.into());
    }
}

pub(super) fn xhr_context_is_destroyed(
    scope: &mut v8::PinScope<'_, '_>,
    target: v8::Local<'_, v8::Object>,
) -> bool {
    let target = v8::Local::new(scope, target);
    get_private_value(scope, target, XHR_CONTEXT_DESTROYED_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}
