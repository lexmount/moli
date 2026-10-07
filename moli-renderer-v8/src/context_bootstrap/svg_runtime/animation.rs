use moli_webapi_declare::WebApiFunctionTemplate;

use super::callbacks::svg_test_string_list_getter;
use crate::{
    document_runtime::DomHandle,
    native_bridge::{
        JsContextHost,
        document::detached_native_object_for_handle,
        element::{node_event_handler_getter_function, node_event_handler_setter_function},
        node_runtime_and_handle_from_object_or_detached, throw_dom_exception,
    },
    util::{callback_data_index_value, v8str},
    web_api_interfaces, webidl,
};

fn svg_receiver_runtime_and_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> anyhow::Result<(*mut JsContextHost, DomHandle)> {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("generated receiver check validates native SVG identity");
    node_runtime_and_handle_from_object_or_detached(scope, receiver)
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGAnimationElement, enumerable, receiver)]
struct SvgAnimationDeclaration {
    #[webapi(accessor_property = "targetElement", getter = target_element_getter)]
    target_element: (),
    #[webapi(accessor_property = "onbegin", getter = node_event_handler_getter_function,
        setter = node_event_handler_setter_function, data = v8str(scope, "onbegin"))]
    onbegin: (),
    #[webapi(accessor_property = "onend", getter = node_event_handler_getter_function,
        setter = node_event_handler_setter_function, data = v8str(scope, "onend"))]
    onend: (),
    #[webapi(accessor_property = "onrepeat", getter = node_event_handler_getter_function,
        setter = node_event_handler_setter_function, data = v8str(scope, "onrepeat"))]
    onrepeat: (),
    #[webapi(accessor_property = "requiredExtensions", getter = svg_test_string_list_getter,
        data = callback_data_index_value(scope, 0))]
    required_extensions: (),
    #[webapi(accessor_property = "systemLanguage", getter = svg_test_string_list_getter,
        data = callback_data_index_value(scope, 1))]
    system_language: (),
    #[webapi(method = "getStartTime", length = 0, callback = get_start_time)]
    get_start_time: (),
    #[webapi(method = "getCurrentTime", length = 0, callback = get_current_time)]
    get_current_time: (),
    #[webapi(method = "getSimpleDuration", length = 0, callback = get_simple_duration)]
    get_simple_duration: (),
    #[webapi(method = "beginElement", length = 0, callback = begin_element)]
    begin_element: (),
    #[webapi(method = "beginElementAt", length = 1, callback = begin_element_at)]
    begin_element_at: (),
    #[webapi(method = "endElement", length = 0, callback = end_element)]
    end_element: (),
    #[webapi(method = "endElementAt", length = 1, callback = end_element_at)]
    end_element_at: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SVGSVGElement, enumerable, receiver)]
struct SvgTimeContainerDeclaration {
    #[webapi(method = "pauseAnimations", length = 0, callback = pause_animations)]
    pause_animations: (),
    #[webapi(method = "unpauseAnimations", length = 0, callback = unpause_animations)]
    unpause_animations: (),
    #[webapi(method = "animationsPaused", length = 0, callback = animations_paused)]
    animations_paused: (),
    #[webapi(method = "getCurrentTime", length = 0, callback = get_current_time)]
    get_current_time: (),
    #[webapi(method = "setCurrentTime", length = 1, callback = set_current_time)]
    set_current_time: (),
}

pub(super) fn install_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    if name == "SVGAnimationElement" {
        let prototype = template.prototype_template(scope);
        SvgAnimationDeclaration::initialize_prototype_template(scope, prototype);
    } else if name == "SVGSVGElement" {
        let prototype = template.prototype_template(scope);
        SvgTimeContainerDeclaration::initialize_prototype_template(scope, prototype);
    }
}

fn target_element_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = svg_receiver_runtime_and_handle(scope, &args) else {
        return;
    };
    let target = unsafe { &*runtime_ptr }.svg_animation_target(handle);
    let Some(target) =
        target.and_then(|handle| detached_native_object_for_handle(scope, runtime_ptr, handle))
    else {
        rv.set_null();
        return;
    };
    rv.set(target.into());
}

fn get_start_time<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Ok((runtime, handle)) = svg_receiver_runtime_and_handle(scope, &args)
        && let Some(start) = unsafe { &*runtime }.svg_animation_start_time(handle)
    {
        rv.set_double(f64::from(start as f32));
        return;
    }
    throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "The animation has no current interval.",
    );
}

fn get_current_time<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Ok((runtime, handle)) = svg_receiver_runtime_and_handle(scope, &args) {
        rv.set_double(f64::from(
            unsafe { &*runtime }.svg_presentation_time(handle) as f32,
        ));
    }
}

fn get_simple_duration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Ok((runtime, handle)) = svg_receiver_runtime_and_handle(scope, &args)
        && let Some(duration) = unsafe { &*runtime }.svg_animation_simple_duration(handle)
    {
        rv.set_double(f64::from(duration as f32));
        return;
    }
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "The animation has no finite simple duration.",
    );
}

fn add_instance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    begin: bool,
    offset: f32,
) {
    if let Ok((runtime, handle)) = svg_receiver_runtime_and_handle(scope, args) {
        unsafe { &mut *runtime }.add_svg_animation_instance(handle, begin, offset);
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVGAnimationElement")]
struct TimingOffsetArgs {
    #[webidl(required, converter = "float")]
    offset: f32,
}

fn begin_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    add_instance(scope, &args, true, 0.0);
}

fn end_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    add_instance(scope, &args, false, 0.0);
}

fn begin_element_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<TimingOffsetArgs>(scope, &args) else {
        return;
    };
    add_instance(scope, &args, true, parsed.offset);
}

fn end_element_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(parsed) = webidl::parse_args::<TimingOffsetArgs>(scope, &args) {
        add_instance(scope, &args, false, parsed.offset);
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVGSVGElement.setCurrentTime")]
struct SeekArgs {
    #[webidl(required, converter = "float")]
    seconds: f32,
}

fn set_current_time<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<SeekArgs>(scope, &args) else {
        return;
    };
    if let Ok((runtime, handle)) = svg_receiver_runtime_and_handle(scope, &args) {
        unsafe { &mut *runtime }.seek_svg_animations(handle, parsed.seconds);
    }
}

fn set_paused<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    pause: bool,
) {
    if let Ok((runtime, handle)) = svg_receiver_runtime_and_handle(scope, args) {
        unsafe { &mut *runtime }.pause_svg_animations(handle, pause);
    }
}

fn pause_animations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_paused(scope, &args, true);
}

fn unpause_animations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_paused(scope, &args, false);
}

fn animations_paused<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Ok((runtime, handle)) = svg_receiver_runtime_and_handle(scope, &args) {
        rv.set_bool(unsafe { &*runtime }.svg_animations_paused(handle));
    }
}
