use super::*;
use crate::native_bridge::{WindowDocumentTaskTarget, WindowExecutionContextBinding};

pub(super) fn is_enabled(scope: &mut v8::PinScope<'_, '_>) -> bool {
    context_host_ptr_from_global_bridge(scope)
        .is_some_and(|host| unsafe { &*host }.layout_policy().is_realtime())
}

pub(super) fn observe_if_realtime<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    targets: v8::Local<'s, v8::Array>,
    target: v8::Local<'s, v8::Object>,
    observed_box: ResizeObserverBoxOptions,
) -> bool {
    if !is_enabled(scope) {
        return false;
    }
    if let Some(index) = observed_record_index(scope, targets, target.into()) {
        let existing = targets
            .get_index(scope, index)
            .and_then(|record| v8::Local::<v8::Object>::try_from(record).ok());
        if existing
            .and_then(|record| observed_record_box(scope, record))
            .is_some_and(|old| old.as_str() == observed_box.as_str())
        {
            // Chromium keeps the original observation and its reported size.
            // Re-observing from a callback must not create an idle loop.
            return true;
        }
        let next = without_target(scope, targets, target.into());
        set_resize_observer_targets(scope, observer, next);
        if let Some(active) = resize_observer_pending_targets(scope, observer) {
            let active = without_target(scope, active, target.into());
            set_resize_observer_pending_targets(scope, observer, active);
        }
    }
    let targets = resize_observer_targets(scope, observer).expect("RO targets");
    let record = build_observed_record(scope, target, observed_box);
    let _ = targets.set_index(scope, targets.length(), record.into());
    if let Some(id) = resize_observer_callback_id(scope, observer)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        crate::observer_runtime::activate_realtime_resize_observer(
            scope,
            unsafe { &*host_ptr },
            id,
            observer,
        );
    }
    true
}

fn without_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    records: v8::Local<'s, v8::Array>,
    target: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Array> {
    let next = v8::Array::new(scope, 0);
    for i in 0..records.length() {
        if let Some(record) = records.get_index(scope, i)
            && !observed_record_matches_target(scope, record, target)
        {
            let _ = next.set_index(scope, next.length(), record);
        }
    }
    next
}

pub(super) fn deactivate_if_realtime<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> bool {
    if !is_enabled(scope) {
        return false;
    }
    if let Some(id) = resize_observer_callback_id(scope, observer)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        crate::observer_runtime::deactivate_realtime_resize_observer(unsafe { &*host_ptr }, id);
    }
    true
}

pub(crate) struct RealtimeResizeObserverGather<'s> {
    pub(crate) observers: Vec<v8::Local<'s, v8::Object>>,
    pub(crate) skipped: bool,
}

fn target_depth<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host: &JsContextHost,
    target: v8::Local<'s, v8::Value>,
) -> usize {
    let mut node = crate::native_bridge::branded_node_handle(
        scope,
        target,
        web_api_interfaces::Element::DESCRIPTOR,
    );
    let dom = host.dom_host();
    let mut depth = 0;
    while let Some(handle) = node {
        if dom.element(handle).is_some() {
            depth += 1;
        }
        node = dom.composed_parent(handle);
    }
    depth
}

pub(crate) fn gather_realtime_resize_observers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    target: WindowDocumentTaskTarget,
    depth: usize,
) -> Result<RealtimeResizeObserverGather<'s>, moli_layout::LayoutError> {
    let observers = crate::observer_runtime::take_realtime_resize_observers(
        scope,
        unsafe { &*host_ptr },
        target,
    );
    let mut result = RealtimeResizeObserverGather {
        observers: Vec::new(),
        skipped: false,
    };
    for (_, observer) in observers {
        let active = v8::Array::new(scope, 0);
        if let Some(targets) = resize_observer_targets(scope, observer) {
            for_each_active_resize_observation(scope, targets, |scope, measurement| {
                if target_depth(scope, unsafe { &*host_ptr }, measurement.entry.target) > depth {
                    if let Some(record) = measurement.record {
                        let _ = active.set_index(scope, active.length(), record.into());
                    }
                } else {
                    result.skipped = true;
                }
            })?;
        }
        // Use the existing traced record slot as the frame route's active
        // targets. unobserve/disconnect can edit it before another observer's
        // broadcast without maintaining a second target/size database.
        set_resize_observer_pending_targets(scope, observer, active);
        if active.length() != 0 {
            result.observers.push(observer);
        }
    }
    Ok(result)
}

pub(crate) fn broadcast_realtime_resize_observer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    observer: v8::Local<'s, v8::Object>,
) -> Result<Option<usize>, moli_layout::LayoutError> {
    let Some(active) = resize_observer_pending_targets(scope, observer) else {
        return Ok(None);
    };
    if active.length() == 0 {
        return Ok(None);
    }
    let Some(residence) = resize_observer_callback_residence(scope, observer) else {
        return Ok(None);
    };
    let Some(callback) = crate::observer_runtime::prepare_callback(scope, host_ptr, residence)
    else {
        return Ok(None);
    };
    let Some(context) = callback.relevant_context(scope) else {
        return Ok(None);
    };
    let mut shallowest = usize::MAX;
    let entries = {
        let scope = &mut v8::ContextScope::new(scope, context);
        let mut entries = Vec::new();
        // Gather selects observations, not immutable entry dimensions. An
        // earlier callback may force a new layout; broadcast reads that latest
        // published sample without itself inserting a layout between observers.
        for_each_resize_observation(scope, active, false, |scope, measurement| {
            shallowest = shallowest.min(target_depth(
                scope,
                unsafe { &*host_ptr },
                measurement.entry.target,
            ));
            entries.push(measurement.into_reported_entry(scope));
        })?;
        serialize_v8_iter_array(scope, entries).unwrap_or_else(|| v8::Array::new(scope, 0))
    };
    let empty = v8::Array::new(scope, 0);
    set_resize_observer_pending_targets(scope, observer, empty);
    if entries.length() == 0 {
        return Ok(None);
    }
    let observer_value: v8::Local<'_, v8::Value> = observer.into();
    if let WindowWebIdlCallbackFunctionOutcome::Threw(report) = callback.invoke(
        scope,
        host_ptr,
        "ResizeObserver callback",
        observer_value,
        &[entries.into(), observer_value],
    ) && callback.is_current(unsafe { &*host_ptr })
        && let Some(identity) = callback.relevant_identity()
        && let Some(context) = callback.relevant_context(scope)
    {
        let binding = WindowExecutionContextBinding::new(
            identity.owner(),
            identity.dispatch_scope(),
            identity.realm_token(),
            v8::Global::new(scope, context),
        );
        binding.with_current_scope(scope, host_ptr, |scope, _| {
            report_event_callback_exception(
                scope,
                host_ptr,
                "resizeobserver",
                Some(identity),
                None,
                &report,
            );
        });
    }
    Ok(Some(shallowest))
}
