use crate::webidl;
use moli_webapi_declare::WebApiObject;
use webidl::WebIdlDictionarySerialize;

fn nullable_window_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Value> {
    window
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into())
}

/// UIEventInit members, converted after EventInit in dictionary order.
#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "UIEventInit", snapshot)]
pub(super) struct UiEventInit<'s> {
    #[webidl(inherit)]
    base: super::EventInit,
    #[webidl(default = 0)]
    detail: i32,
    #[webidl(nullable, interface = crate::web_api_interfaces::InputDeviceCapabilities)]
    source_capabilities: Option<v8::Local<'s, v8::Object>>,
    #[webidl(nullable, interface = crate::web_api_interfaces::Window, brand_check = crate::context_bootstrap::is_window_receiver)]
    view: Option<v8::Local<'s, v8::Object>>,
}

impl<'s> UiEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }

    pub(super) fn initialize(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) {
        self.initialize_snapshot(scope, event)
            .expect("UIEvent state should initialize");
    }
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct LegacyUiEventState<'s> {
    view: v8::Local<'s, v8::Value>,
    detail: i32,
}

pub(in crate::context_bootstrap) fn initialize_legacy_ui_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    view: Option<v8::Local<'s, v8::Object>>,
    detail: i32,
) {
    let view = nullable_window_value(scope, view);
    LegacyUiEventState::new(view, detail)
        .initialize(scope, event)
        .expect("legacy UIEvent state should initialize");
}
