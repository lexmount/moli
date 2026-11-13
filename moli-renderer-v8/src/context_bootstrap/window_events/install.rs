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
use crate::util::{get_private_value, set_private_value, v8str};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(fragment, receiver = crate::context_bootstrap::is_window_receiver)]
struct SimpleWindowEventHandlerDeclaration {
    name: &'static str,
    #[webapi(accessor_property = self.name, data = v8str(scope, self.name), enumerable,
        getter = window_event_handler_getter_function,
        setter = window_event_handler_setter_function,
        access_policy = crate::context_bootstrap::require_same_origin_window_receiver)]
    handler: (),
}

#[derive(WebApiObject)]
#[webapi(fragment, receiver = crate::context_bootstrap::is_window_receiver)]
struct LenientWindowEventHandlerDeclaration {
    name: &'static str,
    #[webapi(accessor_property = self.name, data = v8str(scope, self.name), enumerable, legacy_lenient_this,
        getter = window_event_handler_getter_function,
        setter = window_event_handler_setter_function,
        access_policy = crate::context_bootstrap::require_same_origin_window_receiver)]
    handler: (),
}

pub(crate) fn install_simple_window_event_handlers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) {
    let properties: Vec<_> = WINDOW_EVENT_HANDLER_PROPERTIES
        .iter()
        .chain(SECURE_WINDOW_EVENT_HANDLER_PROPERTIES)
        .chain(SECURE_GLOBAL_EVENT_HANDLER_PROPERTIES)
        .copied()
        .filter(|property| event_handler_property_is_exposed(scope, window, property))
        .collect();
    for name in properties {
        install_window_event_handler_accessor(scope, window, name);
    }
}

fn install_window_event_handler_accessor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    name: &'static str,
) {
    if crate::native_bridge::element::legacy_lenient_this_event_handler(name) {
        LenientWindowEventHandlerDeclaration::new(name).initialize(scope, window)
    } else {
        SimpleWindowEventHandlerDeclaration::new(name).initialize(scope, window)
    }
    .expect("Window handler accessor declaration should initialize");
}

// Reset handler values alongside listener registrations, without invoking
// author-defined property setters.
pub(crate) fn clear_simple_window_event_handler_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) {
    for property in WINDOW_EVENT_HANDLER_PROPERTIES
        .iter()
        .chain(SECURE_WINDOW_EVENT_HANDLER_PROPERTIES)
        .chain(SECURE_GLOBAL_EVENT_HANDLER_PROPERTIES)
    {
        set_private_value(scope, window, property, v8::null(scope).into());
    }
}

#[derive(Default, WebApiObject)]
#[webapi(fragment, prototype = "Window", receiver = crate::context_bootstrap::is_window_receiver)]
struct WindowGlobalEventHandlerAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = window_onmessageerror_getter_function,
        setter = window_onmessageerror_setter_function,
        access_policy = crate::context_bootstrap::require_same_origin_window_receiver,
        enumerable
    )]
    onmessageerror: (),

    #[webapi(
        accessor_property,
        getter = window_onerror_getter_function,
        setter = window_onerror_setter_function,
        access_policy = crate::context_bootstrap::require_same_origin_window_receiver,
        enumerable
    )]
    onerror: (),

    #[webapi(
        accessor_property,
        getter = window_onunhandledrejection_getter_function,
        setter = window_onunhandledrejection_setter_function,
        access_policy = crate::context_bootstrap::require_same_origin_window_receiver,
        enumerable
    )]
    onunhandledrejection: (),

    #[webapi(
        accessor_property,
        getter = window_onrejectionhandled_getter_function,
        setter = window_onrejectionhandled_setter_function,
        access_policy = crate::context_bootstrap::require_same_origin_window_receiver,
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
            install_window_event_handler_accessor(scope, window, name);
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
