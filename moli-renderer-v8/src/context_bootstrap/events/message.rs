use super::{event_backing, reinitialize_event_object};
use crate::context_bootstrap::events::EventInit;
use crate::util::v8_string;
use crate::{web_api_interfaces, webidl};
use webidl::WebIdlDictionarySerialize;

// Interface metadata for the Window | MessagePort | ServiceWorker union.
// The derives handle nullable conversion and retain the original object.
struct MessageEventSource;

impl MessageEventSource {
    const NAME: &'static str = "MessageEventSource";

    fn is_instance<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> bool {
        web_api_interfaces::Window::is_instance(scope, object)
            || web_api_interfaces::MessagePort::is_instance(scope, object)
            || web_api_interfaces::ServiceWorker::is_instance(scope, object)
    }
}

/// Inherited EventInit members precede the lexicographically ordered own
/// members. Conversion finishes before either base or payload state is set.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MessageEventInit", snapshot)]
pub(super) struct MessageEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(converter = "raw", snapshot_default = v8::null(scope).into())]
    data: Option<v8::Local<'s, v8::Value>>,
    #[webidl(name = "lastEventId", default = webidl::DomString16(Vec::new()), converter = "raw")]
    last_event_id: webidl::DomString16,
    #[webidl(default = "", converter = "usv_string")]
    origin: String,
    #[webidl(sequence, interface = crate::web_api_interfaces::MessagePort, default = Vec::new(), snapshot_frozen)]
    ports: Vec<v8::Local<'s, v8::Object>>,
    #[webidl(nullable, interface = MessageEventSource)]
    source: Option<v8::Local<'s, v8::Object>>,
}

impl MessageEventInit<'_> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }
}

pub(super) fn parse_message_event_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<MessageEventInit<'s>> {
    crate::context_bootstrap::events::parse_event_dictionary(scope, args, "MessageEvent")
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
    #[webidl(nullable, interface = MessageEventSource)]
    source: Option<v8::Local<'s, v8::Object>>,
    #[webidl(sequence, interface = crate::web_api_interfaces::MessagePort, default = Vec::new())]
    ports: Vec<v8::Local<'s, v8::Object>>,
}

pub(super) fn initialize_message_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    init: MessageEventInit<'s>,
) {
    let event = event_backing(scope, event);
    // A borrowed initializer still creates the FrozenArray in the event's
    // realm. Array allocation bypasses author-defined indexed setters.
    let context = event
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    // Event construction and legacy reinitialization own the inherited flags.
    // In particular, initMessageEvent preserves the existing composed value.
    init.initialize_own_snapshot(scope, event)
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
        .map(|index| ports.get_index(scope, index)?.try_into().ok())
        .collect::<Option<Vec<_>>>()?;
    initialize_message_event(
        scope,
        event,
        MessageEventInit {
            base: EventInit::default(),
            data: Some(data),
            origin: origin.to_owned(),
            last_event_id: webidl::DomString16(last_event_id.to_vec()),
            source: if source.is_null_or_undefined() {
                None
            } else {
                Some(source.try_into().ok()?)
            },
            ports,
        },
    );
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
            base: EventInit {
                bubbles: parsed.bubbles,
                cancelable: parsed.cancelable,
                composed: false,
            },
            data: parsed.data,
            last_event_id: parsed.last_event_id,
            origin: parsed.origin,
            ports: parsed.ports,
            source: parsed.source,
        },
    );
}
