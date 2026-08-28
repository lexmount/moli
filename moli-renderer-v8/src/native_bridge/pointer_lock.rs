use super::{
    JsContextHost,
    abort::dom_exception_value,
    element::{construct_simple_event, dispatch_public_event},
    node::{
        node_is_document, node_runtime_and_handle_from_args_or_detached,
        node_runtime_and_handle_from_object_or_detached, require_element_method_receiver,
        throw_incompatible_getter_receiver, throw_incompatible_method_receiver,
    },
};
use crate::{
    document_runtime::DomHandle, host::HostTimerOwner, util::context_host_ptr_from_global_bridge,
    webidl,
};

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "PointerLockOptions")]
struct PointerLockOptions {
    #[webidl(name = "unadjustedMovement", default = false)]
    unadjusted_movement: bool,
}

#[derive(Clone, Copy)]
#[repr(i32)]
enum PointerLockFailure {
    WrongDocument = 0,
    NotAllowed = 1,
    NotSupported = 2,
}

impl PointerLockFailure {
    fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::WrongDocument),
            1 => Some(Self::NotAllowed),
            2 => Some(Self::NotSupported),
            _ => None,
        }
    }

    fn exception(self) -> (&'static str, &'static str) {
        match self {
            Self::WrongDocument => (
                "WrongDocumentError",
                "Pointer lock cannot be requested for an element outside the active document.",
            ),
            Self::NotAllowed => (
                "NotAllowedError",
                "Pointer lock requires transient user activation.",
            ),
            Self::NotSupported => (
                "NotSupportedError",
                "Pointer lock is not supported by this headless platform.",
            ),
        }
    }
}

pub(crate) fn element_request_pointer_lock_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Element", "requestPointerLock");
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !require_element_method_receiver(scope, runtime, handle, "requestPointerLock") {
        return;
    }
    let options = match pointer_lock_options(scope, &args) {
        Ok(options) => options,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    // Parsing the dictionary remains observable even though neither adjusted nor raw
    // platform pointer confinement is available in the headless input backend.
    let _ = options.unadjusted_movement;

    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());

    let owner_document = runtime.dom_host().owner_document_handle(handle);
    let active_document = runtime.document_handle();
    if owner_document != Some(active_document) || !runtime.dom_host().is_connected(handle) {
        queue_pointer_lock_failure(
            scope,
            runtime_ptr,
            owner_document.unwrap_or(active_document),
            resolver,
            PointerLockFailure::WrongDocument,
        );
        return;
    }
    let failure = if runtime.transient_user_activation() {
        PointerLockFailure::NotSupported
    } else {
        PointerLockFailure::NotAllowed
    };
    queue_pointer_lock_failure(scope, runtime_ptr, active_document, resolver, failure);
}

pub(crate) fn document_pointer_lock_element_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, document)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        throw_incompatible_getter_receiver(scope, "Document", "pointerLockElement");
        return;
    };
    if !node_is_document(unsafe { &*runtime_ptr }, document) {
        throw_incompatible_getter_receiver(scope, "Document", "pointerLockElement");
        return;
    }
    rv.set_null();
}

pub(crate) fn document_exit_pointer_lock_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, document)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        throw_incompatible_method_receiver(scope, "Document", "exitPointerLock");
        return;
    };
    if !node_is_document(unsafe { &*runtime_ptr }, document) {
        throw_incompatible_method_receiver(scope, "Document", "exitPointerLock");
        return;
    }
    rv.set_undefined();
}

fn pointer_lock_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<PointerLockOptions, webidl::WebIdlError> {
    let context = webidl::Context::argument("Element.requestPointerLock", 1);
    webidl::dictionary_arg(args, 0, context)?
        .map(|object| webidl::parse_dictionary_object(scope, object))
        .transpose()
        .map(|options| options.unwrap_or_default())
}

fn queue_pointer_lock_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    document: DomHandle,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    failure: PointerLockFailure,
) {
    let data = v8::Array::new(scope, 3);
    let document = v8::BigInt::new_from_u64(scope, document.index() as u64);
    let failure = v8::Integer::new(scope, failure as i32);
    if data.set_index(scope, 0, document.into()) != Some(true)
        || data.set_index(scope, 1, resolver.into()) != Some(true)
        || data.set_index(scope, 2, failure.into()) != Some(true)
    {
        reject_pointer_lock_promise(scope, resolver, PointerLockFailure::NotSupported);
        return;
    }
    let Some(callback) = v8::Function::builder(queued_pointer_lock_failure_callback)
        .data(data.into())
        .build(scope)
    else {
        reject_pointer_lock_promise(scope, resolver, PointerLockFailure::NotSupported);
        return;
    };
    let _ = unsafe { &mut *runtime_ptr }.queue_timeout(
        scope,
        callback,
        0,
        HostTimerOwner::Window,
        Vec::new(),
    );
}

fn queued_pointer_lock_failure_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some((document, resolver, failure)) = pointer_lock_failure_task_data(scope, args.data())
    else {
        return;
    };
    if let Some(event) = construct_simple_event(scope, "pointerlockerror", false, false, false) {
        let _ = dispatch_public_event(scope, runtime_ptr, document, event);
    }
    reject_pointer_lock_promise(scope, resolver, failure);
}

fn pointer_lock_failure_task_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<(
    DomHandle,
    v8::Local<'s, v8::PromiseResolver>,
    PointerLockFailure,
)> {
    let data = v8::Local::<v8::Array>::try_from(value).ok()?;
    let document = dom_handle_from_value(data.get_index(scope, 0)?)?;
    let resolver = promise_resolver_from_value(data.get_index(scope, 1)?)?;
    let failure = data.get_index(scope, 2)?.int32_value(scope)?;
    Some((document, resolver, PointerLockFailure::from_i32(failure)?))
}

fn reject_pointer_lock_promise(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    failure: PointerLockFailure,
) {
    let (name, message) = failure.exception();
    let error = dom_exception_value(scope, message, name);
    let _ = resolver.reject(scope, error);
}

fn dom_handle_from_value(value: v8::Local<'_, v8::Value>) -> Option<DomHandle> {
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (index, lossless) = value.u64_value();
    lossless.then(|| DomHandle::new(index as usize))
}

fn promise_resolver_from_value<'s>(
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::PromiseResolver>> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    Some(unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(object) })
}
