use std::rc::Rc;

/// Native event-loop deadline query for an IdleDeadline's creating realm.
pub struct IdleDeadlineRuntime {
    pub pending_work_deadline_ms:
        for<'s, 'i> fn(&mut v8::PinScope<'s, 'i>, v8::Local<'s, v8::Object>, f64) -> Option<f64>,
}

pub fn install_runtime(scope: &mut v8::PinScope<'_, '_>, runtime: IdleDeadlineRuntime) {
    scope.get_current_context().set_slot(Rc::new(runtime));
}

use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, throw_type_error};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
const IDLE_DEADLINE_DID_TIMEOUT_SLOT: &str = "__moliIdleDeadlineDidTimeout";
const IDLE_DEADLINE_MS_SLOT: &str = "__moliIdleDeadlineMs";
const IDLE_DEADLINE_TIME_ORIGIN_SLOT: &str = "__moliIdleDeadlineTimeOrigin";
#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::IdleDeadline)]
struct IdleDeadlineDeclaration {
    #[webapi(slot = IDLE_DEADLINE_MS_SLOT)]
    deadline_ms: f64,
    #[webapi(slot = IDLE_DEADLINE_TIME_ORIGIN_SLOT)]
    time_origin_ms: f64,
    #[webapi(slot = IDLE_DEADLINE_DID_TIMEOUT_SLOT)]
    did_timeout: bool,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::IdleDeadline, enumerable)]
struct IdleDeadlinePrototypeDeclaration {
    #[webapi(
        accessor_property = "didTimeout",
        getter = window_idle_deadline_did_timeout_getter
    )]
    did_timeout: (),
    #[webapi(
        method,
        length = 0,
        callback = window_idle_deadline_time_remaining_callback
    )]
    time_remaining: (),
}

pub fn install_idle_deadline_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    IdleDeadlinePrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

fn idle_deadline_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if web_api_interfaces::IdleDeadline::is_instance(scope, receiver) {
        return Some(receiver);
    }
    throw_type_error(scope, "Illegal invocation");
    None
}

fn window_idle_deadline_did_timeout_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(deadline) = idle_deadline_receiver(scope, args.this()) else {
        return;
    };
    let did_timeout = get_private_value(scope, deadline, IDLE_DEADLINE_DID_TIMEOUT_SLOT)
        .is_some_and(|value| value.boolean_value(scope));
    rv.set_bool(did_timeout);
}

fn window_idle_deadline_time_remaining_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(deadline) = idle_deadline_receiver(scope, args.this()) else {
        return;
    };
    let now_ms = moli_time::unix_epoch_millis();
    let runtime = deadline
        .get_creation_context(scope)
        .and_then(|context| context.get_slot::<IdleDeadlineRuntime>());
    let pending_work_deadline_ms =
        runtime.and_then(|runtime| (runtime.pending_work_deadline_ms)(scope, deadline, now_ms));
    let time_origin_ms = get_private_value(scope, deadline, IDLE_DEADLINE_TIME_ORIGIN_SLOT)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0);
    let remaining_ms = get_private_value(scope, deadline, IDLE_DEADLINE_MS_SLOT)
        .and_then(|value| value.number_value(scope))
        .map_or(0.0, |deadline_ms| {
            let deadline_ms =
                pending_work_deadline_ms.map_or(deadline_ms, |pending| deadline_ms.min(pending));
            // Use the captured origin even if timeRemaining is borrowed from
            // another realm. Both timestamps must match performance.now()'s
            // relative clock and precision rather than the raw wall clock.
            moli_time::coarsened_dom_time_remaining_millis(
                (deadline_ms - time_origin_ms).max(0.0),
                (now_ms - time_origin_ms).max(0.0),
            )
        });
    rv.set(v8::Number::new(scope, remaining_ms).into());
}

pub fn build<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    deadline_ms: f64,
    time_origin_ms: f64,
    did_timeout: bool,
) -> Option<v8::Local<'s, v8::Object>> {
    IdleDeadlineDeclaration::new(deadline_ms, time_origin_ms, did_timeout)
        .bind(scope)
        .ok()
}
