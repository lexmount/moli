//! PaymentRequest argument validation and frontend state for an empty payment
//! handler registry. No payment sheet or successful payment is synthesized.

mod conversion;
use super::{media_queries, shared};
use crate::{
    document_runtime::DomHandle,
    native_bridge::{
        JsContextHost, OwnerDispatchScope, WindowEnvironmentSettings,
        WindowExecutionContextIdentity,
        document::{document_is_fully_active, document_is_hidden},
    },
    util::{context_host_ptr_from_global_bridge, get_private_value, set_private_value, v8str},
    web_api_interfaces, webidl,
};
use conversion::{Args, Text};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, WebApiValue};

const ID: &str = "__moliPaymentRequestId";
const SHIPPING_OPTION: &str = "__moliPaymentRequestShippingOption";
const SHIPPING_TYPE: &str = "__moliPaymentRequestShippingType";
const STATE: &str = "__moliPaymentRequestState";
const LISTENERS: &str = "__moliPaymentRequestListeners";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PaymentRequest)]
struct RequestSlots<'s> {
    #[webapi(slot = ID)]
    id: Text,
    #[webapi(slot = SHIPPING_OPTION)]
    shipping_option: v8::Local<'s, v8::Value>,
    #[webapi(slot = SHIPPING_TYPE)]
    shipping_type: v8::Local<'s, v8::Value>,
    #[webapi(slot = "__moliPaymentRequestDetails")]
    details: v8::Local<'s, v8::Object>,
    #[webapi(slot = "__moliPaymentRequestOptions")]
    options: v8::Local<'s, v8::Object>,
    #[webapi(slot = "__moliPaymentRequestMethods")]
    methods: v8::Local<'s, v8::Array>,
    #[webapi(slot = "__moliPaymentRequestMethodData")]
    method_data: v8::Local<'s, v8::Value>,
    #[webapi(slot = "__moliPaymentRequestModifierData")]
    modifier_data: v8::Local<'s, v8::Value>,
    #[webapi(slot = STATE, value = "created")]
    state: (),
    #[webapi(slot = shared::SIMPLE_EVENT_TARGET_SLOT, value = LISTENERS)]
    event_target: (),
    #[webapi(slot = shared::SIMPLE_EVENT_TARGET_ORDERED_HANDLERS_SLOT, init = true)]
    ordered_handlers: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::PaymentRequest, enumerable, receiver)]
struct RequestPrototype {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, ID))]
    id: (),
    #[webapi(accessor_property, getter = shipping_address)]
    shipping_address: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, SHIPPING_OPTION))]
    shipping_option: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, SHIPPING_TYPE))]
    shipping_type: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = v8str(scope, "shippingaddresschange"))]
    onshippingaddresschange: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = v8str(scope, "shippingoptionchange"))]
    onshippingoptionchange: (),
    #[webapi(accessor_property, getter = handler_getter, setter = handler_setter, data = v8str(scope, "paymentmethodchange"))]
    onpaymentmethodchange: (),
    #[webapi(method, returns_promise, length = 0, callback = show)]
    show: (),
    #[webapi(method, returns_promise, length = 0, callback = abort)]
    abort: (),
    #[webapi(method, returns_promise, length = 0, callback = can_make_payment)]
    can_make_payment: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "PaymentRequest.show")]
struct ShowArgs<'s> {
    _details_promise: Option<v8::Local<'s, v8::Promise>>,
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    if name == "PaymentRequest" {
        RequestPrototype::initialize_prototype_template(scope, template.prototype_template(scope));
    }
}

struct Owner {
    host: *mut JsContextHost,
    identity: WindowExecutionContextIdentity,
    document: DomHandle,
}

fn owner<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) -> Option<Owner> {
    let realm = object.get_creation_context(scope)?;
    let (host_ptr, settings) = {
        let scope = &mut v8::ContextScope::new(scope, realm);
        (
            context_host_ptr_from_global_bridge(scope)?,
            WindowEnvironmentSettings::for_current_realm(scope),
        )
    };
    // The native bridge owns this host during the callback. An old retained
    // realm keeps its Document, but cannot acquire a replacement Window's rights.
    let host = unsafe { &*host_ptr };
    let identity = host.window_execution_context_identity_for_v8_context(scope, realm)?;
    if !host.window_execution_context_identity_is_current(identity) {
        return None;
    }
    // Window.document is lazy during bootstrap. Only a verified current realm
    // may use its live route before retained Document settings have been bound.
    let document = if let Some(settings) = settings {
        settings.document_handle()
    } else {
        match identity.dispatch_scope() {
            OwnerDispatchScope::Top => host.document_handle(),
            OwnerDispatchScope::Child(handle) => {
                host.child_browsing_context_document_handle(handle)?
            }
            OwnerDispatchScope::LightweightPopup(id) => {
                host.lightweight_popup_document_handle(id)?
            }
        }
    };
    Some(Owner {
        host: host_ptr,
        identity,
        document,
    })
}

pub(super) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        crate::util::throw_type_error(scope, "PaymentRequest must be constructed with new.");
        return;
    }
    let Some(mut parsed) = webidl::parse_args::<Args>(scope, &args) else {
        return;
    };
    let allowed = owner(scope, args.this()).is_some_and(|owner| {
        let host = unsafe { &*owner.host };
        host.window_execution_context_identity_is_current(owner.identity)
            && host
                .document_permissions_policy_for_owner(owner.identity.dispatch_scope())
                .is_some_and(|policy| policy.payment_enabled())
    });
    if !allowed {
        webidl::throw_dom_exception(
            scope,
            "SecurityError",
            "Payment is disallowed by the document's permissions policy.",
        );
        return;
    }
    if parsed.details.id.is_none() {
        let mut bytes = [0_u8; 16];
        if let Err(error) = moli_crypto::fill_secure_random(&mut bytes) {
            super::throw_error_exception(scope, &format!("Payment identity failed: {error}"));
            return;
        }
        parsed.details.id = Some(Text::from_string(
            &uuid::Builder::from_random_bytes(bytes)
                .into_uuid()
                .to_string(),
        ));
    }
    let Some(method_data) = conversion::process_methods(scope, &mut parsed.method_data) else {
        return;
    };
    let Some((shipping_option, modifier_data)) =
        conversion::process_details(scope, &mut parsed.details, parsed.options.request_shipping)
    else {
        return;
    };
    let shipping_type = if parsed.options.request_shipping {
        parsed
            .options
            .shipping_type
            .to_v8_value(scope)
            .expect("shipping type")
    } else {
        v8::null(scope).into()
    };
    let shipping_option = match shipping_option {
        Some(value) => value.to_v8_value(scope).expect("shipping option"),
        None => v8::null(scope).into(),
    };
    let id = parsed
        .details
        .id
        .clone()
        .expect("assigned payment identifier");
    let details = parsed.details.into_snapshot(scope);
    let options = parsed
        .options
        .bind(scope)
        .expect("converted PaymentOptions");
    let methods = parsed
        .method_data
        .into_iter()
        .map(|method| method.bind(scope).expect("converted payment method").into())
        .collect::<Vec<_>>();
    let methods = v8::Array::new_with_elements(scope, &methods);
    let method_data = method_data
        .to_v8_value(scope)
        .expect("serialized payment method data");
    let modifier_data = modifier_data
        .to_v8_value(scope)
        .expect("serialized payment modifier data");
    RequestSlots::new(
        id,
        shipping_option,
        shipping_type,
        details,
        options,
        methods,
        method_data,
        modifier_data,
    )
    .initialize(scope, args.this())
    .expect("native PaymentRequest slots");
    rv.set(args.this().into());
}

fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, object)
        .expect("validated PaymentRequest receiver")
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = target(scope, args.this());
    let slot = args.data().to_rust_string_lossy(scope);
    rv.set(get_private_value(scope, object, &slot).expect("native payment value"));
}

fn shipping_address<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set(v8::null(scope).into());
}

fn handler_slot(event: &str) -> &'static str {
    match event {
        "shippingaddresschange" => "__moliPaymentRequestOnshippingaddresschange",
        "shippingoptionchange" => "__moliPaymentRequestOnshippingoptionchange",
        "paymentmethodchange" => "__moliPaymentRequestOnpaymentmethodchange",
        _ => unreachable!("native payment handler name"),
    }
}

fn handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = target(scope, args.this());
    let event = args.data().to_rust_string_lossy(scope);
    rv.set(
        get_private_value(scope, object, handler_slot(&event))
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = target(scope, args.this());
    let event = args.data().to_rust_string_lossy(scope);
    let slot = handler_slot(&event);
    let active = args.get(0).is_object();
    let value = if active {
        args.get(0)
    } else {
        v8::null(scope).into()
    };
    set_private_value(scope, object, slot, value);
    media_queries::simple_object_event_set_ordered_handler(
        scope, object, LISTENERS, &event, slot, active,
    );
}

fn created<'s>(scope: &mut v8::PinScope<'s, '_>, object: v8::Local<'s, v8::Object>) -> bool {
    get_private_value(scope, object, STATE)
        .expect("native payment state")
        .strict_equals(v8str(scope, "created").into())
}

fn active(owner: &Owner) -> bool {
    let host = unsafe { &*owner.host };
    host.window_execution_context_identity_is_current(owner.identity)
        && document_is_fully_active(host, owner.document)
}

fn can_make_payment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let object = target(scope, args.this());
    if !created(scope, object) || owner(scope, object).is_none_or(|owner| !active(&owner)) {
        webidl::throw_dom_exception(
            scope,
            "InvalidStateError",
            "The payment request is not created in an active document.",
        );
        return;
    }
    let promise = v8::PromiseResolver::new(scope).expect("payment capability promise");
    promise.resolve(scope, v8::Boolean::new(scope, false).into());
    rv.set(promise.get_promise(scope).into());
}

fn abort<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    // An empty registry never creates an interactive request or payment sheet.
    webidl::throw_dom_exception(
        scope,
        "InvalidStateError",
        "No interactive payment request can be aborted.",
    );
}

fn show<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    if webidl::parse_args::<ShowArgs>(scope, &args).is_none() {
        return;
    }
    let object = target(scope, args.this());
    let Some(owner) = owner(scope, object) else {
        webidl::throw_dom_exception(
            scope,
            "SecurityError",
            "Payment presentation requires transient user activation.",
        );
        return;
    };
    let activated = {
        let host = unsafe { &mut *owner.host };
        let activated = host.window_has_transient_user_activation(owner.identity.dispatch_scope())
            || host.protocol_user_gesture_activation();
        if activated {
            host.consume_window_user_activation(owner.identity.dispatch_scope());
        }
        activated
    };
    if !activated {
        webidl::throw_dom_exception(
            scope,
            "SecurityError",
            "Payment presentation requires transient user activation.",
        );
        return;
    }
    if !active(&owner) {
        webidl::throw_dom_exception(
            scope,
            "InvalidStateError",
            "The payment document is inactive.",
        );
        return;
    }
    if document_is_hidden(unsafe { &*owner.host }, owner.document) {
        webidl::throw_dom_exception(scope, "AbortError", "The payment document is hidden.");
        return;
    }
    if !created(scope, object) {
        webidl::throw_dom_exception(scope, "InvalidStateError", "The payment request is closed.");
        return;
    }
    set_private_value(scope, object, STATE, v8str(scope, "closed").into());
    webidl::throw_dom_exception(
        scope,
        "NotSupportedError",
        "No registered payment handler supports this request.",
    );
}
