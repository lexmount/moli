use super::{event_backing, reinitialize_event_object};
use crate::context_bootstrap::is_window_receiver;
use crate::util::{v8_string, v8_string_from_utf16_units};
use crate::{web_api_interfaces, webidl};
use moli_webapi_declare::WebApiObject;

struct MessagePortReference<'s>(v8::Local<'s, v8::Object>);

impl<'s> webidl::WebIdlConverter<'s> for MessagePortReference<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && web_api_interfaces::MessagePort::is_instance(scope, object)
        {
            return Ok(Self(object));
        }
        Err(webidl::WebIdlError::cannot_convert(context, "MessagePort"))
    }
}

struct MessageEventSource<'s>(v8::Local<'s, v8::Object>);

impl<'s> webidl::WebIdlConverter<'s> for MessageEventSource<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
            && (web_api_interfaces::MessagePort::is_instance(scope, object)
                || web_api_interfaces::ServiceWorker::is_instance(scope, object)
                || is_window_receiver(scope, object))
        {
            return Ok(Self(object));
        }
        Err(webidl::WebIdlError::cannot_convert(
            context,
            "MessageEventSource",
        ))
    }
}

/// Inherited EventInit members precede the lexicographically ordered own
/// members. Conversion finishes before either base or payload state is set.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MessageEventInit")]
pub(super) struct MessageEventInit<'s> {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
    #[webidl(converter = "raw")]
    data: Option<v8::Local<'s, v8::Value>>,
    #[webidl(name = "lastEventId", default = webidl::DomString16(Vec::new()), converter = "raw")]
    last_event_id: webidl::DomString16,
    #[webidl(default = "", converter = "usv_string")]
    origin: String,
    #[webidl(default = webidl::Sequence(Vec::new()), converter = "raw")]
    ports: webidl::Sequence<MessagePortReference<'s>>,
    #[webidl(nullable, converter = "raw")]
    source: Option<MessageEventSource<'s>>,
}

impl MessageEventInit<'_> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.bubbles, self.cancelable, self.composed)
    }
}

pub(super) fn parse_message_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<MessageEventInit<'s>> {
    let parsed = webidl::dictionary_arg(args, 1, webidl::Context::argument("MessageEvent", 2))
        .and_then(|object| match object {
            Some(object) => webidl::parse_dictionary_object(scope, object),
            None => Ok(MessageEventInit {
                bubbles: false,
                cancelable: false,
                composed: false,
                data: None,
                last_event_id: webidl::DomString16(Vec::new()),
                origin: String::new(),
                ports: webidl::Sequence(Vec::new()),
                source: None,
            }),
        });
    match parsed {
        Ok(init) => Some(init),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MessageEvent.initMessageEvent")]
struct InitMessageEventArgs<'s> {
    #[webidl(
        required,
        converter = "raw",
        missing_message = "Failed to execute 'initMessageEvent': 1 argument required."
    )]
    event_type: webidl::DomString16,
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(converter = "raw")]
    data: Option<v8::Local<'s, v8::Value>>,
    #[webidl(default = "", converter = "usv_string")]
    origin: String,
    #[webidl(name = "lastEventId", default = webidl::DomString16(Vec::new()), converter = "raw")]
    last_event_id: webidl::DomString16,
    #[webidl(nullable, converter = "raw")]
    source: Option<MessageEventSource<'s>>,
    #[webidl(default = webidl::Sequence(Vec::new()), converter = "raw")]
    ports: webidl::Sequence<MessagePortReference<'s>>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct MessageEventState<'s> {
    data: v8::Local<'s, v8::Value>,
    origin: v8::Local<'s, v8::String>,
    last_event_id: v8::Local<'s, v8::String>,
    source: v8::Local<'s, v8::Value>,
    ports: v8::Local<'s, v8::Array>,
}

pub(super) fn initialize_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: MessageEventInit<'s>,
) {
    let ports = init
        .ports
        .0
        .iter()
        .map(|port| port.0.into())
        .collect::<Vec<_>>();
    let data = init.data.unwrap_or_else(|| v8::null(scope).into());
    let source = init
        .source
        .map_or_else(|| v8::null(scope).into(), |source| source.0.into());
    initialize_message_event_state(
        scope,
        event,
        data,
        &init.origin,
        &init.last_event_id.0,
        source,
        &ports,
    );
}

fn initialize_message_event_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    data: v8::Local<'s, v8::Value>,
    origin: &str,
    last_event_id: &[u16],
    source: v8::Local<'s, v8::Value>,
    ports: &[v8::Local<'s, v8::Value>],
) {
    let event = event_backing(scope, event);
    // A borrowed initializer still creates the FrozenArray in the event's
    // realm. Array allocation bypasses author-defined indexed setters.
    let context = event
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let ports = v8::Array::new_with_elements(scope, ports);
    let _ = ports.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
    let origin = v8_string(scope, origin).expect("MessageEvent origin");
    let last_event_id =
        v8_string_from_utf16_units(scope, last_event_id).expect("MessageEvent lastEventId");
    MessageEventState::new(data, origin, last_event_id, source, ports)
        .initialize(scope, event)
        .expect("MessageEvent state should initialize");
}

/// Native producers supply already deserialized values. Their dense ports
/// arrays must not run author-defined iterators or dictionary getters.
pub(crate) fn construct_original_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    data: v8::Local<'s, v8::Value>,
    origin: &str,
    last_event_id: &[u16],
    source: v8::Local<'s, v8::Value>,
    ports: v8::Local<'s, v8::Array>,
) -> Option<v8::Local<'s, v8::Object>> {
    let constructor = super::super::exposed_interfaces::ensure_intrinsic_interface_constructor(
        scope,
        "MessageEvent",
    )
    .ok()?;
    let event_type = v8_string(scope, event_type)?;
    let event = constructor.new_instance(scope, &[event_type.into()])?;
    let ports = (0..ports.length())
        .map(|index| ports.get_index(scope, index))
        .collect::<Option<Vec<_>>>()?;
    initialize_message_event_state(scope, event, data, origin, last_event_id, source, &ports);
    Some(event)
}

pub(in crate::context_bootstrap) fn message_event_init_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<InitMessageEventArgs>(scope, &args) else {
        return;
    };
    let event = event_backing(scope, args.this());
    if !reinitialize_event_object(
        scope,
        event,
        &parsed.event_type.0,
        parsed.bubbles,
        parsed.cancelable,
    ) {
        return;
    }
    initialize_message_event(
        scope,
        event,
        MessageEventInit {
            bubbles: parsed.bubbles,
            cancelable: parsed.cancelable,
            composed: false,
            data: parsed.data,
            last_event_id: parsed.last_event_id,
            origin: parsed.origin,
            ports: parsed.ports,
            source: parsed.source,
        },
    );
}
