use super::super::{COMMAND_EVENT_SOURCE_SLOT, TOGGLE_EVENT_SOURCE_SLOT, event_attribute};
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;
use crate::util::{set_private_value, v8_string_from_utf16_units};
use crate::{web_api_interfaces, webidl};
use moli_webapi_declare::{WebApiObject, web_api_object_target};

struct ElementReference<'s>(v8::Local<'s, v8::Object>);

impl<'s> webidl::WebIdlConverter<'s> for ElementReference<'s> {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _options: &Self::Options,
    ) -> Result<Self, webidl::WebIdlError> {
        let object = v8::Local::<v8::Object>::try_from(value)
            .map_err(|_| webidl::WebIdlError::cannot_convert(context, "Element"))?;
        if !web_api_interfaces::Element::is_instance(scope, object) {
            return Err(webidl::WebIdlError::cannot_convert(context, "Element"));
        }
        Ok(Self(object))
    }
}

/// EventInit members precede the subclass members in lexical order. Convert
/// the entire dictionary before creating either base or payload state.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ToggleEventInit")]
pub(super) struct ToggleEventInit<'s> {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
    #[webidl(name = "newState", default = webidl::DomString16(Vec::new()), converter = "raw")]
    new_state: webidl::DomString16,
    #[webidl(name = "oldState", default = webidl::DomString16(Vec::new()), converter = "raw")]
    old_state: webidl::DomString16,
    #[webidl(nullable, converter = "raw")]
    source: Option<ElementReference<'s>>,
}

impl Default for ToggleEventInit<'_> {
    fn default() -> Self {
        Self {
            bubbles: false,
            cancelable: false,
            composed: false,
            new_state: webidl::DomString16(Vec::new()),
            old_state: webidl::DomString16(Vec::new()),
            source: None,
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "CommandEventInit")]
pub(super) struct CommandEventInit<'s> {
    #[webidl(default = false)]
    bubbles: bool,
    #[webidl(default = false)]
    cancelable: bool,
    #[webidl(default = false)]
    composed: bool,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    command: webidl::DomString16,
    #[webidl(nullable, converter = "raw")]
    source: Option<ElementReference<'s>>,
}

impl Default for CommandEventInit<'_> {
    fn default() -> Self {
        Self {
            bubbles: false,
            cancelable: false,
            composed: false,
            command: webidl::DomString16(Vec::new()),
            source: None,
        }
    }
}

pub(super) fn parse_interaction_event_init<'s, T: webidl::WebIdlDictionary<'s> + Default>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    interface: &'static str,
) -> Option<T> {
    let parsed = webidl::dictionary_arg(args, 1, webidl::Context::argument(interface, 2)).and_then(
        |object| match object {
            Some(object) => webidl::parse_dictionary_object(scope, object),
            None => Ok(T::default()),
        },
    );
    match parsed {
        Ok(init) => Some(init),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct ToggleEventState<'s> {
    old_state: v8::Local<'s, v8::String>,
    new_state: v8::Local<'s, v8::String>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct CommandEventState<'s> {
    command: v8::Local<'s, v8::String>,
}

impl<'s> ToggleEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.bubbles, self.cancelable, self.composed)
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        let old_state = v8_string_from_utf16_units(scope, &self.old_state.0)
            .expect("ToggleEvent oldState DOMString");
        let new_state = v8_string_from_utf16_units(scope, &self.new_state.0)
            .expect("ToggleEvent newState DOMString");
        ToggleEventState::new(old_state, new_state)
            .initialize(scope, event)
            .expect("ToggleEvent state should initialize");
        let source = match self.source {
            Some(source) => source.0.into(),
            None => v8::null(scope).into(),
        };
        set_private_value(scope, event, TOGGLE_EVENT_SOURCE_SLOT, source);
    }
}

impl<'s> CommandEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.bubbles, self.cancelable, self.composed)
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        let command = v8_string_from_utf16_units(scope, &self.command.0)
            .expect("CommandEvent command DOMString");
        CommandEventState::new(command)
            .initialize(scope, event)
            .expect("CommandEvent state should initialize");
        let source = match self.source {
            Some(source) => source.0.into(),
            None => v8::null(scope).into(),
        };
        set_private_value(scope, event, COMMAND_EVENT_SOURCE_SLOT, source);
    }
}

fn native_node_identity<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    if !web_api_interfaces::Node::is_instance(scope, object) {
        return None;
    }
    let target = web_api_object_target(scope, object)?;
    let context = target.get_creation_context(scope)?;
    let scope = &mut v8::ContextScope::new(scope, context);
    crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, target).ok()
}

fn retargeted_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    let source = event_attribute(scope, event, "source")?;
    let Some((source_host, source_node)) = native_node_identity(scope, source) else {
        return Some(source);
    };
    let current_target = event_attribute(scope, event, "currentTarget")?;
    let against = native_node_identity(scope, current_target).and_then(|(host, node)| {
        // Child realms can have different hosts over the same DOM arena.
        std::ptr::eq(
            unsafe { &*host }.dom_host(),
            unsafe { &*source_host }.dom_host(),
        )
        .then_some(node)
    });
    let retargeted = unsafe { &*source_host }
        .dom_host()
        .retarget(source_node, against);
    if retargeted == source_node {
        return Some(source);
    }
    let source_object = v8::Local::<v8::Object>::try_from(source).ok()?;
    unsafe { &mut *source_host }
        .native_bridge_mut()
        .wrap_handle_for_receiver(scope, source_host, source_object, retargeted)
        .map(Into::into)
}

pub(in crate::context_bootstrap) fn interaction_event_source_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    // Generated WebIDL bindings validate the receiver. Always retarget the
    // retained original source against native currentTarget at access time.
    let Some(value) = retargeted_source(scope, args.this()) else {
        return;
    };
    let Some(context) = args.this().get_creation_context(scope) else {
        return;
    };
    if let Some(value) =
        crate::context_bootstrap::platform_object_worlds::in_realm(scope, value, context)
    {
        rv.set(value);
    }
}
