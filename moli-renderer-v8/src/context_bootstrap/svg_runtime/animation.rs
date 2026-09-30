use moli_webapi_declare::WebApiFunctionTemplate;

use super::callbacks::svg_test_string_list_getter;
use crate::{
    document_runtime::DomHandle,
    native_bridge::{
        JsContextHost,
        document::{SVG_NS, detached_native_object_for_handle},
        element::{node_event_handler_getter_function, node_event_handler_setter_function},
        node_runtime_and_handle_from_object_or_detached, throw_dom_exception,
    },
    util::{callback_data_index_value, v8str},
    web_api_interfaces, webidl,
};

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
    #[webapi(method = "beginElement", length = 0, callback = unsupported_timing)]
    begin_element: (),
    #[webapi(method = "beginElementAt", length = 1, callback = unsupported_timing_at)]
    begin_element_at: (),
    #[webapi(method = "endElement", length = 0, callback = unsupported_timing)]
    end_element: (),
    #[webapi(method = "endElementAt", length = 1, callback = unsupported_timing_at)]
    end_element_at: (),
}

pub(super) fn install_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    if name == "SVGAnimationElement" {
        let prototype = template.prototype_template(scope);
        SvgAnimationDeclaration::initialize_prototype_template(scope, prototype);
    }
}

fn target_element(runtime: &JsContextHost, animation: DomHandle) -> Option<DomHandle> {
    let dom = runtime.dom_host();
    if !dom.is_connected_to_document(animation) {
        return None;
    }
    let href = dom
        .get_attribute_ns(animation, None, "href")
        .or_else(|| dom.get_attribute_ns(animation, Some("http://www.w3.org/1999/xlink"), "href"))
        .unwrap_or_default();
    let target = if href.is_empty() {
        dom.node(animation)?.parent_node()?
    } else {
        let fragment = if let Some(fragment) = href.strip_prefix('#') {
            fragment.to_owned()
        } else {
            let document = dom.owner_document_handle(animation)?;
            let mut url = runtime
                .document_base_url_for_handle(document)
                .join(&href)
                .ok()?;
            let fragment = url.fragment()?.to_owned();
            url.set_fragment(None);
            let mut document_url = runtime.document_url_for_handle(document);
            document_url.set_fragment(None);
            if url != document_url {
                return None;
            }
            fragment
        };
        if fragment.is_empty() {
            return None;
        }
        let id = percent_encoding::percent_decode_str(&fragment).decode_utf8_lossy();
        let root = dom.root_node_handle(animation)?;
        dom.element_handle_by_id_in_subtree(root, &id)?
    };
    dom.node(target)
        .is_some_and(|node| node.namespace() == Some(SVG_NS))
        .then_some(target)
}

fn target_element_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let target = target_element(unsafe { &*runtime_ptr }, handle);
    let Some(target) =
        target.and_then(|handle| detached_native_object_for_handle(scope, runtime_ptr, handle))
    else {
        rv.set_null();
        return;
    };
    rv.set(target.into());
}

// Native DOM, interface identity and handler dispatch are supported. A SMIL
// time container is not yet scheduled, so no current interval exists. Timing
// controls fail explicitly instead of pretending to start an animation.
fn get_start_time<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "The animation has no current interval.",
    );
}

fn get_current_time<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set_double(0.0);
}

fn get_simple_duration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "SVG animation durations are not implemented.",
    );
}

fn unsupported_timing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "SVG animation timing is not implemented.",
    );
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SVGAnimationElement")]
struct TimingOffsetArgs {
    #[webidl(required, converter = "double")]
    offset: f64,
}

fn unsupported_timing_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<TimingOffsetArgs>(scope, &args) else {
        return;
    };
    if !(parsed.offset as f32).is_finite() {
        webidl::throw_type_error(
            scope,
            "SVG animation offset is outside the finite float range.",
        );
        return;
    }
    unsupported_timing(scope, args, rv);
}
