//! FetchEvent dictionaries, promise values and native snapshot construction.
//! respondWith and fetch-dispatch completion belong to the worker runtime.
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{get_private_value, throw_type_error, v8str};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use moli_webidl as webidl;

pub const REQUEST_SLOT: &str = "__moliFetchEventRequest";

pub const PRELOAD_RESPONSE_SLOT: &str = "__moliFetchEventPreloadResponse";

pub const CLIENT_ID_SLOT: &str = "__moliFetchEventClientId";

pub const RESULTING_CLIENT_ID_SLOT: &str = "__moliFetchEventResultingClientId";

pub const REPLACES_CLIENT_ID_SLOT: &str = "__moliFetchEventReplacesClientId";

pub const HANDLED_SLOT: &str = "__moliFetchEventHandled";

// Promise-valued dictionary members need a fresh intrinsic capability even
// when the input is already a native Promise with an author-defined `then`.
struct FetchEventInitPromise<'s>(v8::Local<'s, v8::Promise>);

impl<'s> webidl::WebIdlConverter<'s> for FetchEventInitPromise<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> std::result::Result<Self, webidl::WebIdlError> {
        let error = || webidl::WebIdlError::cannot_convert(context, "Promise");
        let resolver = v8::PromiseResolver::new(scope).ok_or_else(error)?;
        let promise = resolver.get_promise(scope);
        if resolver.resolve(scope, value) == Some(true) {
            Ok(Self(promise))
        } else {
            Err(error())
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "FetchEventInit")]
struct FetchEventInit<'s> {
    #[webidl(inherit)]
    base: super::init::EventInit,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    client_id: webidl::DomString16,
    #[webidl(converter = "raw")]
    handled: Option<FetchEventInitPromise<'s>>,
    #[webidl(converter = "raw")]
    preload_response: Option<FetchEventInitPromise<'s>>,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    replaces_client_id: webidl::DomString16,
    #[webidl(required, interface = web_api_interfaces::Request)]
    request: v8::Local<'s, v8::Object>,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    resulting_client_id: webidl::DomString16,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "FetchEvent")]
struct FetchEventConstructorArgs<'s> {
    #[webidl(required, converter = "raw")]
    event_type: webidl::DomString16,
    #[webidl(required, dictionary)]
    init: FetchEventInit<'s>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::FetchEvent, prototype = "Object")]
struct FetchEventStateDeclaration<'scope> {
    #[webapi(slot = REQUEST_SLOT)]
    request: v8::Local<'scope, v8::Object>,
    #[webapi(slot = PRELOAD_RESPONSE_SLOT)]
    preload_response: v8::Local<'scope, v8::Promise>,
    #[webapi(slot = CLIENT_ID_SLOT)]
    client_id: v8::Local<'scope, v8::String>,
    #[webapi(slot = RESULTING_CLIENT_ID_SLOT)]
    resulting_client_id: v8::Local<'scope, v8::String>,
    #[webapi(slot = REPLACES_CLIENT_ID_SLOT)]
    replaces_client_id: v8::Local<'scope, v8::String>,
    #[webapi(slot = HANDLED_SLOT)]
    handled: v8::Local<'scope, v8::Promise>,
}

pub fn fetch_event_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "FetchEvent requires the 'new' operator.");
        return;
    }
    let Some(parsed) = webidl::parse_args::<FetchEventConstructorArgs>(scope, &args) else {
        return;
    };
    let init = parsed.init;
    let event_type =
        v8::String::new_from_two_byte(scope, &parsed.event_type.0, v8::NewStringType::Normal)
            .expect("FetchEvent type");
    let state = super::new_event_state(scope);
    super::initialize_event_object_with_type(
        scope,
        state,
        event_type,
        init.base.bubbles,
        init.base.cancelable,
    );
    let _ = state.set(
        scope,
        v8str(scope, "composed").into(),
        v8::Boolean::new(scope, init.base.composed).into(),
    );
    let event = args.this();
    if super::initialize_event_wrapper(scope, event, state).is_none() {
        return;
    }
    let preload_response = init
        .preload_response
        .map(|promise| promise.0)
        .unwrap_or_else(|| {
            let resolver = v8::PromiseResolver::new(scope).expect("FetchEvent preload promise");
            let _ = resolver.resolve(scope, v8::undefined(scope).into());
            resolver.get_promise(scope)
        });
    let handled = init.handled.map(|promise| promise.0).unwrap_or_else(|| {
        v8::PromiseResolver::new(scope)
            .expect("FetchEvent handled promise")
            .get_promise(scope)
    });
    let string = |scope: &mut v8::PinScope<'s, '_>, value: webidl::DomString16| {
        v8::String::new_from_two_byte(scope, &value.0, v8::NewStringType::Normal)
            .expect("FetchEvent client ID")
    };
    FetchEventStateDeclaration::new(
        init.request,
        preload_response,
        string(scope, init.client_id),
        string(scope, init.resulting_client_id),
        string(scope, init.replaces_client_id),
        handled,
    )
    .initialize(scope, event)
    .expect("FetchEvent state");
    rv.set(event.into());
}

pub fn fetch_event_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    if let Some(value) = get_private_value(scope, args.this(), &slot) {
        rv.set(value);
    }
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::FetchEvent,
    readonly_prototype
)]
struct FetchEventValueTemplateDeclaration {
    #[webapi(accessor_property, enumerable, getter = fetch_event_attribute_getter, data = v8str(scope, REQUEST_SLOT))]
    request: (),
    #[webapi(accessor_property, enumerable, getter = fetch_event_attribute_getter, data = v8str(scope, PRELOAD_RESPONSE_SLOT))]
    preload_response: (),
    #[webapi(accessor_property, enumerable, getter = fetch_event_attribute_getter, data = v8str(scope, CLIENT_ID_SLOT))]
    client_id: (),
    #[webapi(accessor_property, enumerable, getter = fetch_event_attribute_getter, data = v8str(scope, RESULTING_CLIENT_ID_SLOT))]
    resulting_client_id: (),
    #[webapi(accessor_property, enumerable, getter = fetch_event_attribute_getter, data = v8str(scope, REPLACES_CLIENT_ID_SLOT))]
    replaces_client_id: (),
    #[webapi(accessor_property, enumerable, getter = fetch_event_attribute_getter, data = v8str(scope, HANDLED_SLOT))]
    handled: (),
}

pub fn install_value_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    FetchEventValueTemplateDeclaration::initialize_prototype_template(scope, prototype);
}

pub fn initialize_native_fetch_event_values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    handled: v8::Local<'s, v8::Promise>,
    preload_response: v8::Local<'s, v8::Promise>,
    client_id: &str,
    resulting_client_id: &str,
) {
    FetchEventStateDeclaration::new(
        request,
        preload_response,
        v8::String::new(scope, client_id).unwrap_or_else(|| v8::String::empty(scope)),
        v8::String::new(scope, resulting_client_id).unwrap_or_else(|| v8::String::empty(scope)),
        v8::String::empty(scope),
        handled,
    )
    .initialize(scope, event)
    .expect("native FetchEvent state");
}
