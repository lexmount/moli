use crate::{
    abort_signal_route::ResolvedAbortSignal,
    event_listener_args::{AddEventListenerArgs, AddEventListenerOptions},
    native_bridge::WindowExecutionContextIdentity,
    util::context_host_ptr_from_global_bridge,
    webidl,
    window_host::{capture_window_event_target_receiver, register_event_target_webidl_listener},
};

use super::{callbacks::is_current, state, subscriber_next};

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ObservableEventListenerOptions")]
struct ObservableEventListenerOptions {
    #[webidl(default = false)]
    capture: bool,
    passive: Option<bool>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "EventTarget.when")]
struct WhenArgs {
    #[webidl(required, name = "type")]
    event_type: String,
    #[webidl(with = options_arg)]
    options: ObservableEventListenerOptions,
}

fn options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<ObservableEventListenerOptions, webidl::WebIdlError> {
    webidl::parse_dictionary(
        scope,
        args.get(index),
        webidl::Context::argument("EventTarget.when", 2),
    )
    .map(Option::unwrap_or_default)
}

/// The source does not own its EventTarget. In particular, keeping an
/// Observable must not keep an otherwise unreachable standalone target alive.
pub(super) struct EventSource {
    target: v8::Weak<v8::Object>,
    event_type: String,
    options: webidl::EventListenerOptions,
    // A WindowProxy can survive navigation; retain only the exact identity,
    // never a strong context or a lookup that follows the replacement Window.
    window_identity: Option<WindowExecutionContextIdentity>,
}

pub(super) struct PreparedEventSource<'s> {
    target: v8::Local<'s, v8::Object>,
    event_type: String,
    options: webidl::EventListenerOptions,
    window_identity: Option<WindowExecutionContextIdentity>,
}

impl EventSource {
    pub(super) fn prepare<'s>(
        &self,
        scope: &v8::PinScope<'s, '_>,
    ) -> Option<PreparedEventSource<'s>> {
        Some(PreparedEventSource {
            target: self.target.to_local(scope)?,
            event_type: self.event_type.clone(),
            options: self.options,
            window_identity: self.window_identity,
        })
    }
}

pub(crate) fn event_target_when<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = args.this();
    // As with addEventListener, authorize and freeze Window identity before
    // author argument conversion can navigate the receiver.
    let window_identity = if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        let host = unsafe { &*host_ptr };
        let Ok(receiver) = capture_window_event_target_receiver(scope, target, host) else {
            return;
        };
        receiver.map(|receiver| {
            receiver
                .resolve_live_binding(host)
                .and_then(|binding| binding.resolve_identity(host))
        })
    } else {
        None
    };
    let Some(call) = webidl::parse_args::<WhenArgs>(scope, &args) else {
        return;
    };
    if window_identity.is_some_and(|identity| identity.is_none()) || !is_current(scope, target) {
        return;
    }
    let window_identity = window_identity.flatten();
    if let Some(identity) = window_identity
        && !context_host_ptr_from_global_bridge(scope).is_some_and(|host_ptr| {
            unsafe { &*host_ptr }.window_execution_context_identity_is_current(identity)
        })
    {
        return;
    }
    let source = EventSource {
        target: v8::Weak::new(scope, target),
        event_type: call.event_type,
        options: webidl::EventListenerOptions {
            capture: call.options.capture,
            passive: call.options.passive,
            once: false,
        },
        window_identity,
    };
    if let Some(observable) = state::new_native_observable(scope, Some(source)) {
        rv.set(observable.into());
    }
}

pub(super) fn subscribe<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observable: v8::Local<'s, v8::Object>,
    subscriber: v8::Local<'s, v8::Object>,
) {
    let Some(source) = state::event_source(scope, observable) else {
        return;
    };
    let Some(signal) = state::object_slot(scope, subscriber, state::SIGNAL)
        .and_then(|signal| ResolvedAbortSignal::resolve(scope, signal))
    else {
        return;
    };
    if signal.is_aborted(scope) {
        return;
    }
    let callback = v8::Function::builder(event_listener)
        .length(1)
        .data(subscriber.into())
        .build(scope)
        .expect("Observable event listener should allocate");
    let listener = webidl::convert::<webidl::WebIdlCallbackInterface>(
        scope,
        callback.into(),
        webidl::Context::argument("EventTarget.when", 1),
    )
    .expect("native Observable event listener must convert");

    if let Some(identity) = source.window_identity {
        let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
            return;
        };
        let host = unsafe { &*host_ptr };
        if !host.window_execution_context_identity_is_current(identity) {
            return;
        }
        let Some(binding) = host
            .clone_window_execution_context_binding(
                scope,
                identity.owner(),
                identity.dispatch_scope(),
            )
            .filter(|binding| binding.resolve_identity(host) == Some(identity))
        else {
            return;
        };
        let target = v8::Global::new(scope, source.target);
        let signal = v8::Global::new(scope, signal.value());
        binding.with_current_scope(scope, host_ptr, |scope, _| {
            let target = v8::Local::new(scope, &target);
            let signal = v8::Local::new(scope, &signal);
            let signal = ResolvedAbortSignal::resolve(scope, signal);
            register_event_target_webidl_listener(
                scope,
                target,
                AddEventListenerArgs {
                    event_type: source.event_type,
                    listener: Some(listener),
                    options: AddEventListenerOptions {
                        options: source.options,
                        signal,
                    },
                },
            );
        });
    } else {
        register_event_target_webidl_listener(
            scope,
            source.target,
            AddEventListenerArgs {
                event_type: source.event_type,
                listener: Some(listener),
                options: AddEventListenerOptions {
                    options: source.options,
                    signal: Some(signal),
                },
            },
        );
    }
}

fn event_listener<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let subscriber = v8::Local::<v8::Object>::try_from(args.data())
        .expect("Observable Subscriber listener data");
    subscriber_next(scope, subscriber, args.get(0));
}
