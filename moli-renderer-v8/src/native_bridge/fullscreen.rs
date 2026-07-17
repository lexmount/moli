use super::{
    JsContextHost,
    element::{construct_simple_event, dispatch_public_event},
    node::{
        node_is_document, node_runtime_and_handle_from_args_or_detached,
        node_runtime_and_handle_from_object_or_detached, require_element_method_receiver,
        throw_incompatible_method_receiver,
    },
};
use crate::{
    document_runtime::DomHandle,
    host::HostTimerOwner,
    util::{context_host_ptr_from_global_bridge, v8_string},
    webidl,
};

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
#[webidl(name = "FullscreenKeyboardLock")]
enum FullscreenKeyboardLock {
    #[webidl(token = "browser")]
    Browser,
    #[default]
    #[webidl(token = "none")]
    None,
}

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
#[webidl(name = "FullscreenNavigationUI")]
enum FullscreenNavigationUi {
    #[default]
    #[webidl(token = "auto")]
    Auto,
    #[webidl(token = "show")]
    Show,
    #[webidl(token = "hide")]
    Hide,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "FullscreenOptions")]
struct FullscreenOptions {
    #[webidl(
        name = "keyboardLock",
        converter = "enum",
        default = FullscreenKeyboardLock::None
    )]
    keyboard_lock: FullscreenKeyboardLock,
    #[webidl(
        name = "navigationUI",
        converter = "enum",
        default = FullscreenNavigationUi::Auto
    )]
    navigation_ui: FullscreenNavigationUi,
}

pub(crate) fn element_request_fullscreen_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, element)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Element", "requestFullscreen");
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !require_element_method_receiver(scope, runtime, element, "requestFullscreen") {
        return;
    }
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());

    let options = match fullscreen_options_for_promise(scope, &args) {
        Ok(options) => options,
        Err(reason) => {
            let _ = resolver.reject(scope, reason);
            return;
        }
    };
    // Dictionary conversion is observable even though this headless platform
    // deliberately reports that fullscreen is unsupported.
    let _ = (options.keyboard_lock, options.navigation_ui);

    let active_document = runtime.document_handle();
    let Some(owner_document) = runtime.dom_host().owner_document_handle(element) else {
        reject_fullscreen_promise(scope, resolver, "Element has no node document.");
        return;
    };
    if owner_document != active_document {
        reject_fullscreen_promise(scope, resolver, "Document is not fully active.");
        return;
    }

    queue_fullscreen_error_event(scope, runtime_ptr, element, owner_document);
    reject_fullscreen_promise(scope, resolver, "Fullscreen is not supported.");
}

pub(crate) fn document_exit_fullscreen_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, document)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        throw_incompatible_method_receiver(scope, "Document", "exitFullscreen");
        return;
    };
    if !node_is_document(unsafe { &*runtime_ptr }, document) {
        throw_incompatible_method_receiver(scope, "Document", "exitFullscreen");
        return;
    }
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());
    reject_fullscreen_promise(scope, resolver, "Document is not in fullscreen.");
}

fn fullscreen_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<FullscreenOptions, webidl::WebIdlError> {
    let context = webidl::Context::argument("Element.requestFullscreen", 1);
    webidl::dictionary_arg(args, 0, context)?
        .map(|object| webidl::parse_dictionary_object(scope, object))
        .transpose()
        .map(|options| options.unwrap_or_default())
}

fn fullscreen_options_for_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<FullscreenOptions, v8::Local<'s, v8::Value>> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    match fullscreen_options(&mut scope, args) {
        Ok(options) => Ok(options),
        Err(error) if error.is_pending_exception() => Err(scope
            .exception()
            .unwrap_or_else(|| v8::undefined(&scope).into())),
        Err(error) => {
            let message =
                v8_string(&scope, &error.to_string()).unwrap_or_else(|| v8::String::empty(&scope));
            Err(v8::Exception::type_error(&scope, message))
        }
    }
}

fn queue_fullscreen_error_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    element: DomHandle,
    document: DomHandle,
) {
    let data = v8::Array::new(scope, 2);
    let element = v8::BigInt::new_from_u64(scope, element.index() as u64);
    let document = v8::BigInt::new_from_u64(scope, document.index() as u64);
    if data.set_index(scope, 0, element.into()) != Some(true)
        || data.set_index(scope, 1, document.into()) != Some(true)
    {
        return;
    }
    let Some(callback) = v8::Function::builder(queued_fullscreen_error_event_callback)
        .data(data.into())
        .build(scope)
    else {
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

fn queued_fullscreen_error_event_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let Some((element, document)) = fullscreen_error_event_task_data(scope, args.data()) else {
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let target = if runtime.dom_host().owner_document_handle(element) == Some(document)
        && runtime.dom_host().is_connected_to_document(element)
    {
        element
    } else {
        document
    };
    if let Some(event) = construct_simple_event(scope, "fullscreenerror", true, false, true) {
        let _ = dispatch_public_event(scope, runtime_ptr, target, event);
    }
}

fn fullscreen_error_event_task_data(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Option<(DomHandle, DomHandle)> {
    let data = v8::Local::<v8::Array>::try_from(value).ok()?;
    Some((
        dom_handle_from_value(data.get_index(scope, 0)?)?,
        dom_handle_from_value(data.get_index(scope, 1)?)?,
    ))
}

fn reject_fullscreen_promise(
    scope: &mut v8::PinScope<'_, '_>,
    resolver: v8::Local<'_, v8::PromiseResolver>,
    message: &str,
) {
    let error = v8_string(scope, message)
        .map(|message| v8::Exception::type_error(scope, message))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let _ = resolver.reject(scope, error);
}

fn dom_handle_from_value(value: v8::Local<'_, v8::Value>) -> Option<DomHandle> {
    let value = v8::Local::<v8::BigInt>::try_from(value).ok()?;
    let (index, lossless) = value.u64_value();
    lossless.then(|| DomHandle::new(index as usize))
}
