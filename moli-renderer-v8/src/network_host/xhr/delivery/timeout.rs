use super::super::events::{
    xhr_dispatch_progress_event, xhr_fire_readystatechange, xhr_is_aborted,
};
use super::super::*;
use moli_webapi_declare::WebApiObject;
use std::time::{SystemTime, UNIX_EPOCH};

const XHR_TIMEOUT_DATA_XHR: &str = "xhr";
const XHR_TIMEOUT_DATA_INTERNAL_ID: &str = "internalId";

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct XhrTimeoutDataDeclaration<'scope> {
    xhr: v8::Local<'scope, v8::Object>,
    internal_id: f64,
}

pub(in crate::network_host::xhr) fn schedule_xhr_timeout(
    scope: &mut v8::PinScope<'_, '_>,
    host: &mut JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
    internal_id: u64,
) {
    let xhr = local_object_in_scope(scope, xhr);
    let Some(delay_ms) = xhr_timeout_remaining_delay_ms(scope, xhr) else {
        return;
    };
    if internal_id == 0 {
        return;
    }

    cancel_xhr_timeout(scope, xhr);

    let data = XhrTimeoutDataDeclaration::new(xhr, internal_id as f64)
        .bind(scope)
        .expect("XHR timeout data declaration should bind");
    let Some(callback) = v8::FunctionTemplate::builder(xhr_timeout_callback)
        .data(data.into())
        .build(scope)
        .get_function(scope)
    else {
        return;
    };
    let timer_id = host.queue_timeout(
        scope,
        callback,
        delay_ms,
        crate::host::HostTimerOwner::Window,
        Vec::new(),
    );
    set_xhr_state_number(scope, xhr, XHR_TIMEOUT_TIMER_SLOT, timer_id as f64);
}

fn local_object_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let global = v8::Global::new(scope, object);
    v8::Local::new(scope, global)
}

pub(in crate::network_host::xhr) fn cancel_xhr_timeout(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    let timer_id = xhr_state_number_property(scope, xhr, XHR_TIMEOUT_TIMER_SLOT)
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| value as u32)
        .unwrap_or(0);
    if timer_id == 0 {
        return;
    }
    set_xhr_state_number(scope, xhr, XHR_TIMEOUT_TIMER_SLOT, 0.0);
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }.cancel_timer(timer_id);
    }
}

pub(in crate::network_host::xhr) fn mark_xhr_timeout_start(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    set_xhr_state_number(scope, xhr, XHR_TIMEOUT_START_MS_SLOT, current_time_ms());
}

pub(in crate::network_host::xhr) fn clear_xhr_timeout_start(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    set_xhr_state_number(scope, xhr, XHR_TIMEOUT_START_MS_SLOT, 0.0);
}

pub(in crate::network_host::xhr) fn reschedule_xhr_timeout_after_timeout_change(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) {
    let active_internal_id =
        xhr_state_number_property(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT).unwrap_or(0.0) as u64;
    let send_flag = xhr_state_bool_property(scope, xhr, XHR_SEND_FLAG_SLOT).unwrap_or(false);
    cancel_xhr_timeout(scope, xhr);
    if !send_flag || active_internal_id == 0 {
        return;
    }
    if xhr_timeout_start_ms(scope, xhr).is_none() {
        mark_xhr_timeout_start(scope, xhr);
    }
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        schedule_xhr_timeout(scope, unsafe { &mut *host_ptr }, xhr, active_internal_id);
    }
}

pub(in crate::network_host::xhr) fn xhr_timeout_remaining_delay_ms(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> Option<u32> {
    let timeout_ms = xhr_state_number_property(scope, xhr, XHR_TIMEOUT_SLOT)?;
    remaining_timeout_delay_ms(
        timeout_ms,
        xhr_timeout_start_ms(scope, xhr),
        current_time_ms(),
    )
}

fn xhr_timeout_start_ms(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> Option<f64> {
    xhr_state_number_property(scope, xhr, XHR_TIMEOUT_START_MS_SLOT)
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn current_time_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

fn remaining_timeout_delay_ms(
    timeout_ms: f64,
    started_at_ms: Option<f64>,
    now_ms: f64,
) -> Option<u32> {
    if !timeout_ms.is_finite() || timeout_ms <= 0.0 {
        return None;
    }
    let elapsed_ms = started_at_ms
        .filter(|started_at_ms| started_at_ms.is_finite() && *started_at_ms > 0.0)
        .map(|started_at_ms| (now_ms - started_at_ms).max(0.0))
        .unwrap_or(0.0);
    let remaining_ms = (timeout_ms - elapsed_ms).max(0.0).ceil();
    Some(remaining_ms.min(u32::MAX as f64) as u32)
}

fn xhr_timeout_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(data) = args.data().to_object(scope) else {
        rv.set_undefined();
        return;
    };
    let Some(xhr) = data
        .get(scope, v8str(scope, XHR_TIMEOUT_DATA_XHR).into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let scheduled_internal_id = data
        .get(scope, v8str(scope, XHR_TIMEOUT_DATA_INTERNAL_ID).into())
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value > 0.0)
        .map(|value| value as u64)
        .unwrap_or(0);
    let active_internal_id =
        xhr_state_number_property(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT).unwrap_or(0.0) as u64;
    if scheduled_internal_id == 0 || scheduled_internal_id != active_internal_id {
        rv.set_undefined();
        return;
    }

    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let _ = unsafe { &mut *host_ptr }.abort_subresource_fetch(active_internal_id);
    }
    apply_xhr_timeout(scope, xhr);
    rv.set_undefined();
}

pub(crate) fn apply_xhr_timeout(scope: &mut v8::PinScope<'_, '_>, xhr: v8::Local<'_, v8::Object>) {
    set_xhr_state_number(scope, xhr, XHR_TIMEOUT_TIMER_SLOT, 0.0);
    super::clear_xhr_progress_throttle(scope, xhr);
    clear_xhr_timeout_start(scope, xhr);
    set_xhr_state_number(scope, xhr, XHR_ACTIVE_INTERNAL_ID_SLOT, 0.0);
    set_xhr_state_bool(scope, xhr, XHR_SEND_FLAG_SLOT, false);
    if xhr_is_aborted(scope, xhr) {
        return;
    }
    super::reset_xhr_response_for_request_error(scope, xhr);
    set_xhr_state_number(scope, xhr, XHR_READY_STATE_SLOT, 4.0);
    xhr_fire_readystatechange(scope, xhr, 4);
    if xhr_is_aborted(scope, xhr) {
        return;
    }
    super::super::upload::dispatch_xhr_upload_error_if_in_progress(scope, xhr, "timeout");
    xhr_dispatch_progress_event(scope, xhr, "timeout", 0.0, 0.0);
    xhr_dispatch_progress_event(scope, xhr, "loadend", 0.0, 0.0);
}

#[cfg(test)]
mod tests {
    use super::remaining_timeout_delay_ms;

    #[test]
    fn timeout_delay_counts_from_send_start() {
        assert_eq!(
            remaining_timeout_delay_ms(4000.0, Some(1_000.0), 2_500.0),
            Some(2500)
        );
        assert_eq!(
            remaining_timeout_delay_ms(4000.0, Some(1_000.0), 5_500.0),
            Some(0)
        );
    }

    #[test]
    fn timeout_delay_uses_full_timeout_without_start_time() {
        assert_eq!(
            remaining_timeout_delay_ms(2000.0, None, 10_000.0),
            Some(2000)
        );
    }

    #[test]
    fn timeout_delay_ignores_disabled_timeout() {
        assert_eq!(
            remaining_timeout_delay_ms(0.0, Some(1_000.0), 2_000.0),
            None
        );
        assert_eq!(
            remaining_timeout_delay_ms(f64::NAN, Some(1_000.0), 2_000.0),
            None
        );
    }
}
