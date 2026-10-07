use crate::context_bootstrap::{
    events::{modifiers::EventModifierInitMembers, ui::UiEventInit},
    touch_runtime,
};
use crate::{web_api_interfaces, webidl};

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "TouchEventInit")]
struct TouchEventInitMembers<'s> {
    #[webidl(sequence, interface = web_api_interfaces::Touch, default = Vec::new())]
    touches: Vec<v8::Local<'s, v8::Object>>,
    #[webidl(sequence, interface = web_api_interfaces::Touch, default = Vec::new())]
    target_touches: Vec<v8::Local<'s, v8::Object>>,
    #[webidl(sequence, interface = web_api_interfaces::Touch, default = Vec::new())]
    changed_touches: Vec<v8::Local<'s, v8::Object>>,
}

#[derive(Default)]
pub(super) struct TouchEventInit<'s> {
    ui: UiEventInit<'s>,
    modifiers: EventModifierInitMembers,
    members: TouchEventInitMembers<'s>,
}

impl<'s> webidl::WebIdlDictionary<'s> for TouchEventInit<'s> {
    fn parse_dictionary(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Result<Self, webidl::WebIdlError> {
        // Each inheritance level reads its members once, in lexical order.
        // Finish all conversions before initializing event state.
        Ok(Self {
            ui: webidl::parse_dictionary_object(scope, object)?,
            modifiers: webidl::parse_dictionary_object(scope, object)?,
            members: webidl::parse_dictionary_object(scope, object)?,
        })
    }
}

impl<'s> TouchEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.ui.event_flags()
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.ui.initialize(scope, event);
        self.modifiers.initialize(scope, event);
        touch_runtime::initialize_touch_event_lists(
            scope,
            event,
            &self.members.touches,
            &self.members.target_touches,
            &self.members.changed_touches,
        );
    }
}

pub(crate) fn construct_native_touch_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    touches: Vec<v8::Local<'s, v8::Object>>,
    target_touches: Vec<v8::Local<'s, v8::Object>>,
    changed_touches: Vec<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Object>> {
    use super::super::{
        EVENT_SUBCLASS_KIND_SLOT, define_event_property, initialize_event_object,
        mark_event_trusted, new_event_state, new_event_wrapper,
    };
    let state = new_event_state(scope);
    initialize_event_object(scope, state, event_type, true, true);
    let composed = v8::Boolean::new(scope, true);
    define_event_property(scope, state, "composed", composed.into());
    TouchEventInit {
        members: TouchEventInitMembers {
            touches,
            target_touches,
            changed_touches,
        },
        ..Default::default()
    }
    .initialize(scope, state);
    web_api_interfaces::initialize(scope, state, web_api_interfaces::TouchEvent::NAME).ok()?;
    let kind = v8::Integer::new(scope, super::EventSubclassKind::TouchEvent as i32);
    crate::util::set_private_value(scope, state, EVENT_SUBCLASS_KIND_SLOT, kind.into());
    let event = new_event_wrapper(scope, state)?;
    mark_event_trusted(scope, event);
    Some(event)
}
