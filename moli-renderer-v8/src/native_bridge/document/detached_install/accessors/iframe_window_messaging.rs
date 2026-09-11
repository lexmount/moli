use crate::{
    context_bootstrap::{
        dispatch_simple_event_target_event, install_child_window_eval_runtime_state,
        install_child_window_own_methods, install_simple_event_target_methods,
    },
    util::v8str,
};
use moli_webapi_declare::WebApiObject;

use super::iframe_window_message_event::{detached_window_message_event, detached_window_origin};

const DETACHED_IFRAME_WINDOW_EVENT_LISTENERS_SLOT: &str = "__lmDetachedIframeWindowEventListeners";

#[derive(Default, WebApiObject)]
#[webapi(plain)]
struct DetachedIframeWindowMethodsDeclaration {
    #[webapi(method, length = 1, callback = detached_iframe_window_post_message)]
    post_message: (),
}

pub(super) fn install_detached_iframe_window_messaging<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) {
    install_simple_event_target_methods(
        scope,
        window,
        DETACHED_IFRAME_WINDOW_EVENT_LISTENERS_SLOT,
        false,
    );
    let _ = DetachedIframeWindowMethodsDeclaration::default().initialize(scope, window);
    let _ = install_child_window_own_methods(scope, window);
    let _ = install_child_window_eval_runtime_state(scope, window);
}

fn detached_iframe_window_post_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = args.this();
    let target_origin_arg = args.get(1);
    let options = (target_origin_arg.is_object() && !target_origin_arg.is_null_or_undefined())
        .then(|| v8::Local::<v8::Object>::try_from(target_origin_arg).ok())
        .flatten();
    let requested_target_origin = if let Some(options) = options {
        options
            .get(scope, v8str(scope, "targetOrigin").into())
            .and_then(|value| {
                if value.is_undefined() {
                    None
                } else {
                    value
                        .to_string(scope)
                        .map(|value| value.to_rust_string_lossy(scope))
                }
            })
            .unwrap_or_else(|| "/".to_owned())
    } else if target_origin_arg.is_undefined() {
        "/".to_owned()
    } else {
        target_origin_arg
            .to_string(scope)
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_else(|| "*".to_owned())
    };
    let source = target
        .get(scope, v8str(scope, "parent").into())
        .unwrap_or_else(|| scope.get_current_context().global(scope).into());
    let source_origin = v8::Local::<v8::Object>::try_from(source)
        .ok()
        .and_then(|source| detached_window_origin(scope, source))
        .unwrap_or_default();
    let target_origin = detached_window_origin(scope, target).unwrap_or_default();
    let source_security =
        crate::context_bootstrap::RuntimeMessageSourceSecurity::window(source_origin.clone());
    let Some(target_origin_match) =
        crate::window_host::normalized_window_post_message_target_origin(
            scope,
            &requested_target_origin,
            &source_origin,
        )
    else {
        return;
    };
    if !crate::window_host::target_origin_matches(target_origin_match.as_deref(), &target_origin) {
        rv.set_undefined();
        return;
    }
    let data = if options.is_some() {
        crate::context_bootstrap::structured_serialize_value_for_window_post_message_options(
            scope,
            args.get(0),
            args.get(1),
            source_security,
        )
    } else {
        crate::context_bootstrap::structured_serialize_value_for_window_post_message(
            scope,
            args.get(0),
            (args.length() > 2).then(|| args.get(2)),
            source_security,
        )
    };
    let Some(data) = data else {
        return;
    };
    let target_accepts_data =
        crate::context_bootstrap::wasm_module_message_allowed_for_target_origin(
            &data,
            Some(&target_origin),
        );
    let (event_type, data, ports) = if target_accepts_data {
        match crate::context_bootstrap::structured_deserialize_value_for_message_event(scope, &data)
        {
            Some((data, ports)) => ("message", data, ports),
            None => (
                "messageerror",
                v8::null(scope).into(),
                v8::Array::new(scope, 0),
            ),
        }
    } else {
        (
            "messageerror",
            v8::null(scope).into(),
            v8::Array::new(scope, 0),
        )
    };
    let Some(event) =
        detached_window_message_event(scope, event_type, data, source, &source_origin, ports)
    else {
        rv.set_undefined();
        return;
    };
    dispatch_simple_event_target_event(
        scope,
        target,
        DETACHED_IFRAME_WINDOW_EVENT_LISTENERS_SLOT,
        event_type,
        event,
    );
    rv.set_undefined();
}
