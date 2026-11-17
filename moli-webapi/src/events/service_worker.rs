//! Service Worker event values. Dispatch associations and waitUntil remain with the worker.
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{throw_type_error, v8_string_from_utf16_units};
use moli_webapi_declare::WebApiObject;
use moli_webidl as webidl;
use webidl::WebIdlDictionarySerialize;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ExtendableEvent")]
struct ExtendableEventConstructorArgs {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(dictionary)]
    init: super::init::EventInit,
}

// Interface identity, rather than live endpoint IDs or author-visible
// scriptURL properties, determines membership in this WebIDL union.
struct ExtendableMessageEventSource;

impl ExtendableMessageEventSource {
    const NAME: &'static str = "ExtendableMessageEventSource";

    fn is_instance<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> bool {
        web_api_interfaces::Client::is_instance(scope, object)
            || web_api_interfaces::ServiceWorker::is_instance(scope, object)
            || web_api_interfaces::MessagePort::is_instance(scope, object)
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ExtendableMessageEventInit", snapshot)]
struct ExtendableMessageEventInit<'s> {
    #[webidl(inherit)]
    base: super::init::EventInit,
    #[webidl(converter = "raw", snapshot_default = v8::null(scope).into())]
    data: Option<v8::Local<'s, v8::Value>>,
    #[webidl(name = "lastEventId", default = webidl::DomString16(Vec::new()), converter = "raw")]
    last_event_id: webidl::DomString16,
    #[webidl(default = "", converter = "usv_string")]
    origin: String,
    #[webidl(sequence, interface = web_api_interfaces::MessagePort, default = Vec::new(), snapshot_frozen)]
    ports: Vec<v8::Local<'s, v8::Object>>,
    #[webidl(nullable, interface = ExtendableMessageEventSource)]
    source: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ExtendableMessageEvent")]
struct ExtendableMessageEventConstructorArgs<'s> {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(dictionary)]
    init: ExtendableMessageEventInit<'s>,
}

pub fn extendable_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'ExtendableEvent': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<ExtendableEventConstructorArgs>(scope, &args) else {
        return;
    };
    let Some(event_type) = v8_string_from_utf16_units(scope, &parsed.event_type.0) else {
        return;
    };
    initialize_extendable_event_object(scope, args.this(), event_type, &parsed.init);
    rv.set(args.this().into());
}

pub fn extendable_message_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'ExtendableMessageEvent': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<ExtendableMessageEventConstructorArgs>(scope, &args)
    else {
        return;
    };
    let Some(event_type) = v8_string_from_utf16_units(scope, &parsed.event_type.0) else {
        return;
    };
    let event = args.this();
    initialize_extendable_event_object(scope, event, event_type, &parsed.init.base);
    if parsed.init.initialize_own_snapshot(scope, event).is_none() {
        return;
    }
    web_api_interfaces::ExtendableMessageEvent::DESCRIPTOR
        .initialize(scope, event)
        .expect("extendable message event state");
    rv.set(event.into());
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ExtendableEvent, prototype = "Object")]
struct ExtendableEventStateDeclaration {
    #[webapi(data_property, enumerable)]
    composed: bool,
}

fn initialize_extendable_event_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    event_type: v8::Local<'s, v8::String>,
    init: &super::init::EventInit,
) {
    super::initialize_event_object_with_type(
        scope,
        event,
        event_type,
        init.bubbles,
        init.cancelable,
    );
    ExtendableEventStateDeclaration::new(init.composed)
        .initialize(scope, event)
        .expect("extendable event state should initialize");
}
