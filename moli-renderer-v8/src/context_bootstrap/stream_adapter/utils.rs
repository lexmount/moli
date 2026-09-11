use super::*;
use moli_webapi_declare::WebApiObject;

const DELAYED_PENDING_READ_REJECT_SLOT: &str = "__moliReadableStreamDelayedReject";
const DELAYED_PENDING_READ_REASON_SLOT: &str = "__moliReadableStreamDelayedReason";

#[derive(WebApiObject)]
#[webapi(plain)]
struct StreamIteratorResultDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    value: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    done: bool,
}

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct PendingReadEntryDeclaration {
    #[webapi(slot = READABLE_STREAM_PENDING_READ_PROMISE_SLOT, init = "undefined")]
    promise: (),
    #[webapi(slot = READABLE_STREAM_PENDING_READ_RESOLVE_SLOT, init = "undefined")]
    resolve: (),
    #[webapi(slot = READABLE_STREAM_PENDING_READ_REJECT_SLOT, init = "undefined")]
    reject: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct PendingReadPromiseDeclaration<'scope> {
    #[webapi(slot = READABLE_STREAM_PENDING_READ_PROMISE_SLOT)]
    promise: v8::Local<'scope, v8::Promise>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct PendingReadResolverDeclaration<'scope> {
    #[webapi(slot = READABLE_STREAM_PENDING_READ_RESOLVE_SLOT)]
    resolve: v8::Local<'scope, v8::Value>,
    #[webapi(slot = READABLE_STREAM_PENDING_READ_REJECT_SLOT)]
    reject: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct DelayedPendingReadRejectDeclaration<'scope> {
    #[webapi(slot = DELAYED_PENDING_READ_REJECT_SLOT)]
    reject: v8::Local<'scope, v8::Function>,
    #[webapi(slot = DELAYED_PENDING_READ_REASON_SLOT)]
    reason: v8::Local<'scope, v8::Value>,
}

/// Materialize a native callback required to complete a Streams state-machine
/// transition. Building the function does not invoke author JavaScript, and
/// there is no Web Streams recovery semantics for a missing internal callback.
/// Continuing would leave an in-flight operation permanently unsettled, so a
/// live process must fail fast instead of silently returning.
pub(in crate::context_bootstrap) fn build_required_stream_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    builder: v8::FunctionBuilder<'s, v8::Function>,
    role: &'static str,
) -> StreamOwnerPublication<v8::Local<'s, v8::Function>> {
    let callback = builder.build(scope);
    publish_required_stream_value(scope, callback, "callback creation", role)
}

/// Result of publishing required V8 machinery for an internal Streams
/// operation into its realm owner.
///
/// V8 returns an empty `MaybeLocal` when `Worker.terminate()` interrupts the
/// publication. That is not an allocation failure: the worker lifecycle owner
/// abandons all realm-local continuations, and in particular must not turn a
/// terminated transferred-stream worker into a peer-visible stream error.
/// Generic V8 execution termination is deliberately insufficient here because
/// the page watchdog can recover and continue using the same realm.
#[must_use = "Streams publication must distinguish live machinery from realm teardown"]
pub(in crate::context_bootstrap) enum StreamOwnerPublication<T> {
    Published(T),
    OwnerTerminating,
}

impl<T> StreamOwnerPublication<T> {
    /// Consume a publication at a call boundary that has no later local
    /// effects. Both outcomes end the operation: the published machinery owns
    /// continuation in a live realm, while Worker teardown owns abandonment.
    pub(in crate::context_bootstrap) fn finish_at_owner_boundary(self) {
        match self {
            Self::Published(_) | Self::OwnerTerminating => {}
        }
    }
}

/// Build and publish both terminal reactions required by an in-flight Streams
/// operation as one transaction. Native `Promise::then2` does not invoke either
/// reaction synchronously. An empty result in a live realm is therefore an
/// internal machinery failure and must fail fast. Worker termination is an
/// explicit abandonment outcome owned by realm teardown rather than by the
/// stream state machine, regardless of whether it interrupts callback creation
/// or the final `then2()` attachment.
pub(in crate::context_bootstrap) fn publish_required_stream_promise_reactions<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    promise: v8::Local<'s, v8::Promise>,
    on_fulfilled: v8::FunctionBuilder<'s, v8::Function>,
    fulfillment_role: &'static str,
    on_rejected: v8::FunctionBuilder<'s, v8::Function>,
    rejection_role: &'static str,
    role: &'static str,
) -> StreamOwnerPublication<v8::Local<'s, v8::Promise>> {
    let StreamOwnerPublication::Published(on_fulfilled) =
        build_required_stream_callback(scope, on_fulfilled, fulfillment_role)
    else {
        return StreamOwnerPublication::OwnerTerminating;
    };
    let StreamOwnerPublication::Published(on_rejected) =
        build_required_stream_callback(scope, on_rejected, rejection_role)
    else {
        return StreamOwnerPublication::OwnerTerminating;
    };
    if let Some(attached) = promise.then2(scope, on_fulfilled, on_rejected) {
        return StreamOwnerPublication::Published(attached);
    }
    publish_required_stream_value(scope, None, "promise reaction attachment", role)
}

fn publish_required_stream_value<T>(
    scope: &mut v8::PinScope<'_, '_>,
    value: Option<T>,
    operation: &'static str,
    role: &'static str,
) -> StreamOwnerPublication<T> {
    match value {
        Some(value) => StreamOwnerPublication::Published(value),
        None if crate::worker::worker_termination_requested(scope) => {
            StreamOwnerPublication::OwnerTerminating
        }
        None => fail_internal_stream_operation(operation, role),
    }
}

#[track_caller]
pub(crate) fn require_internal_stream_value<T>(
    value: Option<T>,
    operation: &'static str,
    role: &'static str,
) -> T {
    value.unwrap_or_else(|| fail_internal_stream_operation(operation, role))
}

#[track_caller]
fn fail_internal_stream_operation(operation: &'static str, role: &'static str) -> ! {
    panic!("required internal Streams {operation} for `{role}` must not fail silently")
}

pub(in crate::context_bootstrap) fn iter_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    done: bool,
) -> v8::Local<'s, v8::Object> {
    StreamIteratorResultDeclaration::new(value, done)
        .bind(scope)
        .expect("stream iterator result declaration should bind")
}

pub(in crate::context_bootstrap) fn done_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    iter_result(scope, v8::undefined(scope).into(), true)
}

pub(in crate::context_bootstrap) fn new_pending_read_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<(v8::Local<'s, v8::Promise>, v8::Local<'s, v8::Object>)> {
    let entry = PendingReadEntryDeclaration::default()
        .bind(scope)
        .expect("pending read entry declaration should bind");
    let executor = v8::Function::builder(pending_read_promise_executor_callback)
        .data(entry.into())
        .length(2)
        .build(scope)?;
    let global = scope.get_current_context().global(scope);
    let promise_constructor = global
        .get(scope, v8str(scope, "Promise").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let promise = promise_constructor
        .new_instance(scope, &[executor.into()])
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())?;
    PendingReadPromiseDeclaration::new(promise)
        .initialize(scope, entry)
        .ok()?;
    get_private_value(scope, entry, READABLE_STREAM_PENDING_READ_RESOLVE_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    get_private_value(scope, entry, READABLE_STREAM_PENDING_READ_REJECT_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    Some((promise, entry))
}

fn pending_read_promise_executor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(entry) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    PendingReadResolverDeclaration::new(args.get(0), args.get(1))
        .initialize(scope, entry)
        .expect("pending read resolver declaration should initialize entry");
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn resolve_pending_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(resolve) = get_private_value(scope, entry, READABLE_STREAM_PENDING_READ_RESOLVE_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let undefined = v8::undefined(scope);
    let _ = resolve.call(scope, undefined.into(), &[value]);
}

pub(in crate::context_bootstrap) fn reject_pending_read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) {
    let Some(reject) = get_private_value(scope, entry, READABLE_STREAM_PENDING_READ_REJECT_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let undefined = v8::undefined(scope);
    let _ = reject.call(scope, undefined.into(), &[reason]);
}

pub(in crate::context_bootstrap::stream_adapter) fn reject_pending_read_after_timeout<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    reason: v8::Local<'s, v8::Value>,
) -> bool {
    let Some(reject) = get_private_value(scope, entry, READABLE_STREAM_PENDING_READ_REJECT_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return false;
    };
    let data = DelayedPendingReadRejectDeclaration::new(reject, reason)
        .bind(scope)
        .expect("delayed pending read rejection declaration should bind");
    let Some(callback) = v8::Function::builder(delayed_pending_read_reject_callback)
        .data(data.into())
        .build(scope)
    else {
        return false;
    };
    let global = scope.get_current_context().global(scope);
    let Some(set_timeout) = global
        .get(scope, v8str(scope, "setTimeout").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return false;
    };
    let delay = v8::Number::new(scope, 0.0);
    set_timeout
        .call(scope, global.into(), &[callback.into(), delay.into()])
        .is_some()
}

fn delayed_pending_read_reject_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(data) = v8::Local::<v8::Object>::try_from(args.data()) else {
        rv.set_undefined();
        return;
    };
    let Some(reject) = get_private_value(scope, data, DELAYED_PENDING_READ_REJECT_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        rv.set_undefined();
        return;
    };
    let reason = get_private_value(scope, data, DELAYED_PENDING_READ_REASON_SLOT)
        .unwrap_or_else(|| v8::undefined(scope).into());
    let undefined = v8::undefined(scope);
    let _ = reject.call(scope, undefined.into(), &[reason]);
    rv.set_undefined();
}

pub(in crate::context_bootstrap) fn suppress_pending_read_unhandled_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) {
    let Some(promise) = get_private_value(scope, entry, READABLE_STREAM_PENDING_READ_PROMISE_SLOT)
        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())
    else {
        return;
    };
    suppress_promise_unhandled_rejection(scope, promise.into());
}

pub(in crate::context_bootstrap) fn suppress_promise_unhandled_rejection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    promise: v8::Local<'s, v8::Value>,
) {
    let Ok(promise) = v8::Local::<v8::Promise>::try_from(promise) else {
        return;
    };
    let Some(noop) = v8::Function::builder(promise_return_undefined_callback).build(scope) else {
        return;
    };
    let Some(catch) = promise
        .get(scope, v8str(scope, "catch").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let _ = catch.call(scope, promise.into(), &[noop.into()]);
}

pub(in crate::context_bootstrap) fn value_buffer_source_bytes(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<Vec<u8>> {
    if let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) {
        let view = v8::Uint8Array::new(scope, buffer, 0, buffer.byte_length())?;
        let mut bytes = vec![0; view.byte_length()];
        let written = view.copy_contents(&mut bytes);
        bytes.truncate(written);
        return Some(bytes);
    }
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        let mut bytes = vec![0; view.byte_length()];
        let written = view.copy_contents(&mut bytes);
        bytes.truncate(written);
        return Some(bytes);
    }
    None
}

pub(in crate::context_bootstrap) fn call_named_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    argv: &[v8::Local<'s, v8::Value>],
) -> Option<v8::Local<'s, v8::Value>> {
    let value = object.get(scope, v8_string(scope, name)?.into())?;
    let function = v8::Local::<v8::Function>::try_from(value).ok()?;
    function.call(scope, object.into(), argv)
}

pub(in crate::context_bootstrap) fn call_named_method_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    argv: &[v8::Local<'s, v8::Value>],
) -> Result<Option<v8::Local<'s, v8::Value>>, v8::Local<'s, v8::Value>> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let scope = try_catch.init();
    let Some(name_key) = v8_string(&scope, name) else {
        return Ok(None);
    };
    let value = match object.get(&scope, name_key.into()) {
        Some(value) => value,
        None if scope.has_caught() => {
            return Err(scope
                .exception()
                .unwrap_or_else(|| v8::undefined(&scope).into()));
        }
        None => return Ok(None),
    };
    let Ok(function) = v8::Local::<v8::Function>::try_from(value) else {
        return Ok(None);
    };
    match function.call(&scope, object.into(), argv) {
        Some(value) => Ok(Some(value)),
        None if scope.has_caught() => Err(scope
            .exception()
            .unwrap_or_else(|| v8::undefined(&scope).into())),
        None => Ok(None),
    }
}

pub(in crate::context_bootstrap) fn call_function_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    function: v8::Local<'s, v8::Function>,
    receiver: v8::Local<'s, v8::Value>,
    argv: &[v8::Local<'s, v8::Value>],
) -> Result<Option<v8::Local<'s, v8::Value>>, v8::Local<'s, v8::Value>> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let scope = try_catch.init();
    match function.call(&scope, receiver, argv) {
        Some(value) => Ok(Some(value)),
        None if scope.has_caught() => Err(scope
            .exception()
            .unwrap_or_else(|| v8::undefined(&scope).into())),
        None => Ok(None),
    }
}

pub(in crate::context_bootstrap) fn resolve_callable_property_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<Option<v8::Local<'s, v8::Function>>, v8::Local<'s, v8::Value>> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let scope = try_catch.init();
    let method_name = name;
    let Some(name) = v8_string(&scope, method_name) else {
        return Ok(None);
    };
    let value = match object.get(&scope, name.into()) {
        Some(value) => value,
        None if scope.has_caught() => {
            return Err(scope
                .exception()
                .unwrap_or_else(|| v8::undefined(&scope).into()));
        }
        None => return Ok(None),
    };
    if value.is_undefined() {
        return Ok(None);
    }
    let Ok(function) = v8::Local::<v8::Function>::try_from(value) else {
        let message = match method_name {
            "cancel" => "underlyingSource.cancel must be a function or undefined",
            "pull" => "underlyingSource.pull must be a function or undefined",
            _ => "underlyingSource method must be a function or undefined",
        };
        return Err(v8::Exception::type_error(&scope, v8str(&scope, message)));
    };
    Ok(Some(function))
}

pub(in crate::context_bootstrap) fn set_resolved_promise(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    value: v8::Local<'_, v8::Value>,
) {
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        rv.set_undefined();
        return;
    };
    let promise = resolver.get_promise(scope);
    let _ = resolver.resolve(scope, value);
    rv.set(promise.into());
}

pub(in crate::context_bootstrap) fn resolved_promise_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let _ = resolver.resolve(scope, value);
    Some(promise.into())
}

pub(in crate::context_bootstrap) fn rejected_promise_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    reason: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let _ = resolver.reject(scope, reason);
    Some(promise.into())
}

pub(in crate::context_bootstrap) fn promise_then_undefined<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    promise: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    let promise_object = v8::Local::<v8::Object>::try_from(promise).ok()?;
    let then = promise_object
        .get(scope, v8str(scope, "then").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let on_fulfilled = v8::Function::builder(promise_return_undefined_callback)
        .length(0)
        .build(scope)?;
    then.call(scope, promise, &[on_fulfilled.into()])
}

pub(in crate::context_bootstrap) fn promise_return_undefined_callback<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_undefined();
}

#[cfg(test)]
mod tests {
    use super::require_internal_stream_value;

    #[test]
    fn required_internal_stream_value_returns_available_value() {
        assert_eq!(
            require_internal_stream_value(Some(7_u8), "callback creation", "test reaction"),
            7
        );
    }

    #[test]
    #[should_panic(
        expected = "required internal Streams callback creation for `test reaction` must not fail silently"
    )]
    fn required_internal_stream_value_fails_fast_when_missing() {
        require_internal_stream_value::<()>(None, "callback creation", "test reaction");
    }
}
