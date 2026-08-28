use super::*;
use crate::{util::get_private_value, webidl};
use moli_web_errors::{DOM_EXCEPTION_CONSTANTS, dom_exception_legacy_code};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, WebApiTemplateValue};
use std::cell::RefCell;

const DOM_EXCEPTION_BRAND_SLOT: &str = "__lmDomExceptionBrand";
const DOM_EXCEPTION_MESSAGE_SLOT: &str = "__lmDomExceptionMessage";
const DOM_EXCEPTION_NAME_SLOT: &str = "__lmDomExceptionName";
const DOM_EXCEPTION_CODE_SLOT: &str = "__lmDomExceptionCode";
const DOM_ERROR_BRAND_SLOT: &str = "__lmDomErrorBrand";
const DOM_ERROR_MESSAGE_SLOT: &str = "__lmDomErrorMessage";
const DOM_ERROR_NAME_SLOT: &str = "__lmDomErrorName";
const QUOTA_EXCEEDED_ERROR_BRAND_SLOT: &str = "__lmQuotaExceededErrorBrand";
const QUOTA_EXCEEDED_ERROR_QUOTA_SLOT: &str = "__lmQuotaExceededErrorQuota";
const QUOTA_EXCEEDED_ERROR_REQUESTED_SLOT: &str = "__lmQuotaExceededErrorRequested";
const WEBSOCKET_ERROR_BRAND_SLOT: &str = "__lmWebSocketErrorBrand";
const WEBSOCKET_ERROR_CLOSE_CODE_SLOT: &str = "__lmWebSocketErrorCloseCode";
const WEBSOCKET_ERROR_REASON_SLOT: &str = "__lmWebSocketErrorReason";

#[derive(Debug)]
pub(crate) struct DomExceptionPrototypeSlot {
    prototype: RefCell<DomExceptionPrototypeHandle>,
}

#[derive(Debug)]
enum DomExceptionPrototypeHandle {
    Strong(v8::Global<v8::Object>),
    Weak(v8::Weak<v8::Object>),
}

impl DomExceptionPrototypeHandle {
    fn to_local<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Object>> {
        match self {
            Self::Strong(prototype) => Some(v8::Local::new(scope, prototype)),
            Self::Weak(prototype) => prototype.to_local(scope),
        }
    }

    fn weaken(&mut self, scope: &mut v8::PinScope<'_, '_>) {
        let Self::Strong(prototype) = self else {
            return;
        };
        let prototype = v8::Local::new(scope, &*prototype);
        *self = Self::Weak(v8::Weak::new(scope, prototype));
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMException")]
struct DomExceptionConstructorArgs {
    #[webidl(default = "")]
    message: String,
    #[webidl(default = "Error")]
    name: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "DOMError")]
struct DomErrorConstructorArgs {
    #[webidl(required)]
    name: String,
    #[webidl(default = "")]
    message: String,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "QuotaExceededErrorOptions")]
struct QuotaExceededErrorOptions {
    #[webidl(converter = "double")]
    quota: Option<f64>,
    #[webidl(converter = "double")]
    requested: Option<f64>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "QuotaExceededError")]
struct QuotaExceededErrorConstructorArgs {
    #[webidl(default = "")]
    message: String,
    #[webidl(default = QuotaExceededErrorOptions::default(), with = quota_exceeded_error_options_arg)]
    options: QuotaExceededErrorOptions,
}

#[derive(WebApiObject)]
#[webapi(interface = "DOMException")]
struct DomExceptionObjectDeclaration<'scope> {
    #[webapi(slot = DOM_EXCEPTION_BRAND_SLOT, init = true)]
    brand: (),
    #[webapi(slot = DOM_EXCEPTION_MESSAGE_SLOT)]
    message: Option<v8::Local<'scope, v8::String>>,
    #[webapi(slot = DOM_EXCEPTION_NAME_SLOT)]
    name: Option<v8::Local<'scope, v8::String>>,
    #[webapi(slot = DOM_EXCEPTION_CODE_SLOT)]
    code: f64,
}

#[derive(WebApiObject)]
#[webapi(interface = "DOMError", fallback_to_string_tag = "DOMError")]
struct DomErrorObjectDeclaration<'scope> {
    #[webapi(slot = DOM_ERROR_BRAND_SLOT, init = true)]
    brand: (),
    #[webapi(slot = DOM_ERROR_NAME_SLOT)]
    name: Option<v8::Local<'scope, v8::String>>,
    #[webapi(slot = DOM_ERROR_MESSAGE_SLOT)]
    message: Option<v8::Local<'scope, v8::String>>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    name = "DOMException",
    intrinsic_prototype_parent = v8::Intrinsic::ErrorPrototype
)]
struct DomExceptionPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = dom_exception_message_getter_callback, enumerable)]
    message: (),
    #[webapi(accessor_property, getter = dom_exception_name_getter_callback, enumerable)]
    name: (),
    #[webapi(accessor_property, getter = dom_exception_code_getter_callback, enumerable)]
    code: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DOMError")]
struct DomErrorPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = dom_error_name_getter_callback, enumerable)]
    name: (),
    #[webapi(accessor_property, getter = dom_error_message_getter_callback, enumerable)]
    message: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "QuotaExceededError")]
struct QuotaExceededErrorObjectDeclaration<'scope> {
    #[webapi(slot = DOM_EXCEPTION_BRAND_SLOT, init = true)]
    dom_exception_brand: (),
    #[webapi(slot = DOM_EXCEPTION_MESSAGE_SLOT)]
    message: Option<v8::Local<'scope, v8::String>>,
    #[webapi(slot = DOM_EXCEPTION_NAME_SLOT, init = string("QuotaExceededError"))]
    name: (),
    #[webapi(slot = DOM_EXCEPTION_CODE_SLOT, value = f64::from(dom_exception_legacy_code("QuotaExceededError")))]
    code: (),
    #[webapi(slot = QUOTA_EXCEEDED_ERROR_BRAND_SLOT, init = true)]
    quota_exceeded_error_brand: (),
    #[webapi(slot = QUOTA_EXCEEDED_ERROR_QUOTA_SLOT)]
    quota: v8::Local<'scope, v8::Value>,
    #[webapi(slot = QUOTA_EXCEEDED_ERROR_REQUESTED_SLOT)]
    requested: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "QuotaExceededError")]
struct QuotaExceededErrorPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = quota_exceeded_error_quota_getter_callback, enumerable)]
    quota: (),
    #[webapi(
        accessor_property,
        getter = quota_exceeded_error_requested_getter_callback,
        enumerable
    )]
    requested: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "WebSocketError")]
struct WebSocketErrorObjectDeclaration<'scope> {
    #[webapi(slot = DOM_EXCEPTION_BRAND_SLOT, init = true)]
    dom_exception_brand: (),
    #[webapi(slot = DOM_EXCEPTION_MESSAGE_SLOT)]
    message: Option<v8::Local<'scope, v8::String>>,
    #[webapi(slot = DOM_EXCEPTION_NAME_SLOT, init = string("WebSocketError"))]
    name: (),
    #[webapi(slot = DOM_EXCEPTION_CODE_SLOT, value = f64::from(dom_exception_legacy_code("WebSocketError")))]
    code: (),
    #[webapi(slot = WEBSOCKET_ERROR_BRAND_SLOT, init = true)]
    websocket_error_brand: (),
    #[webapi(slot = WEBSOCKET_ERROR_CLOSE_CODE_SLOT)]
    close_code: v8::Local<'scope, v8::Value>,
    #[webapi(slot = WEBSOCKET_ERROR_REASON_SLOT)]
    reason: v8::Local<'scope, v8::String>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "WebSocketError")]
struct WebSocketErrorPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = websocket_error_close_code_getter_callback, enumerable)]
    close_code: (),
    #[webapi(accessor_property, getter = websocket_error_reason_getter_callback, enumerable)]
    reason: (),
}

pub(crate) fn install_dom_exception_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "DOMException" => {
            DomExceptionPrototypeAccessorsDeclaration::initialize_template(scope, template);
            for constant in DOM_EXCEPTION_CONSTANTS {
                let value = u32::from(constant.value);
                let value = WebApiTemplateValue::to_v8_template_value(&value, scope)
                    .expect("DOMException constant should convert to a template value");
                let key = v8str(scope, constant.property);
                template.set_with_attr(
                    key.into(),
                    value.into(),
                    v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE,
                );
                prototype.set_with_attr(
                    key.into(),
                    value.into(),
                    v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE,
                );
            }
            DomExceptionPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "DOMError" => {
            DomErrorPrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
        }
        "QuotaExceededError" => {
            QuotaExceededErrorPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "WebSocketError" => {
            WebSocketErrorPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

pub(crate) fn finalize_dom_exception_realm_bindings(
    scope: &mut v8::PinScope<'_, '_>,
    prototype: v8::Local<'_, v8::Object>,
) {
    let _ = scope
        .get_current_context()
        .set_slot(std::rc::Rc::new(DomExceptionPrototypeSlot {
            prototype: RefCell::new(DomExceptionPrototypeHandle::Strong(v8::Global::new(
                scope, prototype,
            ))),
        }));
}

pub(crate) fn weaken_dom_exception_prototype_for_context_teardown(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<std::rc::Rc<DomExceptionPrototypeSlot>> {
    let slot = scope
        .get_current_context()
        .get_slot::<DomExceptionPrototypeSlot>()?;
    slot.prototype.borrow_mut().weaken(scope);
    Some(slot)
}

fn dom_exception_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if get_private_value(scope, receiver, DOM_EXCEPTION_BRAND_SLOT).is_some() {
        Some(receiver)
    } else {
        throw_type_error(
            scope,
            "DOMException getter called on incompatible receiver.",
        );
        None
    }
}

fn dom_error_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if get_private_value(scope, receiver, DOM_ERROR_BRAND_SLOT).is_some() {
        Some(receiver)
    } else {
        throw_type_error(scope, "DOMError getter called on incompatible receiver.");
        None
    }
}

pub(crate) fn dom_exception_clone_fields<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(String, String)> {
    get_private_value(scope, object, DOM_EXCEPTION_BRAND_SLOT)?;
    let message = get_private_value(scope, object, DOM_EXCEPTION_MESSAGE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let name = get_private_value(scope, object, DOM_EXCEPTION_NAME_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "Error".to_owned());
    Some((message, name))
}

pub(crate) fn quota_exceeded_error_clone_fields<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<(String, Option<f64>, Option<f64>)> {
    get_private_value(scope, object, QUOTA_EXCEEDED_ERROR_BRAND_SLOT)?;
    let (message, _) = dom_exception_clone_fields(scope, object)?;
    let quota = nullable_double_clone_slot(scope, object, QUOTA_EXCEEDED_ERROR_QUOTA_SLOT)?;
    let requested = nullable_double_clone_slot(scope, object, QUOTA_EXCEEDED_ERROR_REQUESTED_SLOT)?;
    Some((message, quota, requested))
}

fn nullable_double_clone_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<Option<f64>> {
    let value = get_private_value(scope, object, slot)?;
    if value.is_null() {
        Some(None)
    } else {
        value.number_value(scope).map(Some)
    }
}

fn dom_exception_message_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(receiver) = dom_exception_receiver(scope, args.this()) else {
        return;
    };
    let message = get_private_value(scope, receiver, DOM_EXCEPTION_MESSAGE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if let Some(message) = v8_string(scope, &message) {
        rv.set(message.into());
    } else {
        rv.set_empty_string();
    }
}

fn dom_exception_name_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(receiver) = dom_exception_receiver(scope, args.this()) else {
        return;
    };
    let name = get_private_value(scope, receiver, DOM_EXCEPTION_NAME_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "Error".to_owned());
    if let Some(name) = v8_string(scope, &name) {
        rv.set(name.into());
    } else {
        rv.set_empty_string();
    }
}

fn dom_exception_code_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(receiver) = dom_exception_receiver(scope, args.this()) else {
        return;
    };
    let code = get_private_value(scope, receiver, DOM_EXCEPTION_CODE_SLOT)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0);
    rv.set(v8::Number::new(scope, code).into());
}

fn dom_error_name_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(receiver) = dom_error_receiver(scope, args.this()) else {
        return;
    };
    let name = get_private_value(scope, receiver, DOM_ERROR_NAME_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if let Some(name) = v8_string(scope, &name) {
        rv.set(name.into());
    } else {
        rv.set_empty_string();
    }
}

fn dom_error_message_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(receiver) = dom_error_receiver(scope, args.this()) else {
        return;
    };
    let message = get_private_value(scope, receiver, DOM_ERROR_MESSAGE_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if let Some(message) = v8_string(scope, &message) {
        rv.set(message.into());
    } else {
        rv.set_empty_string();
    }
}

fn quota_exceeded_error_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    dom_exception_subclass_receiver(
        scope,
        receiver,
        QUOTA_EXCEEDED_ERROR_BRAND_SLOT,
        "QuotaExceededError",
    )
}

fn websocket_error_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    dom_exception_subclass_receiver(
        scope,
        receiver,
        WEBSOCKET_ERROR_BRAND_SLOT,
        "WebSocketError",
    )
}

fn dom_exception_subclass_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    brand_slot: &'static str,
    interface_name: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    if get_private_value(scope, receiver, brand_slot).is_some() {
        Some(receiver)
    } else {
        throw_type_error(
            scope,
            &format!("{interface_name} getter called on incompatible receiver."),
        );
        None
    }
}

fn quota_exceeded_error_slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    slot: &'static str,
) {
    let Some(receiver) = quota_exceeded_error_receiver(scope, args.this()) else {
        return;
    };
    match get_private_value(scope, receiver, slot) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

fn quota_exceeded_error_quota_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    quota_exceeded_error_slot_getter(scope, args, rv, QUOTA_EXCEEDED_ERROR_QUOTA_SLOT);
}

fn quota_exceeded_error_requested_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    quota_exceeded_error_slot_getter(scope, args, rv, QUOTA_EXCEEDED_ERROR_REQUESTED_SLOT);
}

fn websocket_error_slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
    slot: &'static str,
) {
    let Some(receiver) = websocket_error_receiver(scope, args.this()) else {
        return;
    };
    match get_private_value(scope, receiver, slot) {
        Some(value) => rv.set(value),
        None => rv.set_null(),
    }
}

fn websocket_error_close_code_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    websocket_error_slot_getter(scope, args, rv, WEBSOCKET_ERROR_CLOSE_CODE_SLOT);
}

fn websocket_error_reason_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(receiver) = websocket_error_receiver(scope, args.this()) else {
        return;
    };
    let reason = get_private_value(scope, receiver, WEBSOCKET_ERROR_REASON_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if let Some(reason) = v8_string(scope, &reason) {
        rv.set(reason.into());
    } else {
        rv.set_empty_string();
    }
}

fn initialize_dom_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exception: v8::Local<'s, v8::Object>,
    message: &str,
    name: &str,
) {
    dom_exception_declaration(scope, message, name)
        .initialize(scope, exception)
        .expect("DOMException declaration should initialize");
}

fn dom_exception_declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    name: &str,
) -> DomExceptionObjectDeclaration<'s> {
    DomExceptionObjectDeclaration::new(
        v8_string(scope, message),
        v8_string(scope, name),
        f64::from(dom_exception_legacy_code(name)),
    )
}

fn nullable_double_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<f64>,
) -> v8::Local<'s, v8::Value> {
    match value {
        Some(value) => v8::Number::new(scope, value).into(),
        None => v8::null(scope).into(),
    }
}

fn quota_exceeded_error_declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    quota: Option<f64>,
    requested: Option<f64>,
) -> QuotaExceededErrorObjectDeclaration<'s> {
    let quota = nullable_double_slot_value(scope, quota);
    let requested = nullable_double_slot_value(scope, requested);
    QuotaExceededErrorObjectDeclaration::new(v8_string(scope, message), quota, requested)
}

fn initialize_quota_exceeded_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exception: v8::Local<'s, v8::Object>,
    message: &str,
    quota: Option<f64>,
    requested: Option<f64>,
) {
    quota_exceeded_error_declaration(scope, message, quota, requested)
        .initialize(scope, exception)
        .expect("QuotaExceededError declaration should initialize");
}

fn websocket_error_declaration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    close_code: Option<u16>,
    reason: &str,
) -> WebSocketErrorObjectDeclaration<'s> {
    let close_code = nullable_double_slot_value(scope, close_code.map(f64::from));
    let reason = v8_string(scope, reason).unwrap_or_else(|| v8::String::empty(scope));
    WebSocketErrorObjectDeclaration::new(v8_string(scope, message), close_code, reason)
}

pub(crate) fn new_dom_exception_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    name: &str,
) -> v8::Local<'s, v8::Value> {
    if let Some(prototype) = scope
        .get_current_context()
        .get_slot::<DomExceptionPrototypeSlot>()
        .and_then(|slot| slot.prototype.borrow().to_local(scope))
    {
        let exception = v8::Object::new(scope);
        initialize_dom_exception(scope, exception, message, name);
        let _ = exception.set_prototype(scope, prototype.into());
        return exception.into();
    }
    dom_exception_declaration(scope, message, name)
        .bind(scope)
        .expect("DOMException declaration should bind")
        .into()
}

pub(crate) fn throw_dom_exception_value(
    scope: &mut v8::PinScope<'_, '_>,
    message: &str,
    name: &str,
) {
    let exception = new_dom_exception_value(scope, message, name);
    scope.throw_exception(exception);
}

pub(crate) fn new_most_derived_dom_exception_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    name: &str,
) -> v8::Local<'s, v8::Value> {
    match name {
        "QuotaExceededError" => new_quota_exceeded_error_value(scope, message, None, None),
        "WebSocketError" => new_websocket_error_value(scope, message, None, ""),
        _ => new_dom_exception_value(scope, message, name),
    }
}

pub(crate) fn dom_exception_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'DOMException': Please use the 'new' operator.",
        );
        return;
    }

    let Some(parsed) = webidl::parse_args::<DomExceptionConstructorArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let exception = args.this();
    initialize_dom_exception(scope, exception, &parsed.message, &parsed.name);
    rv.set(args.this().into());
}

pub(crate) fn dom_error_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'DOMError': Please use the 'new' operator.",
        );
        return;
    }

    let Some(parsed) = webidl::parse_args::<DomErrorConstructorArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    DomErrorObjectDeclaration::new(
        v8_string(scope, &parsed.name),
        v8_string(scope, &parsed.message),
    )
    .initialize(scope, args.this())
    .expect("DOMError declaration should initialize");
    rv.set(args.this().into());
}

pub(crate) fn new_dom_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    message: &str,
) -> v8::Local<'s, v8::Value> {
    crate::context_bootstrap::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope, "DOMError",
    )
    .expect("DOMError prototype should materialize before callback delivery");
    DomErrorObjectDeclaration::new(v8_string(scope, name), v8_string(scope, message))
        .bind(scope)
        .expect("DOMError declaration should bind")
        .into()
}

fn quota_exceeded_error_options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<QuotaExceededErrorOptions, webidl::WebIdlError> {
    let context = webidl::Context::argument("QuotaExceededError", (index + 1) as usize);
    if args.length() <= index {
        return Ok(QuotaExceededErrorOptions::default());
    }
    let value = args.get(index);
    webidl::parse_dictionary::<QuotaExceededErrorOptions>(scope, value, context)
        .map(|value| value.unwrap_or_default())
}

pub(crate) fn quota_exceeded_error_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'QuotaExceededError': Please use the 'new' operator.",
        );
        return;
    }

    let Some(parsed) = webidl::parse_args::<QuotaExceededErrorConstructorArgs>(scope, &args) else {
        rv.set_undefined();
        return;
    };
    let exception = args.this();
    initialize_quota_exceeded_error(
        scope,
        exception,
        &parsed.message,
        parsed.options.quota,
        parsed.options.requested,
    );
    rv.set(exception.into());
}

pub(crate) fn new_quota_exceeded_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    quota: Option<f64>,
    requested: Option<f64>,
) -> v8::Local<'s, v8::Value> {
    quota_exceeded_error_declaration(scope, message, quota, requested)
        .bind(scope)
        .expect("QuotaExceededError declaration should bind")
        .into()
}

pub(crate) fn initialize_websocket_error<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exception: v8::Local<'s, v8::Object>,
    message: &str,
    close_code: Option<u16>,
    reason: &str,
) {
    websocket_error_declaration(scope, message, close_code, reason)
        .initialize(scope, exception)
        .expect("WebSocketError declaration should initialize");
}

pub(crate) fn new_websocket_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    message: &str,
    close_code: Option<u16>,
    reason: &str,
) -> v8::Local<'s, v8::Value> {
    websocket_error_declaration(scope, message, close_code, reason)
        .bind(scope)
        .expect("WebSocketError declaration should bind")
        .into()
}

pub(crate) fn websocket_error_close_info<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<(Option<u16>, String)> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    get_private_value(scope, object, WEBSOCKET_ERROR_BRAND_SLOT)?;
    let close_code_value = get_private_value(scope, object, WEBSOCKET_ERROR_CLOSE_CODE_SLOT)?;
    let reason_value = get_private_value(scope, object, WEBSOCKET_ERROR_REASON_SLOT)?;
    let close_code = if close_code_value.is_null_or_undefined() {
        None
    } else {
        close_code_value
            .number_value(scope)
            .filter(|value| value.is_finite() && value.fract() == 0.0)
            .map(|value| value as u16)
    };
    let reason = reason_value
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    Some((close_code, reason))
}
