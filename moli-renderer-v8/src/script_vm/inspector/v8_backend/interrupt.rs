use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use super::RendererInspectorSessionExecutorLocal;
use crate::{
    devtools::{
        ingress::{
            io::{RendererInspectorInterruptTarget, RendererInspectorIoOwnerWake},
            main::{RendererInspectorMainOwnerDispatch, RendererInspectorMainOwnerWake},
        },
        route::RendererInspectorSessionExecutorRouteId,
    },
    inspector_microtasks::with_scoped_inspector_microtasks,
};

thread_local! {
    static INSPECTOR_SESSION_EXECUTORS: RefCell<HashMap<RendererInspectorSessionExecutorRouteId, Weak<RendererInspectorSessionExecutorLocal>>> =
        RefCell::new(HashMap::new());
}

static NEXT_SESSION_EXECUTOR_ROUTE_ID: AtomicUsize = AtomicUsize::new(1);

pub(super) fn allocate_session_executor_route_id() -> usize {
    NEXT_SESSION_EXECUTOR_ROUTE_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("renderer Inspector session-executor route ID exhausted")
}

pub(super) fn register_session_executor(
    route_id: RendererInspectorSessionExecutorRouteId,
    executor: &Rc<RendererInspectorSessionExecutorLocal>,
) {
    let previous = INSPECTOR_SESSION_EXECUTORS.with(|executors| {
        executors
            .borrow_mut()
            .insert(route_id, Rc::downgrade(executor))
    });
    assert!(
        previous.is_none(),
        "renderer Inspector session-executor route IDs must be unique"
    );
}

pub(super) fn unregister_session_executor(route_id: RendererInspectorSessionExecutorRouteId) {
    let _ = INSPECTOR_SESSION_EXECUTORS.try_with(|executors| {
        executors.borrow_mut().remove(&route_id);
    });
}

fn session_executor(
    route_id: RendererInspectorSessionExecutorRouteId,
) -> Option<Rc<RendererInspectorSessionExecutorLocal>> {
    INSPECTOR_SESSION_EXECUTORS
        .try_with(|executors| executors.borrow().get(&route_id).and_then(Weak::upgrade))
        .ok()
        .flatten()
}

pub(super) unsafe extern "C" fn dispatch_inspector_interrupt(
    isolate: v8::UnsafeRawIsolatePtr,
    data: *mut std::ffi::c_void,
) {
    // SAFETY: every accepted request passes one `Arc::into_raw` reference to
    // V8, and V8 invokes its interrupt callback at most once for that request.
    // Reconstructing it here consumes exactly that callback-owned reference.
    let callback_target = unsafe { Arc::from_raw(data.cast::<RendererInspectorInterruptTarget>()) };
    let Some(session_executor) = session_executor(callback_target.route_id()) else {
        return;
    };
    let mut isolate_ptr = isolate;
    let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(&mut isolate_ptr) };
    with_scoped_inspector_microtasks(isolate, || {
        session_executor.dispatch_next_io_command_from_interrupt();
    });
}

pub(crate) fn dispatch_inspector_io_owner_wake(wake: RendererInspectorIoOwnerWake) {
    let Some(session_executor) = session_executor(wake.route_id()) else {
        return;
    };
    let isolate = unsafe { &mut *session_executor.isolate.get() };
    let isolate = unsafe { v8::Isolate::ref_from_raw_isolate_ptr_mut(isolate) };
    with_scoped_inspector_microtasks(isolate, || {
        session_executor.dispatch_next_io_command_from_owner();
    });
}

pub(crate) fn dispatch_inspector_main_owner_wake(
    wake: RendererInspectorMainOwnerWake,
) -> Option<RendererInspectorMainOwnerDispatch> {
    session_executor(wake.route_id())
        .and_then(|session_executor| session_executor.claim_next_main_command_from_owner())
}
