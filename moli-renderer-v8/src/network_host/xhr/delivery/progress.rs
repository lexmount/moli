use super::super::events::{
    xhr_dispatch_progress_event_with_length_computable, xhr_fire_readystatechange, xhr_is_aborted,
};
use super::super::instance_state::{
    XHR_PROGRESS_HAS_DISPATCHED_SLOT, XHR_PROGRESS_LENGTH_COMPUTABLE_SLOT,
    XHR_PROGRESS_LOADED_SLOT, XHR_PROGRESS_PENDING_SLOT, XHR_PROGRESS_TIMER_SLOT,
    XHR_PROGRESS_TOTAL_SLOT,
};
use super::super::*;
use moli_webapi_declare::WebApiObject;

const XHR_PROGRESS_DATA_XHR: &str = "xhr";
const XHR_PROGRESS_DATA_INTERNAL_ID: &str = "internalId";
const XHR_PROGRESS_MINIMUM_INTERVAL_MS: u32 = 50;

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct XhrProgressTimerDataDeclaration<'scope> {
    xhr: v8::Local<'scope, v8::Object>,
    internal_id: f64,
}

pub(super) fn xhr_stream_is_current(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    internal_id: u64,
) -> bool {
    !xhr_is_aborted(scope, xhr)
        && xhr_state_bool_property(scope, xhr, XHR_SEND_FLAG_SLOT).unwrap_or(false)
        && xhr_state_number_property(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT)
            .is_some_and(|active| active as u64 == internal_id)
}

pub(super) fn dispatch_or_defer_xhr_streaming_progress(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    internal_id: u64,
    loaded: usize,
    total: Option<usize>,
) -> bool {
    if !xhr_stream_is_current(scope, xhr, internal_id) {
        return false;
    }

    if xhr_progress_timer_id(scope, xhr) != 0 {
        store_deferred_xhr_progress(scope, xhr, loaded, total);
        return xhr_stream_is_current(scope, xhr, internal_id);
    }

    let snapshot = XhrProgressSnapshot::new(loaded, total);
    if !dispatch_xhr_progress_snapshot(scope, xhr, internal_id, snapshot) {
        return false;
    }
    schedule_xhr_progress_gate(scope, xhr, internal_id);
    xhr_stream_is_current(scope, xhr, internal_id)
}

pub(super) fn flush_xhr_streaming_progress(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    internal_id: u64,
) -> bool {
    if !xhr_stream_is_current(scope, xhr, internal_id) {
        clear_xhr_progress_throttle(scope, xhr);
        return false;
    }

    cancel_xhr_progress_timer(scope, xhr);
    let deferred = take_deferred_xhr_progress(scope, xhr);
    if let Some(snapshot) = deferred
        && !dispatch_xhr_progress_snapshot(scope, xhr, internal_id, snapshot)
    {
        clear_xhr_progress_state(scope, xhr);
        return false;
    }
    clear_xhr_progress_state(scope, xhr);
    xhr_stream_is_current(scope, xhr, internal_id)
}

pub(in crate::network_host::xhr) fn clear_xhr_progress_throttle(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    cancel_xhr_progress_timer(scope, xhr);
    clear_xhr_progress_state(scope, xhr);
}

fn dispatch_xhr_progress_snapshot(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    internal_id: u64,
    snapshot: XhrProgressSnapshot,
) -> bool {
    let ready_state =
        xhr_state_number_property(scope, xhr, XHR_READY_STATE_SLOT).unwrap_or(0.0) as u32;
    let has_dispatched =
        xhr_state_bool_property(scope, xhr, XHR_PROGRESS_HAS_DISPATCHED_SLOT).unwrap_or(false);
    if ready_state != 3 || has_dispatched {
        xhr_fire_readystatechange(scope, xhr, 3);
        if scope.is_execution_terminating() || !xhr_stream_is_current(scope, xhr, internal_id) {
            return false;
        }
    }

    set_xhr_state_bool(scope, xhr, XHR_PROGRESS_HAS_DISPATCHED_SLOT, true);
    xhr_dispatch_progress_event_with_length_computable(
        scope,
        xhr,
        "progress",
        snapshot.length_computable,
        snapshot.loaded,
        snapshot.total,
    );
    !scope.is_execution_terminating() && xhr_stream_is_current(scope, xhr, internal_id)
}

fn schedule_xhr_progress_gate(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    internal_id: u64,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let xhr = local_object_in_scope(scope, xhr);
    let data = XhrProgressTimerDataDeclaration::new(xhr, internal_id as f64)
        .bind(scope)
        .expect("XHR progress timer data declaration should bind");
    let Some(callback) = v8::FunctionTemplate::builder(xhr_progress_timer_callback)
        .data(data.into())
        .build(scope)
        .get_function(scope)
    else {
        return;
    };
    let timer_id = unsafe { &mut *host_ptr }.queue_timeout_with_receiver(
        scope,
        callback,
        xhr,
        XHR_PROGRESS_MINIMUM_INTERVAL_MS,
        crate::host::HostTimerOwner::Window,
        Vec::new(),
    );
    set_xhr_state_number(scope, xhr, XHR_PROGRESS_TIMER_SLOT, timer_id as f64);
}

fn xhr_progress_timer_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(data) = args.data().to_object(scope) else {
        rv.set_undefined();
        return;
    };
    let Some(xhr) = data
        .get(scope, v8str(scope, XHR_PROGRESS_DATA_XHR).into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let internal_id = data
        .get(scope, v8str(scope, XHR_PROGRESS_DATA_INTERNAL_ID).into())
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| value as u64)
        .unwrap_or(0);
    if internal_id == 0 || !xhr_stream_is_current(scope, xhr, internal_id) {
        rv.set_undefined();
        return;
    }

    set_xhr_state_number(scope, xhr, XHR_PROGRESS_TIMER_SLOT, 0.0);
    let Some(snapshot) = take_deferred_xhr_progress(scope, xhr) else {
        rv.set_undefined();
        return;
    };
    if dispatch_xhr_progress_snapshot(scope, xhr, internal_id, snapshot) {
        schedule_xhr_progress_gate(scope, xhr, internal_id);
    }
    rv.set_undefined();
}

fn cancel_xhr_progress_timer(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    let timer_id = xhr_progress_timer_id(scope, xhr);
    set_xhr_state_number(scope, xhr, XHR_PROGRESS_TIMER_SLOT, 0.0);
    if timer_id != 0
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.cancel_timer(timer_id);
    }
}

fn xhr_progress_timer_id(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) -> u32 {
    xhr_state_number_property(scope, xhr, XHR_PROGRESS_TIMER_SLOT)
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| value as u32)
        .unwrap_or(0)
}

fn store_deferred_xhr_progress(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    loaded: usize,
    total: Option<usize>,
) {
    let snapshot = XhrProgressSnapshot::new(loaded, total);
    set_xhr_state_bool(scope, xhr, XHR_PROGRESS_PENDING_SLOT, true);
    set_xhr_state_bool(
        scope,
        xhr,
        XHR_PROGRESS_LENGTH_COMPUTABLE_SLOT,
        snapshot.length_computable,
    );
    set_xhr_state_number(scope, xhr, XHR_PROGRESS_LOADED_SLOT, snapshot.loaded);
    set_xhr_state_number(scope, xhr, XHR_PROGRESS_TOTAL_SLOT, snapshot.total);
}

fn take_deferred_xhr_progress(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> Option<XhrProgressSnapshot> {
    if !xhr_state_bool_property(scope, xhr, XHR_PROGRESS_PENDING_SLOT).unwrap_or(false) {
        return None;
    }
    set_xhr_state_bool(scope, xhr, XHR_PROGRESS_PENDING_SLOT, false);
    Some(XhrProgressSnapshot {
        length_computable: xhr_state_bool_property(scope, xhr, XHR_PROGRESS_LENGTH_COMPUTABLE_SLOT)
            .unwrap_or(false),
        loaded: xhr_state_number_property(scope, xhr, XHR_PROGRESS_LOADED_SLOT).unwrap_or(0.0),
        total: xhr_state_number_property(scope, xhr, XHR_PROGRESS_TOTAL_SLOT).unwrap_or(0.0),
    })
}

fn clear_xhr_progress_state(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    set_xhr_state_bool(scope, xhr, XHR_PROGRESS_PENDING_SLOT, false);
    set_xhr_state_bool(scope, xhr, XHR_PROGRESS_HAS_DISPATCHED_SLOT, false);
    set_xhr_state_bool(scope, xhr, XHR_PROGRESS_LENGTH_COMPUTABLE_SLOT, false);
    set_xhr_state_number(scope, xhr, XHR_PROGRESS_LOADED_SLOT, 0.0);
    set_xhr_state_number(scope, xhr, XHR_PROGRESS_TOTAL_SLOT, 0.0);
}

fn local_object_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let global = v8::Global::new(scope, object);
    v8::Local::new(scope, global)
}

#[derive(Clone, Copy)]
struct XhrProgressSnapshot {
    length_computable: bool,
    loaded: f64,
    total: f64,
}

impl XhrProgressSnapshot {
    fn new(loaded: usize, total: Option<usize>) -> Self {
        match total.filter(|total| *total > 0 && loaded <= *total) {
            Some(total) => Self {
                length_computable: true,
                loaded: loaded as f64,
                total: total as f64,
            },
            None => Self {
                length_computable: false,
                loaded: loaded as f64,
                total: 0.0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::XhrProgressSnapshot;

    #[test]
    fn progress_snapshot_only_exposes_a_valid_nonzero_total() {
        let valid = XhrProgressSnapshot::new(4, Some(7));
        assert!(valid.length_computable);
        assert_eq!(valid.loaded, 4.0);
        assert_eq!(valid.total, 7.0);

        for invalid in [
            XhrProgressSnapshot::new(0, None),
            XhrProgressSnapshot::new(0, Some(0)),
            XhrProgressSnapshot::new(8, Some(7)),
        ] {
            assert!(!invalid.length_computable);
            assert_eq!(invalid.total, 0.0);
        }
    }
}
