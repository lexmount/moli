//! Observable subscriptions share one producer until their last observer leaves.
//! Callback residence is V8-traced; the Observable's link to its Subscriber is
//! weak. AbortSignal state and callback invocation remain with their existing
//! Window/worker owners.

mod callbacks;
mod event_target;
mod state;

pub(crate) use event_target::event_target_when;

use moli_webapi_declare::WebApiFunctionTemplate;

use crate::{
    abort_signal_route::ResolvedAbortSignal, util::set_private_value, web_api_interfaces, webidl,
};
use callbacks::{invoke, is_current, report};
use state::*;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Observable, enumerable, receiver)]
struct ObservablePrototype {
    #[webapi(method, length = 0, callback = subscribe)]
    subscribe: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Subscriber, enumerable, receiver)]
struct SubscriberPrototype {
    #[webapi(method, length = 1, callback = next)]
    next: (),
    #[webapi(method, length = 1, callback = error)]
    error: (),
    #[webapi(method, length = 0, callback = complete)]
    complete: (),
    #[webapi(method, length = 1, callback = add_teardown)]
    add_teardown: (),
    #[webapi(accessor_property, getter = active_getter)]
    active: (),
    #[webapi(accessor_property, getter = signal_getter)]
    signal: (),
}

pub(crate) fn install_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    let prototype = template.prototype_template(scope);
    match name {
        "Observable" => ObservablePrototype::initialize_prototype_template(scope, prototype),
        "Subscriber" => SubscriberPrototype::initialize_prototype_template(scope, prototype),
        _ => {}
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable")]
struct ConstructorArgs {
    #[webidl(required, converter = "callback_function")]
    callback: webidl::WebIdlCallbackFunction,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Observable.subscribe")]
struct SubscribeArgs<'scope> {
    #[webidl(with = observer_arg)]
    observer: v8::Local<'scope, v8::Object>,
    #[webidl(with = signal_arg)]
    signal: Option<ResolvedAbortSignal<'scope>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Subscriber")]
struct ValueArgs<'scope> {
    #[webidl(required, converter = "raw")]
    value: v8::Local<'scope, v8::Value>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Subscriber.addTeardown")]
struct TeardownArgs {
    #[webidl(required, converter = "callback_function")]
    callback: webidl::WebIdlCallbackFunction,
}

fn dictionary<'s>(
    value: v8::Local<'s, v8::Value>,
    message: &'static str,
) -> Result<Option<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    v8::Local::<v8::Object>::try_from(value)
        .map(Some)
        .map_err(|_| webidl::WebIdlError::custom_message(message))
}

fn observer_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    let observer = v8::Object::new(scope);
    let Some(input) = dictionary(args.get(index), "SubscriptionObserver must be an object")? else {
        return Ok(observer);
    };
    if input.is_callable() {
        let callback = webidl::convert::<webidl::WebIdlCallbackFunction>(
            scope,
            input.into(),
            webidl::Context::argument("Observable.subscribe", 1),
        )?;
        set_callback(scope, observer, NEXT, callback);
        return Ok(observer);
    }
    // Web IDL dictionary member conversion is lexicographic, even though the
    // subscription itself delivers next/error/complete notifications.
    for (name, slot) in [("complete", COMPLETE), ("error", ERROR), ("next", NEXT)] {
        let context = webidl::Context::member("SubscriptionObserver", name);
        if let Some(value) = webidl::property_result(scope, input, name, context)?
            && !value.is_undefined()
        {
            let callback =
                webidl::convert::<webidl::WebIdlCallbackFunction>(scope, value, context)?;
            set_callback(scope, observer, slot, callback);
        }
    }
    Ok(observer)
}

fn signal_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<Option<ResolvedAbortSignal<'s>>, webidl::WebIdlError> {
    let Some(options) = dictionary(args.get(index), "SubscribeOptions must be an object")? else {
        return Ok(None);
    };
    let context = webidl::Context::member("SubscribeOptions", "signal");
    let Some(value) = webidl::property_result(scope, options, "signal", context)? else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    v8::Local::<v8::Object>::try_from(value)
        .ok()
        .filter(|signal| web_api_interfaces::AbortSignal::is_instance(scope, *signal))
        .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))
        .map(Some)
        .ok_or_else(|| {
            webidl::WebIdlError::custom_message("SubscribeOptions.signal must be an AbortSignal")
        })
}

pub(crate) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        webidl::throw_type_error(scope, "Observable must be constructed with new");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    initialize_observable(scope, args.this(), parsed.callback);
    rv.set(args.this().into());
}

fn subscribe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SubscribeArgs<'s>>(scope, &args) else {
        return;
    };
    let observable = args.this();
    if !is_current(scope, observable) {
        return;
    }
    let (subscriber, fresh) = match current_subscriber(scope, observable)
        .filter(|subscriber| active(scope, *subscriber))
    {
        Some(subscriber) => (subscriber, false),
        None => {
            let Some(subscriber) = new_subscriber(scope) else {
                return;
            };
            set_subscriber(scope, observable, subscriber);
            (subscriber, true)
        }
    };
    let mut observers = list(scope, subscriber, OBSERVERS);
    observers.push(parsed.observer);
    set_list(scope, subscriber, OBSERVERS, &observers);
    if let Some(signal) = parsed.signal {
        if signal.is_aborted(scope) {
            if fresh {
                let reason = signal.reason(scope);
                close(scope, subscriber, Some(reason));
            } else {
                observers.pop();
                set_list(scope, subscriber, OBSERVERS, &observers);
            }
        } else {
            let data =
                v8::Array::new_with_elements(scope, &[subscriber.into(), parsed.observer.into()]);
            let algorithm = v8::Function::builder(cancel_observer)
                .data(data.into())
                .build(scope)
                .expect("Observable abort algorithm should allocate");
            set_private_value(scope, parsed.observer, INPUT_SIGNAL, signal.value().into());
            set_private_value(scope, parsed.observer, ABORT_ALGORITHM, algorithm.into());
            signal.register_algorithm(scope, algorithm);
        }
    }
    if fresh {
        if let Some(callback) = object_slot(scope, observable, INITIALIZER) {
            if let Some(exception) = invoke(scope, callback, &[subscriber.into()]) {
                subscriber_error(scope, subscriber, exception);
            }
        } else {
            event_target::subscribe(scope, observable, subscriber);
        }
    }
}

fn cancel_observer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let data = v8::Local::<v8::Array>::try_from(args.data()).expect("Observable abort data");
    let subscriber = v8::Local::<v8::Object>::try_from(data.get_index(scope, 0).unwrap()).unwrap();
    let observer = v8::Local::<v8::Object>::try_from(data.get_index(scope, 1).unwrap()).unwrap();
    if !active(scope, subscriber) {
        return;
    }
    let mut observers = list(scope, subscriber, OBSERVERS);
    observers.retain(|entry| *entry != observer);
    set_list(scope, subscriber, OBSERVERS, &observers);
    release_abort_algorithm(scope, observer);
    if observers.is_empty() {
        close(scope, subscriber, Some(args.get(0)));
    }
}

fn close<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    subscriber: v8::Local<'s, v8::Object>,
    reason: Option<v8::Local<'s, v8::Value>>,
) {
    if !active(scope, subscriber) {
        return;
    }
    set_private_value(
        scope,
        subscriber,
        ACTIVE,
        v8::Boolean::new(scope, false).into(),
    );
    for observer in list(scope, subscriber, OBSERVERS) {
        release_abort_algorithm(scope, observer);
    }
    let signal = object_slot(scope, subscriber, SIGNAL)
        .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal));
    let reason = reason
        .filter(|reason| !reason.is_undefined())
        .unwrap_or_else(|| crate::native_bridge::abort::abort_error_value(scope));
    if let Some(signal) = signal {
        signal.abort(scope, reason);
    }
    let teardowns = list(scope, subscriber, TEARDOWNS);
    set_list(scope, subscriber, TEARDOWNS, &[]);
    for callback in teardowns.into_iter().rev() {
        if !is_current(scope, subscriber) {
            break;
        }
        invoke_and_report(scope, callback, &[]);
    }
}

fn release_abort_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) {
    if let Some(signal) = object_slot(scope, observer, INPUT_SIGNAL)
        .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))
        && let Some(algorithm) = object_slot(scope, observer, ABORT_ALGORITHM)
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    {
        signal.unregister_algorithm(scope, algorithm);
    }
    set_private_value(scope, observer, INPUT_SIGNAL, v8::undefined(scope).into());
    set_private_value(
        scope,
        observer,
        ABORT_ALGORITHM,
        v8::undefined(scope).into(),
    );
}

fn invoke_and_report<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: v8::Local<'s, v8::Object>,
    values: &[v8::Local<'s, v8::Value>],
) {
    if let Some(error) = invoke(scope, callback, values) {
        callbacks::report_callback_exception(scope, callback, error);
    }
}

fn next<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ValueArgs<'s>>(scope, &args) else {
        return;
    };
    subscriber_next(scope, args.this(), parsed.value);
}

fn subscriber_next<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    subscriber: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    if !active(scope, subscriber) || !is_current(scope, subscriber) {
        return;
    }
    // Reentrant subscribe/cancel must not change this notification's snapshot.
    for observer in list(scope, subscriber, OBSERVERS) {
        if let Some(callback) = object_slot(scope, observer, NEXT) {
            invoke_and_report(scope, callback, &[value]);
        }
    }
}

fn subscriber_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    subscriber: v8::Local<'s, v8::Object>,
    error: v8::Local<'s, v8::Value>,
) {
    if !active(scope, subscriber) {
        report(scope, subscriber, error);
        return;
    }
    if !is_current(scope, subscriber) {
        return;
    }
    close(scope, subscriber, Some(error));
    let observers = list(scope, subscriber, OBSERVERS);
    set_list(scope, subscriber, OBSERVERS, &[]);
    for observer in observers {
        if let Some(callback) = object_slot(scope, observer, ERROR) {
            invoke_and_report(scope, callback, &[error]);
        } else {
            callbacks::report_default_error(scope, error);
        }
    }
}

fn error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ValueArgs<'s>>(scope, &args) else {
        return;
    };
    subscriber_error(scope, args.this(), parsed.value);
}

fn complete<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let subscriber = args.this();
    if !active(scope, subscriber) || !is_current(scope, subscriber) {
        return;
    }
    close(scope, subscriber, None);
    let observers = list(scope, subscriber, OBSERVERS);
    set_list(scope, subscriber, OBSERVERS, &[]);
    for observer in observers {
        if let Some(callback) = object_slot(scope, observer, COMPLETE) {
            invoke_and_report(scope, callback, &[]);
        }
    }
}

fn add_teardown<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<TeardownArgs>(scope, &args) else {
        return;
    };
    let subscriber = args.this();
    if !is_current(scope, subscriber) {
        return;
    }
    let callback = callbacks::trace(scope, parsed.callback);
    if active(scope, subscriber) {
        let mut teardowns = list(scope, subscriber, TEARDOWNS);
        teardowns.push(callback);
        set_list(scope, subscriber, TEARDOWNS, &teardowns);
    } else {
        invoke_and_report(scope, callback, &[]);
    }
}

fn active_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_bool(active(scope, args.this()));
}

fn signal_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(signal) = object_slot(scope, args.this(), SIGNAL) {
        rv.set(signal.into());
    }
}
