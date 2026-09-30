use super::accessors::{
    window_event_handler_getter_function, window_event_handler_setter_function,
    window_onerror_getter_function, window_onerror_setter_function,
    window_onmessageerror_getter_function, window_onmessageerror_setter_function,
    window_onrejectionhandled_getter_function, window_onrejectionhandled_setter_function,
    window_onunhandledrejection_getter_function, window_onunhandledrejection_setter_function,
};
use super::{
    SECURE_GLOBAL_EVENT_HANDLER_PROPERTIES, SECURE_WINDOW_EVENT_HANDLER_PROPERTIES,
    WINDOW_EVENT_HANDLER_PROPERTIES,
};
use crate::definitions::define_function_accessor_property;
use crate::util::{get_private_value, v8str};
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
            "onerror" | "onmessageerror" | "onunhandledrejection" | "onrejectionhandled"
        ) {
            continue;
        }
        install_window_event_handler_accessor(scope, global, name);
    }
}

pub(crate) fn install_secure_window_event_handler_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    secure_context_available: bool,
) {
    for name in SECURE_WINDOW_EVENT_HANDLER_PROPERTIES
        .iter()
        .chain(SECURE_GLOBAL_EVENT_HANDLER_PROPERTIES)
    {
        if secure_context_available {
            let data = v8str(scope, name).into();
            define_function_accessor_property(
                scope,
                window,
                name,
                window_event_handler_getter_function,
                Some(data),
                window_event_handler_setter_function,
                Some(data),
                v8::PropertyAttribute::NONE,
            )
            .expect("secure Window event handler accessor should initialize");
        } else {
            let _ = window.delete(scope, v8str(scope, name).into());
        }
    }
}

pub(in crate::context_bootstrap) fn finalize_secure_global_event_handler_realm_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> anyhow::Result<()> {
    let global = scope.get_current_context().global(scope);
    if window_realm_has_secure_context(scope, global) {
        return Ok(());
    }

    // The isolate template is shared by secure and insecure realms. Filter
    // the realm-local prototype before exposing it to author script.
    for name in SECURE_GLOBAL_EVENT_HANDLER_PROPERTIES {
        if !prototype
            .delete(scope, v8str(scope, name).into())
            .unwrap_or(false)
        {
            return Err(anyhow::anyhow!(
                "failed to remove secure GlobalEventHandlers member `{name}`"
            ));
        }
    }
    Ok(())
}

pub(crate) fn event_handler_property_is_exposed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    property_name: &str,
) -> bool {
    if !SECURE_GLOBAL_EVENT_HANDLER_PROPERTIES.contains(&property_name)
        && !SECURE_WINDOW_EVENT_HANDLER_PROPERTIES.contains(&property_name)
    {
        return true;
    }
    let Some(context) = target.get_creation_context(scope) else {
        return false;
    };
    let global = context.global(scope);
    window_realm_has_secure_context(scope, global)
}

fn window_realm_has_secure_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(
        scope,
        global,
        super::super::runtime_state::WINDOW_SECURE_CONTEXT_AVAILABLE_SLOT,
    )
    .is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn install_window_event_handler_accessor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    name: &'static str,
) {
    let data = v8str(scope, name).into();
    define_function_accessor_property(
        scope,
        window,
        name,
        window_event_handler_getter_function,
        Some(data),
        window_event_handler_setter_function,
        Some(data),
        v8::PropertyAttribute::NONE,
    )
    .expect("Window event handler accessor should initialize");
}
