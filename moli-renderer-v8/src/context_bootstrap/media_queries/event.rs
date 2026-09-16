use crate::{
    context_bootstrap::events::{initialize_event_object, mark_event_trusted},
    util::{get_private_value, throw_type_error, v8str},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const MEDIA_SLOT: &str = "__moliMediaQueryListEventMedia";
const MATCHES_SLOT: &str = "__moliMediaQueryListEventMatches";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaQueryListEvent)]
struct MediaQueryListEventState<'scope> {
    #[webapi(slot = MEDIA_SLOT)]
    media: v8::Local<'scope, v8::String>,
    #[webapi(slot = MATCHES_SLOT)]
    matches: bool,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaQueryListEvent, enumerable, receiver)]
struct MediaQueryListEventPrototype {
    #[webapi(accessor_property, getter = media_getter)]
    media: (),
    #[webapi(accessor_property, getter = matches_getter)]
    matches: (),
}

// Inherited EventInit members precede this dictionary's own members, each
// group in lexicographic order. Convert the complete dictionary only once.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MediaQueryListEventInit")]
struct MediaQueryListEventInit<'s> {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
    #[webidl(default = false)]
    matches: bool,
    #[webidl(with = media_member)]
    media: v8::Local<'s, v8::String>,
}

fn media_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<v8::Local<'s, v8::String>, webidl::WebIdlError> {
    let context = webidl::Context::member("MediaQueryListEventInit", name);
    let Some(value) = webidl::property_result(scope, object, name, context)?
        .filter(|value| !value.is_undefined())
    else {
        return Ok(v8str(scope, ""));
    };
    // Keep the DOMString as UTF-16, including unpaired surrogates.
    value
        .to_string(scope)
        .ok_or_else(|| webidl::WebIdlError::pending_exception(context))
}

fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(scope, "MediaQueryListEvent requires 'new'.");
        return;
    }
    if args.length() == 0 {
        throw_type_error(scope, "MediaQueryListEvent requires an event type.");
        return;
    }
    let Some(event_type) = args.get(0).to_string(scope) else {
        return;
    };
    let parsed = webidl::dictionary_arg(
        &args,
        1,
        webidl::Context::argument("MediaQueryListEvent", 2),
    )
    .and_then(|object| match object {
        Some(object) => webidl::parse_dictionary_object::<MediaQueryListEventInit>(scope, object),
        None => Ok(MediaQueryListEventInit {
            bubbles: false,
            cancelable: false,
            composed: false,
            matches: false,
            media: v8str(scope, ""),
        }),
    });
    let init = match parsed {
        Ok(init) => init,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let event = args.this();
    initialize_event_object(scope, event, "", init.bubbles, init.cancelable);
    let _ = event.set(scope, v8str(scope, "type").into(), event_type.into());
    let _ = event.set(
        scope,
        v8str(scope, "composed").into(),
        v8::Boolean::new(scope, init.composed).into(),
    );
    MediaQueryListEventState::new(init.media, init.matches)
        .initialize(scope, event)
        .expect("MediaQueryListEvent state should initialize");
    rv.set(event.into());
}

pub(in crate::context_bootstrap) fn build_media_query_list_event_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let template = v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
        web_api_interfaces::MediaQueryListEvent,
        constructor
    ))
    .length(1)
    .build(scope);
    let prototype = template.prototype_template(scope);
    MediaQueryListEventPrototype::initialize_prototype_template(scope, prototype);
    template
}

pub(super) fn create_media_query_list_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    media: &str,
    matches: bool,
) -> v8::Local<'s, v8::Object> {
    let media = v8::String::new(scope, media).expect("media query string");
    let event = MediaQueryListEventState::new(media, matches)
        .bind(scope)
        .expect("MediaQueryListEvent should bind");
    initialize_event_object(scope, event, "change", false, false);
    mark_event_trusted(scope, event);
    event
}

fn media_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), MEDIA_SLOT) {
        rv.set(value);
    }
}

fn matches_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(value) = get_private_value(scope, args.this(), MATCHES_SLOT) {
        rv.set(value);
    }
}
