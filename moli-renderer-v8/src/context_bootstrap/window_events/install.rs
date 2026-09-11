use super::WINDOW_EVENT_HANDLER_PROPERTIES;
use super::accessors::{
    window_event_handler_getter_function, window_event_handler_setter_function,
    window_onerror_getter_function, window_onerror_setter_function,
    window_onmessageerror_getter_function, window_onmessageerror_setter_function,
    window_onrejectionhandled_getter_function, window_onrejectionhandled_setter_function,
    window_onunhandledrejection_getter_function, window_onunhandledrejection_setter_function,
};
use crate::definitions::define_function_accessor_property;
use crate::util::v8str;
use moli_webapi_declare::WebApiObject;

#[derive(Default, WebApiObject)]
#[webapi(fragment, prototype = "Window")]
struct WindowGlobalEventHandlerAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = window_onmessageerror_getter_function,
        setter = window_onmessageerror_setter_function,
        enumerable
    )]
    onmessageerror: (),

    #[webapi(
        accessor_property,
        getter = window_onerror_getter_function,
        setter = window_onerror_setter_function,
        enumerable
    )]
    onerror: (),

    #[webapi(
        accessor_property,
        getter = window_onunhandledrejection_getter_function,
        setter = window_onunhandledrejection_setter_function,
        enumerable
    )]
    onunhandledrejection: (),

    #[webapi(
        accessor_property,
        getter = window_onrejectionhandled_getter_function,
        setter = window_onrejectionhandled_setter_function,
        enumerable
    )]
    onrejectionhandled: (),
}

pub(in crate::context_bootstrap) fn install_window_global_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) {
    WindowGlobalEventHandlerAccessorsDeclaration::default()
        .initialize(scope, global)
        .expect("Window global event handler accessors declaration should initialize");
    for name in WINDOW_EVENT_HANDLER_PROPERTIES {
        if matches!(
            *name,
            "onerror" | "onunhandledrejection" | "onrejectionhandled"
        ) {
            continue;
        }
        let data = v8str(scope, name).into();
        define_function_accessor_property(
            scope,
            global,
            name,
            window_event_handler_getter_function,
            Some(data),
            window_event_handler_setter_function,
            Some(data),
            v8::PropertyAttribute::NONE,
        )
        .expect("Window event handler accessor should initialize");
    }
}
