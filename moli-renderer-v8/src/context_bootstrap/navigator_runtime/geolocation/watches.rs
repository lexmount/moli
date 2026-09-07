//! Watch callbacks are traced from the Geolocation object, not independent
//! Rust roots. A callback capturing its Navigator/Window cannot create a
//! permanent Rust-to-JavaScript reference cycle.

use crate::{
    util::{get_private_value, set_private_value},
    v8_traced_webidl_callback::V8TracedWebIdlCallbackFunction,
};
use moli_webidl_callback::WebIdlCallbackFunction;

const WATCHES_SLOT: &str = "__moliGeolocationWatches";
const SUCCESS_SLOT: &str = "__moliGeolocationWatchSuccess";
const ERROR_SLOT: &str = "__moliGeolocationWatchError";
const TIMEOUT_SLOT: &str = "__moliGeolocationWatchTimeout";

pub(super) fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    geolocation: v8::Local<'s, v8::Object>,
) {
    set_private_value(scope, geolocation, WATCHES_SLOT, v8::Map::new(scope).into());
}

pub(super) fn insert<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    geolocation: v8::Local<'s, v8::Object>,
    watch_id: i32,
    success: WebIdlCallbackFunction,
    error: Option<WebIdlCallbackFunction>,
    timeout: u32,
) {
    let watch = v8::Object::new(scope);
    let success = V8TracedWebIdlCallbackFunction::new(scope, success).into_object();
    set_private_value(scope, watch, SUCCESS_SLOT, success.into());
    if let Some(error) = error {
        let error = V8TracedWebIdlCallbackFunction::new(scope, error).into_object();
        set_private_value(scope, watch, ERROR_SLOT, error.into());
    }
    set_private_value(
        scope,
        watch,
        TIMEOUT_SLOT,
        v8::Integer::new_from_unsigned(scope, timeout).into(),
    );
    let _ = map(scope, geolocation).set(
        scope,
        v8::Integer::new(scope, watch_id).into(),
        watch.into(),
    );
    queue(scope, geolocation, watch_id, watch);
}

pub(super) fn remove<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    geolocation: v8::Local<'s, v8::Object>,
    watch_id: i32,
) {
    let _ = map(scope, geolocation).delete(scope, v8::Integer::new(scope, watch_id).into());
}

pub(super) fn notify<'s>(scope: &mut v8::PinScope<'s, '_>, geolocation: v8::Local<'s, v8::Object>) {
    let entries = map(scope, geolocation).as_array(scope);
    for index in (0..entries.length()).step_by(2) {
        let id = entries
            .get_index(scope, index)
            .and_then(|value| value.int32_value(scope));
        let watch = entries
            .get_index(scope, index + 1)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
        if let (Some(id), Some(watch)) = (id, watch) {
            queue(scope, geolocation, id, watch);
        }
    }
}

fn map<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    geolocation: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Map> {
    get_private_value(scope, geolocation, WATCHES_SLOT)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
        .expect("branded Geolocation retains its watch map")
}

fn callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    watch: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<WebIdlCallbackFunction> {
    let carrier = get_private_value(scope, watch, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let prepared = V8TracedWebIdlCallbackFunction::from_object(carrier).prepare(scope);
    let callback = prepared.callback(scope);
    let relevant_context = prepared.relevant_context(scope);
    let incumbent_context = prepared.incumbent_context(scope);
    WebIdlCallbackFunction::try_new(scope, callback, relevant_context, incumbent_context)
}

fn queue<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    geolocation: v8::Local<'s, v8::Object>,
    id: i32,
    watch: v8::Local<'s, v8::Object>,
) {
    let success = callback(scope, watch, SUCCESS_SLOT).expect("watch success callback");
    let error = callback(scope, watch, ERROR_SLOT);
    let timeout = get_private_value(scope, watch, TIMEOUT_SLOT)
        .and_then(|value| value.uint32_value(scope))
        .expect("watch timeout");
    super::queue_geolocation_result(scope, geolocation, success, error, timeout, Some(id));
}
