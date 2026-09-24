use std::ptr::NonNull;

use crate::{
    dom::native::{DomHost, DomMutationEffects, NativeNodeId},
    native_bridge::{JsContextHost, RuntimeObservableContextToken, WindowExecutionContextOwner},
};
use moli_webidl_callback::WebIdlCallbackFunction;

use super::{
    IntersectionMutationPlan, IntersectionObserverOptions, MutationObserverOptions,
    ObserverMutationPlan, ObserverStore, build_intersection_entries_array,
    build_mutation_records_array,
    callback::{
        ObserverCallback, ObserverCallbackBinding, ObserverCallbackId, PreparedObserverCallback,
    },
    intersection::{self, IntersectionCheckBatch},
    invoke_intersection_deliveries, invoke_mutation_deliveries, node_is_intersection_root,
    node_is_intersection_target,
    schedule::{self, ObserverTask},
    target_is_intersection_observable,
};

/// V8-traced values for one callback plus the exact identity binding that
/// authorizes their use.
///
/// Keeping these fields together prevents observer APIs from swapping
/// relevant/incumbent context anchors when preparing a delivery.
pub(crate) struct ObserverCallbackResidence<'s> {
    id: ObserverCallbackId,
    callback: v8::Local<'s, v8::Object>,
    relevant_global: v8::Local<'s, v8::Object>,
    incumbent_global: v8::Local<'s, v8::Object>,
}

impl<'s> ObserverCallbackResidence<'s> {
    pub(crate) fn from_parts(
        id: ObserverCallbackId,
        callback: v8::Local<'s, v8::Object>,
        relevant_global: v8::Local<'s, v8::Object>,
        incumbent_global: v8::Local<'s, v8::Object>,
    ) -> Self {
        Self {
            id,
            callback,
            relevant_global,
            incumbent_global,
        }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ObserverCallbackId,
        v8::Local<'s, v8::Object>,
        v8::Local<'s, v8::Object>,
        v8::Local<'s, v8::Object>,
    ) {
        (
            self.id,
            self.callback,
            self.relevant_global,
            self.incumbent_global,
        )
    }
}

pub(crate) fn register_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    observer: v8::Local<'s, v8::Object>,
    callback: WebIdlCallbackFunction,
) -> ObserverCallbackResidence<'s> {
    let mut access = ObserverHostAccess::new(host_ptr);
    let binding =
        access.read(|host| ObserverCallbackBinding::new(scope, host, observer, &callback));
    let registry = access.store(|store| store.callback_registry.clone());
    let id = registry.register(binding);
    let callback_value = callback.value(scope);
    let relevant_global = callback.relevant_context(scope).global(scope);
    // Preserve the exact conversion-time incumbent context rather than
    // rediscovering it from the callback object.
    let incumbent_global = callback.incumbent_context(scope).global(scope);
    let callback = v8::Local::<v8::Object>::try_from(callback_value)
        .expect("a Web IDL callback function must be an object");
    crate::v8_finalizer::track_context_owned_v8_finalizer(
        scope,
        observer,
        registry.finalizer_cleanup(id),
    );
    ObserverCallbackResidence {
        id,
        callback,
        relevant_global,
        incumbent_global,
    }
}

pub(crate) fn prepare_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    residence: ObserverCallbackResidence<'s>,
) -> Option<PreparedObserverCallback> {
    let ObserverCallbackResidence {
        id,
        callback,
        relevant_global,
        incumbent_global,
    } = residence;
    let mut access = ObserverHostAccess::new(host_ptr);
    let registry = access.store(|store| store.callback_registry.clone());
    access
        .read(|host| registry.prepare(scope, host, id, callback, relevant_global, incumbent_global))
}

pub(crate) fn callback_is_current(host_ptr: *mut JsContextHost, id: ObserverCallbackId) -> bool {
    let mut access = ObserverHostAccess::new(host_ptr);
    let registry = access.store(|store| store.callback_registry.clone());
    access.read(|host| registry.is_current(host, id))
}

pub(crate) fn activate_performance_observer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    id: ObserverCallbackId,
    observer: v8::Local<'s, v8::Object>,
) -> bool {
    let mut access = ObserverHostAccess::new(host_ptr);
    let registry = access.store(|store| store.callback_registry.clone());
    access.read(|host| registry.activate_performance_observer(scope, host, id, observer))
}

pub(crate) fn deactivate_performance_observer_callback(
    host_ptr: *mut JsContextHost,
    id: ObserverCallbackId,
) -> bool {
    let registry = ObserverHostAccess::new(host_ptr).store(|store| store.callback_registry.clone());
    registry.deactivate_performance_observer(id)
}

pub(crate) fn active_performance_observer_callbacks<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
) -> Vec<v8::Local<'s, v8::Object>> {
    let registry = ObserverHostAccess::new(host_ptr).store(|store| store.callback_registry.clone());
    registry.active_performance_observers(scope)
}

#[cfg(test)]
pub(crate) fn callback_binding_count_for_test(host: &mut JsContextHost) -> usize {
    host.observers_mut(&OBSERVER_STORE_ACCESS)
        .callback_registry
        .len()
}

pub(crate) struct ObserverStoreAccessToken {
    _private: (),
}

const OBSERVER_STORE_ACCESS: ObserverStoreAccessToken = ObserverStoreAccessToken { _private: () };

/// The only raw `JsContextHost` reborrow boundary in the observer runtime.
///
/// Each operation returns an owned value (`bool`, handles, options, entries, or
/// a check batch). The higher-ranked closure prevents a host reference from
/// escaping a phase, while `&mut self` prevents nested reborrows through this
/// façade. No closure passed here may call page JavaScript or a V8 callback.
struct ObserverHostAccess {
    host: NonNull<JsContextHost>,
}

impl ObserverHostAccess {
    fn new(host_ptr: *mut JsContextHost) -> Self {
        Self {
            host: NonNull::new(host_ptr).expect("observer host pointer must be non-null"),
        }
    }

    fn read<R>(&mut self, read: impl for<'host> FnOnce(&'host JsContextHost) -> R) -> R {
        // SAFETY: `ObserverHostAccess` stays private to this module. Its
        // higher-ranked closure cannot return a reference tied to the host,
        // and `&mut self` prevents a nested access while this borrow is live.
        unsafe { read(self.host.as_ref()) }
    }

    fn mutate<R>(&mut self, mutate: impl for<'host> FnOnce(&'host mut JsContextHost) -> R) -> R {
        // SAFETY: see `read`. Every call ends before another access phase or
        // any page/V8 callback is entered.
        unsafe { mutate(self.host.as_mut()) }
    }

    fn store<R>(&mut self, mutate: impl for<'store> FnOnce(&'store mut ObserverStore) -> R) -> R {
        self.mutate(|host| mutate(host.observers_mut(&OBSERVER_STORE_ACCESS)))
    }
}

fn request_task_with_access(
    access: &mut ObserverHostAccess,
    scope: &mut v8::PinScope<'_, '_>,
    task: ObserverTask,
) {
    let should_enqueue = access.store(|store| store.request_task(task));
    if should_enqueue && !schedule::enqueue(scope, task) {
        access.store(|store| store.cancel_task(task));
    }
}

pub(super) fn init_mutation_observer(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    observer: v8::Local<'_, v8::Object>,
    callback: WebIdlCallbackFunction,
) {
    let mut access = ObserverHostAccess::new(host_ptr);
    let callback = access.read(|host| ObserverCallback::new(scope, host, observer, callback));
    access.store(|store| {
        store.init_mutation_observer(scope, observer, callback);
    });
}

pub(super) fn observe_mutation_target(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    observer: v8::Local<'_, v8::Object>,
    target: NativeNodeId,
    options: MutationObserverOptions,
) {
    let mut access = ObserverHostAccess::new(host_ptr);
    let enabled = access.store(|store| {
        let _ = store.observe_mutation_target(scope, observer, target, options);
        store.has_active_mutation_observation()
    });
    access.read(|host| {
        host.dom_host()
            .set_mutation_observer_records_enabled(enabled)
    });
}

pub(super) fn disconnect_mutation_observer(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    observer: v8::Local<'_, v8::Object>,
) {
    let mut access = ObserverHostAccess::new(host_ptr);
    let enabled = access.store(|store| {
        store.disconnect_mutation_observer(scope, observer);
        store.has_active_mutation_observation()
    });
    access.read(|host| {
        host.dom_host()
            .set_mutation_observer_records_enabled(enabled)
    });
}

pub(super) fn take_mutation_records<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    observer: v8::Local<'_, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let records = ObserverHostAccess::new(host_ptr)
        .store(|store| store.take_mutation_records(scope, observer))?;
    Some(build_mutation_records_array(scope, host_ptr, &records))
}

pub(crate) fn queue_mutation_records(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    dom_host: &DomHost,
    effects: &DomMutationEffects,
) {
    let mut access = ObserverHostAccess::new(host_ptr);
    let ObserverMutationPlan {
        queue_mutation_delivery,
        intersection,
    } = access.store(|store| store.queue_mutation_records(dom_host, effects));

    if queue_mutation_delivery {
        request_task_with_access(&mut access, scope, ObserverTask::MutationDelivery);
    }
    match intersection {
        IntersectionMutationPlan::None => {}
        IntersectionMutationPlan::CheckNow => {
            queue_intersection_checks_with_dom(&mut access, scope, dom_host)
        }
        IntersectionMutationPlan::ScheduleCheck => {
            request_task_with_access(&mut access, scope, ObserverTask::IntersectionCheck)
        }
    }
    crate::context_bootstrap::queue_resize_observer_checks(scope);
}

pub(crate) fn coalesce_child_list_replacement_records(
    host_ptr: *mut JsContextHost,
    target: NativeNodeId,
    added_nodes: &[NativeNodeId],
    removed_nodes: &[NativeNodeId],
    previous_sibling: Option<NativeNodeId>,
    next_sibling: Option<NativeNodeId>,
) {
    ObserverHostAccess::new(host_ptr).store(|store| {
        store.coalesce_child_list_replacement_records(
            target,
            added_nodes,
            removed_nodes,
            previous_sibling,
            next_sibling,
        );
    });
}

pub(super) fn flush_mutation_observers(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    let mut access = ObserverHostAccess::new(host_ptr);
    let deliveries = access.mutate(|host| {
        host.begin_mutation_observer_delivery();
        let store = host.observers_mut(&OBSERVER_STORE_ACCESS);
        store.begin_task(ObserverTask::MutationDelivery);
        store.collect_mutation_deliveries(scope)
    });

    // `access` holds no host reference here. The callback can safely call
    // observe(), disconnect(), takeRecords(), or mutate the DOM reentrantly.
    invoke_mutation_deliveries(scope, host_ptr, deliveries);

    access.mutate(JsContextHost::end_mutation_observer_delivery);
    dispatch_pending_slotchange_events(&mut access, scope, host_ptr);
}

pub(crate) fn flush_slotchange_microtask(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    let mut access = ObserverHostAccess::new(host_ptr);
    if access.read(JsContextHost::has_scheduled_mutation_delivery) {
        access.mutate(|host| host.defer_slotchange_flush(scope));
        return;
    }
    dispatch_pending_slotchange_events(&mut access, scope, host_ptr);
}

fn dispatch_pending_slotchange_events(
    access: &mut ObserverHostAccess,
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    let slots = access.mutate(JsContextHost::take_pending_slotchange_slots);
    for slot in slots {
        if !access.read(|host| host.dom_host().is_html_element_named(slot, "slot")) {
            continue;
        }
        let Some(event) = crate::native_bridge::element::construct_simple_event(
            scope,
            "slotchange",
            true,
            false,
            false,
        ) else {
            continue;
        };
        // No host reference is live while event listeners execute.
        let _ = crate::native_bridge::element::dispatch_public_event(scope, host_ptr, slot, event);
    }
    if !access.read(JsContextHost::has_scheduled_mutation_delivery) {
        access.mutate(|host| host.promote_deferred_slotchange_events(scope));
    }
}

pub(super) fn is_intersection_root(host_ptr: *mut JsContextHost, root: NativeNodeId) -> bool {
    ObserverHostAccess::new(host_ptr).read(|host| node_is_intersection_root(host.dom_host(), root))
}

pub(super) fn init_intersection_observer(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    observer: v8::Local<'_, v8::Object>,
    callback: WebIdlCallbackFunction,
    options: IntersectionObserverOptions,
) {
    let mut access = ObserverHostAccess::new(host_ptr);
    let callback = access.read(|host| ObserverCallback::new(scope, host, observer, callback));
    access.store(|store| {
        store.init_intersection_observer(scope, observer, callback, options);
    });
}

pub(crate) fn retire_execution_context_owner(
    host: &mut JsContextHost,
    owner: WindowExecutionContextOwner,
) -> usize {
    let (retired, mutation_records_enabled) = {
        let store = host.observers_mut(&OBSERVER_STORE_ACCESS);
        let retired = store.retire_execution_context_owner(owner);
        (retired, store.has_active_mutation_observation())
    };
    host.dom_host()
        .set_mutation_observer_records_enabled(mutation_records_enabled);
    retired
}

pub(crate) fn retire_context_token(
    host: &mut JsContextHost,
    context_token: RuntimeObservableContextToken,
) -> usize {
    let (retired, mutation_records_enabled) = {
        let store = host.observers_mut(&OBSERVER_STORE_ACCESS);
        let retired = store.retire_context_token(context_token);
        (retired, store.has_active_mutation_observation())
    };
    host.dom_host()
        .set_mutation_observer_records_enabled(mutation_records_enabled);
    retired
}

pub(super) fn observe_intersection_target(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    observer: v8::Local<'_, v8::Object>,
    target: NativeNodeId,
) -> bool {
    let mut access = ObserverHostAccess::new(host_ptr);
    if !access.read(|host| node_is_intersection_target(host.dom_host(), target)) {
        return false;
    }
    let options = access.store(|store| store.intersection_observe_target(scope, observer, target));
    let should_check = options.as_ref().is_some_and(|options| {
        access.read(|host| target_is_intersection_observable(host.dom_host(), target, options))
    });
    if should_check {
        request_task_with_access(&mut access, scope, ObserverTask::IntersectionCheck);
    }
    true
}

pub(super) fn unobserve_intersection_target(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    observer: v8::Local<'_, v8::Object>,
    target: NativeNodeId,
) -> bool {
    let mut access = ObserverHostAccess::new(host_ptr);
    if !access.read(|host| node_is_intersection_target(host.dom_host(), target)) {
        return false;
    }
    access.store(|store| {
        store.intersection_unobserve_target(scope, observer, target);
    });
    true
}

pub(super) fn disconnect_intersection_observer(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    observer: v8::Local<'_, v8::Object>,
) {
    ObserverHostAccess::new(host_ptr).store(|store| {
        store.disconnect_intersection_observer(scope, observer);
    });
}

pub(super) fn take_intersection_records<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    observer: v8::Local<'_, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let (entries, options) = ObserverHostAccess::new(host_ptr)
        .store(|store| store.take_intersection_records(scope, observer))?;
    Some(build_intersection_entries_array(
        scope, host_ptr, &options, &entries,
    ))
}

pub(crate) fn queue_intersection_checks(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    let mut access = ObserverHostAccess::new(host_ptr);
    let Some(batch) = access.store(ObserverStore::take_intersection_check_batch) else {
        return;
    };
    let Ok(completed) = access.read(|runtime| {
        intersection::compute_intersection_check_batch(runtime, runtime.dom_host(), batch)
    }) else {
        return;
    };
    let queued_any = access.store(|store| store.apply_intersection_check_batch(completed));
    if queued_any {
        request_task_with_access(&mut access, scope, ObserverTask::IntersectionDelivery);
    }
}

fn queue_intersection_checks_with_dom(
    access: &mut ObserverHostAccess,
    scope: &mut v8::PinScope<'_, '_>,
    dom_host: &DomHost,
) {
    let Some(batch): Option<IntersectionCheckBatch> =
        access.store(ObserverStore::take_intersection_check_batch)
    else {
        return;
    };
    let Ok(completed) = access
        .read(|runtime| intersection::compute_intersection_check_batch(runtime, dom_host, batch))
    else {
        return;
    };
    let queued_any = access.store(|store| store.apply_intersection_check_batch(completed));
    if queued_any {
        request_task_with_access(access, scope, ObserverTask::IntersectionDelivery);
    }
}

pub(super) fn flush_intersection_checks(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    let mut access = ObserverHostAccess::new(host_ptr);
    access.store(|store| store.begin_task(ObserverTask::IntersectionCheck));
    queue_intersection_checks(scope, host_ptr);
}

pub(super) fn flush_intersection_observers(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    let deliveries = ObserverHostAccess::new(host_ptr).store(|store| {
        store.begin_task(ObserverTask::IntersectionDelivery);
        store.collect_intersection_deliveries(scope)
    });

    // Blink likewise removes observers from its controller pending set before
    // invoking `IntersectionObserver::Deliver`, so reentrant observe/disconnect
    // updates fresh owner state instead of aliasing the delivery traversal.
    invoke_intersection_deliveries(scope, host_ptr, deliveries);
}
