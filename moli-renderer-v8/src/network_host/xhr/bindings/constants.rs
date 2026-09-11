use super::*;
use crate::context_bootstrap::simple_object_event_set_ordered_handler;
use crate::util::{
    callback_data_index_value, callback_data_item, get_private_value, set_private_value,
};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::XMLHttpRequest, enumerable)]
struct XmlHttpRequestConstantsDeclaration {
    #[webapi(constant = "UNSENT", value = 0u32)]
    _unsent: (),
    #[webapi(constant = "OPENED", value = 1u32)]
    _opened: (),
    #[webapi(constant = "HEADERS_RECEIVED", value = 2u32)]
    _headers_received: (),
    #[webapi(constant = "LOADING", value = 3u32)]
    _loading: (),
    #[webapi(constant = "DONE", value = 4u32)]
    _done: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::XMLHttpRequest, enumerable)]
struct XmlHttpRequestPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = xhr_number_getter, data = callback_data_index_value(scope, 0))]
    ready_state: (),
    #[webapi(accessor_property, getter = xhr_number_getter, data = callback_data_index_value(scope, 1))]
    status: (),
    #[webapi(accessor_property, getter = xhr_string_getter, data = callback_data_index_value(scope, 2))]
    status_text: (),
    #[webapi(accessor_property, getter = xhr_response_text_getter, data = callback_data_index_value(scope, 3))]
    response_text: (),
    #[webapi(
        accessor_property,
        getter = xhr_string_getter,
        setter = xhr_response_type_setter,
        data = callback_data_index_value(scope, 4)
    )]
    response_type: (),
    #[webapi(accessor_property = "responseURL", getter = xhr_string_getter, data = callback_data_index_value(scope, 5))]
    response_url: (),
    #[webapi(accessor_property, getter = xhr_value_getter, data = callback_data_index_value(scope, 6))]
    response: (),
    #[webapi(
        accessor_property,
        getter = xhr_number_getter,
        setter = xhr_timeout_setter,
        data = callback_data_index_value(scope, 8)
    )]
    timeout: (),
    #[webapi(
        accessor_property,
        getter = xhr_bool_getter,
        setter = xhr_with_credentials_setter,
        data = callback_data_index_value(scope, 9)
    )]
    with_credentials: (),
    #[webapi(
        accessor_property,
        getter = xhr_value_getter,
        setter = xhr_value_setter,
        data = callback_data_index_value(scope, 10)
    )]
    onreadystatechange: (),
    #[webapi(accessor_property, getter = xhr_upload_getter, data = callback_data_index_value(scope, 11))]
    upload: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::XMLHttpRequest, enumerable)]
struct XmlHttpRequestResponseXmlAccessorDeclaration {
    #[webapi(accessor_property = "responseXML", getter = xhr_response_xml_getter, data = callback_data_index_value(scope, 7))]
    response_xml: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::XMLHttpRequestEventTarget, enumerable)]
struct XmlHttpRequestEventTargetPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = xhr_value_getter, setter = xhr_value_setter, data = callback_data_index_value(scope, 12))]
    onload: (),
    #[webapi(accessor_property, getter = xhr_value_getter, setter = xhr_value_setter, data = callback_data_index_value(scope, 13))]
    onerror: (),
    #[webapi(accessor_property, getter = xhr_value_getter, setter = xhr_value_setter, data = callback_data_index_value(scope, 14))]
    onprogress: (),
    #[webapi(accessor_property, getter = xhr_value_getter, setter = xhr_value_setter, data = callback_data_index_value(scope, 15))]
    onabort: (),
    #[webapi(accessor_property, getter = xhr_value_getter, setter = xhr_value_setter, data = callback_data_index_value(scope, 16))]
    ontimeout: (),
    #[webapi(accessor_property, getter = xhr_value_getter, setter = xhr_value_setter, data = callback_data_index_value(scope, 17))]
    onloadstart: (),
    #[webapi(accessor_property, getter = xhr_value_getter, setter = xhr_value_setter, data = callback_data_index_value(scope, 18))]
    onloadend: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XMLHttpRequest.timeout")]
struct XhrTimeoutArgs {
    #[webidl(required)]
    value: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XMLHttpRequest.responseType")]
struct XhrResponseTypeArgs {
    #[webidl(required, converter = "enum")]
    value: XmlHttpRequestResponseType,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "XMLHttpRequest.withCredentials")]
struct XhrWithCredentialsArgs {
    #[webidl(required)]
    value: bool,
}

pub(crate) fn install_xml_http_request_template_surface<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "XMLHttpRequest" => {
            XmlHttpRequestConstantsDeclaration::initialize_template(scope, template);
            XmlHttpRequestConstantsDeclaration::initialize_prototype_template(scope, prototype);
            XmlHttpRequestPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "XMLHttpRequestEventTarget" => {
            XmlHttpRequestEventTargetPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

pub(crate) fn install_window_xml_http_request_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    XmlHttpRequestResponseXmlAccessorDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

fn xhr_number_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(xhr) = args.this().to_object(scope) else {
        rv.set(v8::Number::new(scope, 0.0).into());
        return;
    };
    let Some(key) = callback_data_item(
        scope,
        &args,
        XHR_CALLBACK_DATA_SLOTS,
        "XMLHttpRequest callback data slots",
    ) else {
        rv.set(v8::Number::new(scope, 0.0).into());
        return;
    };
    let value = xhr_state_number_property(scope, xhr, key).unwrap_or(0.0);
    rv.set(v8::Number::new(scope, value).into());
}

fn xhr_string_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(xhr) = args.this().to_object(scope) else {
        rv.set(
            v8_string(scope, "")
                .map(|value| value.into())
                .unwrap_or_else(|| v8::undefined(scope).into()),
        );
        return;
    };
    let Some(key) = callback_data_item(
        scope,
        &args,
        XHR_CALLBACK_DATA_SLOTS,
        "XMLHttpRequest callback data slots",
    ) else {
        rv.set(
            v8_string(scope, "")
                .map(|value| value.into())
                .unwrap_or_else(|| v8::undefined(scope).into()),
        );
        return;
    };
    let value = xhr_state_string_property(scope, xhr, key).unwrap_or_default();
    rv.set(
        v8_string(scope, &value)
            .map(|string| string.into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

fn xhr_response_text_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(xhr) = args.this().to_object(scope) else {
        return;
    };
    let response_type = xhr_state_string_property(scope, xhr, XHR_RESPONSE_TYPE_SLOT)
        .as_deref()
        .and_then(XmlHttpRequestResponseType::parse)
        .unwrap_or(XmlHttpRequestResponseType::Default);
    if !matches!(
        response_type,
        XmlHttpRequestResponseType::Default | XmlHttpRequestResponseType::Text
    ) {
        xhr_throw_invalid_state(
            scope,
            "XMLHttpRequest.responseText is only available when responseType is '' or 'text'.",
        );
        return;
    }
    xhr_string_getter(scope, args, rv);
}

fn xhr_response_xml_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(xhr) = args.this().to_object(scope) else {
        return;
    };
    let response_type = xhr_state_string_property(scope, xhr, XHR_RESPONSE_TYPE_SLOT)
        .as_deref()
        .and_then(XmlHttpRequestResponseType::parse)
        .unwrap_or(XmlHttpRequestResponseType::Default);
    if !matches!(
        response_type,
        XmlHttpRequestResponseType::Default | XmlHttpRequestResponseType::Document
    ) {
        xhr_throw_invalid_state(
            scope,
            "XMLHttpRequest.responseXML is only available when responseType is '' or 'document'.",
        );
        return;
    }
    xhr_value_getter(scope, args, rv);
}

fn xhr_bool_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(xhr) = args.this().to_object(scope) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let Some(key) = callback_data_item(
        scope,
        &args,
        XHR_CALLBACK_DATA_SLOTS,
        "XMLHttpRequest callback data slots",
    ) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    let value = xhr_state_bool_property(scope, xhr, key).unwrap_or(false);
    rv.set(v8::Boolean::new(scope, value).into());
}

fn xhr_value_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(xhr) = args.this().to_object(scope) else {
        rv.set_null();
        return;
    };
    let Some(key) = callback_data_item(
        scope,
        &args,
        XHR_CALLBACK_DATA_SLOTS,
        "XMLHttpRequest callback data slots",
    ) else {
        rv.set_null();
        return;
    };
    rv.set(
        xhr_state_value(scope, xhr, key)
            .or_else(|| get_private_value(scope, xhr, key))
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn xhr_upload_getter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(xhr) = args.this().to_object(scope) else {
        rv.set_undefined();
        return;
    };
    rv.set(
        xhr_upload_object(scope, xhr)
            .map(|object| object.into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

fn xhr_timeout_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(xhr) = args.this().to_object(scope) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<XhrTimeoutArgs>(scope, &args) else {
        return;
    };
    let async_request = xhr_state_bool_property(scope, xhr, XHR_ASYNC_SLOT).unwrap_or(true);
    if xhr_is_synchronous_document_request(scope, async_request) {
        xhr_throw_invalid_access(
            scope,
            "Failed to set the 'timeout' property on 'XMLHttpRequest': Timeouts cannot be set for synchronous requests made from a document.",
        );
        return;
    }
    set_xhr_state_number(scope, xhr, XHR_TIMEOUT_SLOT, f64::from(parsed.value));
    if !crate::worker::try_worker_xhr_reschedule_timeout_after_timeout_change(scope, xhr) {
        super::super::delivery::reschedule_xhr_timeout_after_timeout_change(scope, xhr);
    }
}

fn xhr_response_type_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(xhr) = args.this().to_object(scope) else {
        return;
    };
    let ready_state = xhr_state_number_property(scope, xhr, XHR_READY_STATE_SLOT).unwrap_or(0.0);
    if matches!(ready_state as u32, 3 | 4) {
        xhr_throw_invalid_state(
            scope,
            "XMLHttpRequest.responseType cannot be changed once loading has started.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<XhrResponseTypeArgs>(scope, &args) else {
        return;
    };
    let response_type = parsed.value;
    let async_request = xhr_state_bool_property(scope, xhr, XHR_ASYNC_SLOT).unwrap_or(true);
    if xhr_is_synchronous_document_request(scope, async_request) {
        xhr_throw_invalid_access(
            scope,
            "Failed to set the 'responseType' property on 'XMLHttpRequest': The response type cannot be changed for synchronous requests made from a document.",
        );
        return;
    }
    if response_type == XmlHttpRequestResponseType::Document
        && xhr_current_context_is_worker_global(scope)
    {
        return;
    }
    set_xhr_state_string(scope, xhr, XHR_RESPONSE_TYPE_SLOT, response_type.label());
}

fn xhr_with_credentials_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(xhr) = args.this().to_object(scope) else {
        return;
    };
    let ready_state = xhr_state_number_property(scope, xhr, XHR_READY_STATE_SLOT).unwrap_or(0.0);
    let send_flag = xhr_state_bool_property(scope, xhr, XHR_SEND_FLAG_SLOT).unwrap_or(false);
    if ready_state as u32 != 0 && (ready_state as u32 != 1 || send_flag) {
        xhr_throw_invalid_state(
            scope,
            "XMLHttpRequest.withCredentials cannot be changed in the current state.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<XhrWithCredentialsArgs>(scope, &args) else {
        return;
    };
    set_xhr_state_bool(scope, xhr, XHR_WITH_CREDENTIALS_SLOT, parsed.value);
}

fn xhr_value_setter(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(xhr) = args.this().to_object(scope) else {
        return;
    };
    let Some(key) = callback_data_item(
        scope,
        &args,
        XHR_CALLBACK_DATA_SLOTS,
        "XMLHttpRequest callback data slots",
    ) else {
        return;
    };
    let value = if args.length() > 0 {
        args.get(0)
    } else {
        v8::undefined(scope).into()
    };
    let stored = if xhr_event_type_for_handler_slot(key).is_some() {
        if value.is_function() {
            value
        } else {
            v8::null(scope).into()
        }
    } else {
        value
    };
    set_xhr_state_value(scope, xhr, key, stored);
    set_private_value(scope, xhr, key, stored);
    if let Some(event_type) = xhr_event_type_for_handler_slot(key) {
        simple_object_event_set_ordered_handler(
            scope,
            xhr,
            XHR_SIMPLE_EVENT_TARGET_LISTENERS_SLOT,
            event_type,
            key,
            stored.is_function(),
        );
    }
}

const XHR_CALLBACK_DATA_SLOTS: &[&str] = &[
    XHR_READY_STATE_SLOT,
    XHR_STATUS_SLOT,
    XHR_STATUS_TEXT_SLOT,
    XHR_RESPONSE_TEXT_SLOT,
    XHR_RESPONSE_TYPE_SLOT,
    XHR_RESPONSE_URL_SLOT,
    XHR_RESPONSE_SLOT,
    XHR_RESPONSE_XML_SLOT,
    XHR_TIMEOUT_SLOT,
    XHR_WITH_CREDENTIALS_SLOT,
    "onreadystatechange",
    "__lmXhrUpload",
    "onload",
    "onerror",
    "onprogress",
    "onabort",
    "ontimeout",
    "onloadstart",
    "onloadend",
];

fn xhr_event_type_for_handler_slot(slot: &'static str) -> Option<&'static str> {
    slot.strip_prefix("on")
}
