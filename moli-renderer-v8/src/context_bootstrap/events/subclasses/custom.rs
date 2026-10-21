use crate::context_bootstrap::events::EventInit;
use crate::webidl::{self, WebIdlDictionarySerialize};

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "CustomEventInit", snapshot)]
pub(super) struct CustomEventInit<'s> {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(converter = "raw", snapshot_default = v8::null(scope).into())]
    detail: Option<v8::Local<'s, v8::Value>>,
}

impl<'s> CustomEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.initialize_snapshot(scope, event)
            .expect("CustomEvent state should initialize");
    }
}
