use super::super::timer_callback::WorkerTimerCallback;
use super::*;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DedicatedWorkerGlobalScope.requestAnimationFrame")]
struct WorkerRequestAnimationFrameArgs {
    #[webidl(
        required,
        converter = "callback_function",
        missing_message = "Failed to execute 'requestAnimationFrame' on 'DedicatedWorkerGlobalScope': parameter 1 is not a function."
    )]
    callback: webidl::WebIdlCallbackFunction,
}

pub(super) fn worker_set_timeout_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    let timer_id = {
        let mut s = state.borrow_mut();
        s.next_timer_id += 1;
        s.next_timer_id
    };

    if args.length() > 0 {
        let Some((handler, delay_ms)) = prepare_worker_timer_arguments(scope, &args, "setTimeout")
        else {
            return;
        };
        let callback = match worker_timer_callback_from_arg(scope, handler, "setTimeout") {
            Ok(Some(callback)) => callback,
            Ok(None) => {
                rv.set_uint32(0);
                return;
            }
            Err(()) => return,
        };

        // Collect extra arguments to pass to the callback
        let extra_args: Vec<v8::Global<v8::Value>> = (2..args.length())
            .map(|i| v8::Global::new(scope, args.get(i)))
            .collect();

        let timer_info = TimerInfo {
            id: timer_id,
            callback,
            delay_ms,
            is_interval: false,
            extra_args,
        };
        if let Some(timers) = worker_isolate_timer_queues(scope) {
            timers.push_pending(timer_info);
        }
    }

    rv.set(v8::Integer::new(scope, timer_id as i32).into());
}

pub(super) fn worker_set_interval_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    let timer_id = {
        let mut s = state.borrow_mut();
        s.next_timer_id += 1;
        s.next_timer_id
    };

    if args.length() > 0 {
        let Some((handler, delay_ms)) = prepare_worker_timer_arguments(scope, &args, "setInterval")
        else {
            return;
        };
        let callback = match worker_timer_callback_from_arg(scope, handler, "setInterval") {
            Ok(Some(callback)) => callback,
            Ok(None) => {
                rv.set_uint32(0);
                return;
            }
            Err(()) => return,
        };

        let extra_args: Vec<v8::Global<v8::Value>> = (2..args.length())
            .map(|i| v8::Global::new(scope, args.get(i)))
            .collect();

        let timer_info = TimerInfo {
            id: timer_id,
            callback,
            delay_ms,
            is_interval: true,
            extra_args,
        };
        if let Some(timers) = worker_isolate_timer_queues(scope) {
            timers.push_pending(timer_info);
        }
    }

    rv.set(v8::Integer::new(scope, timer_id as i32).into());
}

fn prepare_worker_timer_arguments<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    timer_name: &'static str,
) -> Option<(v8::Local<'s, v8::Value>, u64)> {
    let value = args.get(0);
    // Web IDL conversions precede both Trusted Types default-policy calls and
    // CSP checks. Preserve callable and TrustedScript union members as objects.
    let handler = if value.is_string()
        || v8::Local::<v8::Object>::try_from(value).is_ok_and(|object| object.is_callable())
        || crate::context_bootstrap::trusted_type_kind(scope, value)
            == Some(crate::context_bootstrap::TrustedTypeKind::Script)
    {
        value
    } else {
        value.to_string(scope)?.into()
    };
    let prefix = match timer_name {
        "setInterval" => "DedicatedWorkerGlobalScope.setInterval",
        _ => "DedicatedWorkerGlobalScope.setTimeout",
    };
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let delay_ms = worker_timer_delay_ms(&mut scope, args, prefix);
    if scope.has_caught() {
        let _ = scope.rethrow();
        return None;
    }
    Some((handler, delay_ms))
}

fn worker_timer_callback_from_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    timer_name: &'static str,
) -> Result<Option<WorkerTimerCallback>, ()> {
    if let Ok(callback) = v8::Local::<v8::Object>::try_from(value)
        && callback.is_callable()
    {
        let current_context = scope.get_current_context();
        let relevant_context = callback
            .get_creation_context(scope)
            .unwrap_or(current_context);
        let incumbent_context = scope.get_incumbent_context().unwrap_or(current_context);
        let callback = webidl::WebIdlCallbackFunction::try_new(
            scope,
            callback,
            relevant_context,
            incumbent_context,
        )
        .expect("a callable worker timer handler must convert to a callback function");
        return Ok(Some(WorkerTimerCallback::webidl_timer(scope, callback)));
    }

    let requirements = worker_trusted_types_for_script_requirements(scope).unwrap_or_default();
    let sink = match timer_name {
        "setInterval" => "WorkerGlobalScope setInterval",
        _ => "WorkerGlobalScope setTimeout",
    };
    let Some(source) = crate::context_bootstrap::trusted_script_string_or_type_error(
        scope,
        value,
        requirements,
        sink,
        timer_name,
    ) else {
        return Err(());
    };
    let allow_trusted_types_eval =
        requirements.is_enforced() && worker_allows_trusted_types_eval(scope).unwrap_or(false);
    if !worker_allows_eval_code_generation_by_csp(scope, allow_trusted_types_eval, Some(&source))
        .unwrap_or(true)
    {
        return Ok(None);
    }
    let wrapper = format!("(function() {{\n{source}\n}})");
    let Some(source) = v8::String::new(scope, &wrapper) else {
        return Ok(None);
    };
    Ok(v8::Script::compile(scope, source, None)
        .and_then(|script| script.run(scope))
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .map(|callback| WorkerTimerCallback::browser_function(scope, callback)))
}

pub(super) fn worker_clear_timeout_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() > 0 {
        let id = args.get(0).uint32_value(scope).unwrap_or(0);
        if let Some(timers) = worker_isolate_timer_queues(scope) {
            timers.clear_pending_and_active(id);
        }
    }
}

pub(super) fn worker_timer_delay_ms<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    prefix: &'static str,
) -> u64 {
    u64::from(webidl::timer_milliseconds_arg(scope, args, 1, prefix))
}

pub(super) fn worker_clear_interval_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    // clearInterval is the same as clearTimeout in our implementation.
    worker_clear_timeout_callback(scope, args, rv);
}

pub(super) fn worker_request_animation_frame_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = get_worker_state(scope) else {
        rv.set_uint32(0);
        return;
    };
    let Some(parsed) = webidl::parse_args::<WorkerRequestAnimationFrameArgs>(scope, &args) else {
        return;
    };

    let timer_id = {
        let mut s = state.borrow_mut();
        s.next_timer_id += 1;
        s.next_timer_id
    };
    let target_timestamp = monotonic_unix_epoch_millis() + 16.0;
    let timer_info = TimerInfo {
        id: timer_id,
        callback: WorkerTimerCallback::webidl_animation_frame(
            scope,
            parsed.callback,
            target_timestamp,
        ),
        delay_ms: 16,
        is_interval: false,
        extra_args: Vec::new(),
    };
    if let Some(timers) = worker_isolate_timer_queues(scope) {
        timers.push_pending(timer_info);
    }
    rv.set_uint32(timer_id);
}
