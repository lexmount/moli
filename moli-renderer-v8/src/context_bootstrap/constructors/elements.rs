use super::*;
use crate::webidl;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct HtmlElementConstructorTrapHandlerDeclaration<'scope> {
    #[webapi(data_property)]
    construct: v8::Local<'scope, v8::Function>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Image")]
struct ImageConstructorArgs {
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Audio")]
struct AudioConstructorArgs {
    src: Option<String>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Option")]
struct OptionConstructorArgs {
    #[webidl(default = "")]
    text: String,
    value: Option<String>,
    #[webidl(default = false)]
    default_selected: bool,
    #[webidl(default = false)]
    selected: bool,
}

pub(in crate::context_bootstrap) fn image_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "Image constructor must be called with new");
        return;
    }
    let Some(parsed) = webidl::parse_args::<ImageConstructorArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let global = scope.get_current_context().global(scope);
    let Some(document) = global.get(scope, v8str(scope, "document").into()) else {
        rv.set_undefined();
        return;
    };
    let Ok(document) = v8::Local::<v8::Object>::try_from(document) else {
        rv.set_undefined();
        return;
    };
    let Some(create_element) = document.get(scope, v8str(scope, "createElement").into()) else {
        rv.set_undefined();
        return;
    };
    let Ok(create_element) = v8::Local::<v8::Function>::try_from(create_element) else {
        rv.set_undefined();
        return;
    };
    let Some(tag) = v8_string(scope, "img") else {
        rv.set_undefined();
        return;
    };
    let Some(image) = create_element.call(scope, document.into(), &[tag.into()]) else {
        rv.set_undefined();
        return;
    };
    let Ok(image) = v8::Local::<v8::Object>::try_from(image) else {
        rv.set_undefined();
        return;
    };
    if let Some(width) = parsed.width {
        let value = v8::Integer::new_from_unsigned(scope, width);
        let _ = image.set(scope, v8str(scope, "width").into(), value.into());
    }
    if let Some(height) = parsed.height {
        let value = v8::Integer::new_from_unsigned(scope, height);
        let _ = image.set(scope, v8str(scope, "height").into(), value.into());
    }
    apply_legacy_factory_new_target_prototype(scope, &args, image, "HTMLImageElement");
    rv.set(image.into());
}

pub(in crate::context_bootstrap) fn audio_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "Audio constructor must be called with new");
        return;
    }
    let Some(parsed) = webidl::parse_args::<AudioConstructorArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let global = scope.get_current_context().global(scope);
    let Some(document) = global.get(scope, v8str(scope, "document").into()) else {
        rv.set_undefined();
        return;
    };
    let Ok(document) = v8::Local::<v8::Object>::try_from(document) else {
        rv.set_undefined();
        return;
    };
    let Some(create_element) = document.get(scope, v8str(scope, "createElement").into()) else {
        rv.set_undefined();
        return;
    };
    let Ok(create_element) = v8::Local::<v8::Function>::try_from(create_element) else {
        rv.set_undefined();
        return;
    };
    let Some(tag) = v8_string(scope, "audio") else {
        rv.set_undefined();
        return;
    };
    let Some(audio) = create_element.call(scope, document.into(), &[tag.into()]) else {
        rv.set_undefined();
        return;
    };
    let Ok(audio) = v8::Local::<v8::Object>::try_from(audio) else {
        rv.set_undefined();
        return;
    };
    if let Some(set_attribute) = audio.get(scope, v8str(scope, "setAttribute").into())
        && let Ok(set_attribute) = v8::Local::<v8::Function>::try_from(set_attribute)
    {
        let _ = set_attribute.call(
            scope,
            audio.into(),
            &[v8str(scope, "preload").into(), v8str(scope, "auto").into()],
        );
    }
    if let Some(src) = parsed.src {
        let Some(src) = v8_string(scope, &src) else {
            rv.set_undefined();
            return;
        };
        if let Some(set_attribute) = audio.get(scope, v8str(scope, "setAttribute").into())
            && let Ok(set_attribute) = v8::Local::<v8::Function>::try_from(set_attribute)
        {
            let _ = set_attribute.call(
                scope,
                audio.into(),
                &[v8str(scope, "src").into(), src.into()],
            );
        }
    }
    apply_legacy_factory_new_target_prototype(scope, &args, audio, "HTMLAudioElement");
    rv.set(audio.into());
}

pub(in crate::context_bootstrap) fn option_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "Option constructor must be called with new");
        return;
    }
    let Some(parsed) = webidl::parse_args::<OptionConstructorArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let global = scope.get_current_context().global(scope);
    let Some(document) = global.get(scope, v8str(scope, "document").into()) else {
        rv.set_undefined();
        return;
    };
    let Ok(document) = v8::Local::<v8::Object>::try_from(document) else {
        rv.set_undefined();
        return;
    };
    let Some(create_element) = document.get(scope, v8str(scope, "createElement").into()) else {
        rv.set_undefined();
        return;
    };
    let Ok(create_element) = v8::Local::<v8::Function>::try_from(create_element) else {
        rv.set_undefined();
        return;
    };
    let Some(tag) = v8_string(scope, "option") else {
        rv.set_undefined();
        return;
    };
    let Some(option) = create_element.call(scope, document.into(), &[tag.into()]) else {
        rv.set_undefined();
        return;
    };
    let Ok(option) = v8::Local::<v8::Object>::try_from(option) else {
        rv.set_undefined();
        return;
    };

    let Some(text) = v8_string(scope, &parsed.text) else {
        rv.set_undefined();
        return;
    };
    let _ = option.set(scope, v8str(scope, "text").into(), text.into());

    if let Some(value) = parsed.value {
        let Some(value) = v8_string(scope, &value) else {
            rv.set_undefined();
            return;
        };
        let _ = option.set(scope, v8str(scope, "value").into(), value.into());
    }

    if parsed.default_selected {
        let _ = option.set(
            scope,
            v8str(scope, "defaultSelected").into(),
            v8::Boolean::new(scope, true).into(),
        );
        if args.length() < 4
            && let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, option)
        {
            let _ = unsafe { &mut *runtime_ptr }.set_selected_state_with_dirty(
                scope,
                runtime_ptr,
                handle,
                false,
                false,
            );
        }
    }
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, option) {
        // WHATWG HTML's legacy Option() factory overrides selectedness to
        // false when the fourth argument is omitted, even if defaultSelected
        // set the selected attribute.
        let _ = unsafe { &mut *runtime_ptr }.set_selected_state_with_dirty(
            scope,
            runtime_ptr,
            handle,
            parsed.selected,
            false,
        );
    }
    apply_legacy_factory_new_target_prototype(scope, &args, option, "HTMLOptionElement");
    rv.set(option.into());
}

fn apply_legacy_factory_new_target_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    result: v8::Local<'s, v8::Object>,
    default_constructor_name: &str,
) {
    let receiver = args.this();
    crate::util::apply_webidl_constructor_prototype_fallback(
        scope,
        receiver,
        args.new_target(),
        default_constructor_name,
    );
    if let Some(prototype) = receiver.get_prototype(scope) {
        let _ = result.set_prototype(scope, prototype);
    }
}

pub(crate) fn html_element_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "HTML element constructor must be called with new");
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        throw_type_error(
            scope,
            "Missing native bridge host for HTML element construction",
        );
        return;
    };
    let Ok(new_target) = v8::Local::<v8::Function>::try_from(args.new_target()) else {
        throw_type_error(scope, "Invalid custom element constructor target");
        return;
    };
    let active_constructor_name = args
        .data()
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "HTMLElement".to_owned());
    match unsafe { &mut *host_ptr }.take_pending_custom_element_wrapper_for(scope, new_target) {
        Some(custom_elements::PendingCustomElementConstruction::Wrapper(wrapper, _handle)) => {
            custom_elements::set_wrapper_custom_element_constructor_prototype(
                scope, wrapper, new_target,
            );
            rv.set(wrapper.into());
        }
        Some(custom_elements::PendingCustomElementConstruction::AlreadyConstructed(handle)) => {
            custom_elements::throw_already_constructed_custom_element_error(scope, handle);
        }
        None => {
            match custom_elements::create_element_from_registered_constructor(
                scope,
                host_ptr,
                new_target,
                &active_constructor_name,
                args.this(),
            ) {
                Some(wrapper) => {
                    rv.set(wrapper.into());
                }
                None => {
                    throw_type_error(
                        scope,
                        "Invalid custom element construction outside document.createElement()",
                    );
                }
            }
        }
    }
}

pub(crate) fn html_element_constructor_with_early_sanity_trap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Function>,
    active_constructor_name: &str,
) -> Option<v8::Local<'s, v8::Proxy>> {
    let constructor_name = v8_string(scope, active_constructor_name)?;
    let construct = v8::Function::builder(html_element_constructor_construct_trap_callback)
        .data(constructor_name.into())
        .length(3)
        .build(scope)?;
    let handler = HtmlElementConstructorTrapHandlerDeclaration { construct }
        .bind(scope)
        .ok()?;
    v8::Proxy::new(scope, constructor.into(), handler)
}

fn html_element_constructor_construct_trap_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        throw_type_error(
            scope,
            "Missing native bridge host for HTML element construction",
        );
        return;
    };
    let active_constructor_name = args
        .data()
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "HTMLElement".to_owned());
    let Ok(new_target) = v8::Local::<v8::Function>::try_from(args.get(2)) else {
        throw_type_error(scope, "Invalid custom element constructor target");
        return;
    };
    if !custom_elements::html_constructor_new_target_passes_early_sanity(
        scope,
        host_ptr,
        new_target,
        &active_constructor_name,
    ) {
        throw_type_error(
            scope,
            "Invalid custom element construction outside document.createElement()",
        );
        return;
    }
    let global = scope.get_current_context().global(scope);
    let Some(reflect) = global
        .get(scope, v8str(scope, "Reflect").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        throw_type_error(
            scope,
            "Missing Reflect.construct for HTML element construction",
        );
        return;
    };
    let Some(reflect_construct) = reflect
        .get(scope, v8str(scope, "construct").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        throw_type_error(
            scope,
            "Missing Reflect.construct for HTML element construction",
        );
        return;
    };
    let forwarded = [args.get(0), args.get(1), new_target.into()];
    let Some(value) = reflect_construct.call(scope, reflect.into(), &forwarded) else {
        return;
    };
    rv.set(value);
}
