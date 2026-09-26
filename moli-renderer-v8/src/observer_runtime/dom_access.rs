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
    intersection, invoke_intersection_deliveries, invoke_mutation_deliveries,
    node_is_intersection_root, node_is_intersection_target,
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

pub(crate) fn activate_resize_observer_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    id: ObserverCallbackId,
    observer: v8::Local<'s, v8::Object>,
) {
    let mut access = ObserverHostAccess::new(host_ptr);
    let registry = access.store(|store| store.callback_registry.clone());
    if let Some(identity) =
        access.read(|host| registry.activate_resize_observer(scope, host, id, observer))
    {
        access.mutate(|host| host.queue_document_observer_update(scope, identity.dispatch_scope()));
    }
}

pub(crate) fn deactivate_resize_observer_callback(
    host_ptr: *mut JsContextHost,
    id: ObserverCallbackId,
) {
    let registry = ObserverHostAccess::new(host_ptr).store(|store| store.callback_registry.clone());
    registry.deactivate_resize_observer(id);
}

pub(crate) fn active_resize_observer_callbacks<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    owner: Option<WindowExecutionContextOwner>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let mut access = ObserverHostAccess::new(host_ptr);
    let registry = access.store(|store| store.callback_registry.clone());
    access.read(|host| registry.active_resize_observers(scope, host, owner))
}

pub(crate) fn queue_resize_observer_rendering_updates(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    let mut access = ObserverHostAccess::new(host_ptr);
    let registry = access.store(|store| store.callback_registry.clone());
    let identities = access.read(|host| registry.resize_observer_identities(host));
    for identity in identities {
        access.mutate(|host| host.queue_document_observer_update(scope, identity.dispatch_scope()));
    }
}

/// CSSOM and adopted-sheet edits bypass DOM MutationRecords but invalidate
/// observer geometry and can make the focused element unavailable.
pub(crate) fn queue_style_rendering_update(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    queue_resize_observer_rendering_updates(scope, host_ptr);
    queue_intersection_checks(scope, host_ptr);
    ObserverHostAccess::new(host_ptr).mutate(|host| host.queue_focused_document_fixup(scope));
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
        IntersectionMutationPlan::ScheduleCheck => {
            queue_intersection_checks(scope, host_ptr);
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
        queue_intersection_checks(scope, host_ptr);
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
    let identities = access.store(|store| {
        let mut identities = Vec::new();
        for state in store.intersection_observers.values() {
            if !state.observed_targets.is_empty()
                && let Some(identity) = state.callback.observer_identity()
                && !identities.contains(&identity)
            {
                identities.push(identity);
            }
        }
        identities
    });
    for identity in identities {
        if access.read(|host| host.window_execution_context_identity_is_current(identity)) {
            access.mutate(|host| {
                host.queue_document_observer_update(scope, identity.dispatch_scope())
            });
        }
    }
}

pub(crate) fn document_has_rendering_observers(
    host: &mut JsContextHost,
    owner: WindowExecutionContextOwner,
) -> bool {
    let store = host.observers_mut(&OBSERVER_STORE_ACCESS);
    let registry = store.callback_registry.clone();
    let has_intersection = store.intersection_observers.values().any(|state| {
        !state.observed_targets.is_empty()
            && state
                .callback
                .observer_identity()
                .is_some_and(|identity| identity.owner() == owner)
    });
    has_intersection
        || registry
            .resize_observer_identities(host)
            .iter()
            .any(|identity| identity.owner() == owner)
}

/// Sample intersections after the ResizeObserver loop and focus fixup. Delivery
/// receives its own exact-Document task, so callback cleanup cannot run it in
/// the middle of another rendering phase.
pub(crate) fn update_document_intersections(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    target: crate::native_bridge::WindowDocumentTaskTarget,
    owner: WindowExecutionContextOwner,
) -> Result<(), moli_layout::LayoutError> {
    let mut access = ObserverHostAccess::new(host_ptr);
    if !access.read(|host| {
        host.window_document_owner_is_current_for_dispatch_scope(
            target.owner(),
            target.dispatch_scope(),
        )
    }) {
        return Ok(());
    }
    let Some(batch) = access.store(|store| store.take_intersection_check_batch(owner)) else {
        return Ok(());
    };
    let completed = access.read(|host| {
        intersection::compute_intersection_check_batch(host, host.dom_host(), batch)
    })?;
    if access.store(|store| store.apply_intersection_check_batch(completed)) {
        access.mutate(|host| host.queue_document_intersection_delivery(target, owner));
    }
    let _ = scope;
    Ok(())
}

pub(crate) fn deliver_document_intersections(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    owner: WindowExecutionContextOwner,
    target: crate::native_bridge::WindowDocumentTaskTarget,
) {
    let deliveries = ObserverHostAccess::new(host_ptr)
        .store(|store| store.collect_intersection_deliveries(scope, owner));
    invoke_intersection_deliveries(scope, host_ptr, target, deliveries);
}
