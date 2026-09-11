use crate::native_bridge::abort;
use crate::web_api_interfaces;
use moli_webapi_declare::{WebApiFunctionTemplate, v8};

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AbortSignal)]
struct AbortSignalConstructorDeclaration {
    #[webapi(
        static_method = "abort",
        length = 1,
        callback = abort::abort_signal_static_abort_callback,
        enumerable
    )]
    abort: (),
    #[webapi(
        static_method = "timeout",
        length = 1,
        callback = abort::abort_signal_timeout_callback,
        enumerable
    )]
    timeout: (),
    #[webapi(
        static_method = "any",
        length = 1,
        callback = abort::abort_signal_any_callback,
        enumerable
    )]
    any: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AbortSignal, enumerable)]
struct AbortSignalPrototypeDeclaration {
    #[webapi(
        method,
        length = 2,
        callback = abort::abort_signal_add_event_listener_callback
    )]
    add_event_listener: (),
    #[webapi(
        method,
        length = 2,
        callback = abort::abort_signal_remove_event_listener_callback
    )]
    remove_event_listener: (),
    #[webapi(
        method,
        length = 1,
        callback = abort::abort_signal_dispatch_event_callback
    )]
    dispatch_event: (),
    #[webapi(
        method,
        length = 0,
        callback = abort::abort_signal_throw_if_aborted_callback
    )]
    throw_if_aborted: (),
    #[webapi(
        accessor_property,
        getter = abort::abort_signal_aborted_getter_callback,
        enumerable
    )]
    aborted: (),
    #[webapi(
        accessor_property,
        getter = abort::abort_signal_reason_getter_callback,
        enumerable
    )]
    reason: (),
    #[webapi(
        accessor_property,
        getter = abort::abort_signal_onabort_getter_callback,
        setter = abort::abort_signal_onabort_setter_callback,
        enumerable
    )]
    onabort: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AbortController, enumerable)]
struct AbortControllerPrototypeDeclaration {
    #[webapi(method, length = 0, callback = abort::abort_controller_abort_callback)]
    abort: (),
    #[webapi(
        accessor_property,
        getter = abort::abort_controller_signal_getter_callback,
        enumerable
    )]
    signal: (),
}

pub(in crate::context_bootstrap) fn install_abort_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "AbortSignal" => {
            AbortSignalConstructorDeclaration::initialize_template(scope, template);
            AbortSignalPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "AbortController" => {
            AbortControllerPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        _ => {}
    }
}
