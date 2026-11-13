//! FetchEvent construction and the state shared with native fetch dispatch.

use super::*;
use crate::webidl;

const FETCH_EVENT_PROTOTYPE_SLOT: &str = "__moliFetchEventPrototype";
const REQUEST_SLOT: &str = "__moliFetchEventRequest";
const PRELOAD_RESPONSE_SLOT: &str = "__moliFetchEventPreloadResponse";
const CLIENT_ID_SLOT: &str = "__moliFetchEventClientId";
const RESULTING_CLIENT_ID_SLOT: &str = "__moliFetchEventResultingClientId";
const REPLACES_CLIENT_ID_SLOT: &str = "__moliFetchEventReplacesClientId";
const HANDLED_SLOT: &str = "__moliFetchEventHandled";

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::FetchEvent,
    constructor_callback = fetch_event_constructor_callback,
    constructor_length = 2,
    readonly_prototype,
    receiver = web_api_interfaces::FetchEvent::is_instance
)]
struct FetchEventTemplateDeclaration {
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
    #[webapi(method = "respondWith", enumerable, callback = fetch_event_respond_with_callback, length = 1)]
    respond_with: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct FetchEventConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "FetchEvent")]
    fetch_event: v8::Local<'scope, v8::Function>,
}

#[derive(webidl::WebIdlDictionary)]
struct FetchEventBaseInit {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
}

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
    base: FetchEventBaseInit,
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

pub(super) fn install_service_worker_fetch_event_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let template = FetchEventTemplateDeclaration::build(scope);
    let constructor = template
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to build FetchEvent constructor"))?;
    let prototype = constructor_prototype(scope, constructor, "FetchEvent")?;
    set_worker_to_string_tag(scope, prototype, "FetchEvent");
    let parent = global_constructor_object(scope, "ExtendableEvent")
        .ok_or_else(|| anyhow!("missing ExtendableEvent constructor"))?;
    let parent_prototype = global_constructor_prototype(scope, "ExtendableEvent")
        .ok_or_else(|| anyhow!("missing ExtendableEvent prototype"))?;
    let _ = constructor.set_prototype(scope, parent.into());
    let _ = prototype.set_prototype(scope, parent_prototype.into());
    // Native dispatch must not consult an author-replaced constructor/getter.
    set_private_value(scope, global, FETCH_EVENT_PROTOTYPE_SLOT, prototype.into());
    FetchEventConstructorGlobalDeclaration::new(constructor)
        .initialize(scope, global)
        .map_err(Into::into)
}

fn fetch_event_constructor_callback<'s>(
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
    let state = crate::context_bootstrap::new_event_state(scope);
    crate::context_bootstrap::initialize_event_object_with_type(
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
    if crate::context_bootstrap::initialize_event_wrapper(scope, event, state).is_none() {
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

fn fetch_event_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    if let Some(value) = get_private_value(scope, args.this(), &slot) {
        rv.set(value);
    }
}

fn fetch_event_respond_with_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    if args.length() == 0 {
        throw_type_error(scope, "FetchEvent.respondWith requires an argument.");
        return;
    }
    crate::worker::thread::service_worker_respond_with_callback(scope, args, rv);
}

pub(in crate::worker) fn initialize_native_fetch_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    request: v8::Local<'s, v8::Object>,
    handled: v8::Local<'s, v8::Promise>,
    preload_response: v8::Local<'s, v8::Promise>,
    client_id: &str,
    resulting_client_id: &str,
) {
    let global = scope.get_current_context().global(scope);
    let prototype = get_private_value(scope, global, FETCH_EVENT_PROTOTYPE_SLOT)
        .expect("Service Worker FetchEvent prototype");
    assert_eq!(event.set_prototype(scope, prototype), Some(true));
    crate::context_bootstrap::initialize_event_object(scope, event, "fetch", false, true);
    crate::context_bootstrap::mark_event_trusted(scope, event);
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
