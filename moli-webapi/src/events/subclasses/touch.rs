use crate::events::modifiers::EventModifierInit;
use crate::interfaces as web_api_interfaces;
use moli_webidl as webidl;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "TouchEventInit")]
pub(super) struct TouchEventInit<'s> {
    #[webidl(inherit)]
    modifiers: EventModifierInit<'s>,
    #[webidl(sequence, interface = web_api_interfaces::Touch, default = Vec::new())]
    changed_touches: Vec<v8::Local<'s, v8::Object>>,
    #[webidl(sequence, interface = web_api_interfaces::Touch, default = Vec::new())]
    target_touches: Vec<v8::Local<'s, v8::Object>>,
    #[webidl(sequence, interface = web_api_interfaces::Touch, default = Vec::new())]
    touches: Vec<v8::Local<'s, v8::Object>>,
}

impl<'s> TouchEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.modifiers.event_flags()
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.modifiers.initialize(scope, event);
        crate::touch::initialize_touch_event_lists(
            scope,
            event,
            &self.touches,
            &self.target_touches,
            &self.changed_touches,
        );
    }
}
