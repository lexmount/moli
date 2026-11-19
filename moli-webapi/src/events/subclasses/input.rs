use super::super::ui::UiEventInit;
use super::*;
use moli_webidl as webidl;
use webidl::WebIdlDictionarySerialize;

/// InputEventInit members through isComposing, in WebIDL conversion order.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "InputEventInit", snapshot)]
pub struct InputEventValues<'s> {
    #[webidl(inherit)]
    ui: UiEventInit<'s>,
    #[webidl(nullable, converter = "raw")]
    data: Option<webidl::DomString16>,
    #[webidl(nullable, interface = web_api_interfaces::DataTransfer)]
    data_transfer: Option<v8::Local<'s, v8::Object>>,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    input_type: webidl::DomString16,
    #[webidl(default = false)]
    is_composing: bool,
}

impl<'s> InputEventValues<'s> {
    pub fn event_flags(&self) -> (bool, bool, bool) {
        self.ui.event_flags()
    }

    pub fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) -> bool {
        self.initialize_snapshot(scope, event).is_some()
    }
}

pub fn construct_native_input_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event_type: &str,
    input_type: &str,
    data: Option<&str>,
    data_transfer: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let base = EventInit {
        bubbles: true,
        cancelable: event_type == "beforeinput",
        composed: true,
    };
    let init = InputEventValues {
        ui: UiEventInit::from_native(base, 0, None),
        data: data.map(|value| webidl::DomString16(value.encode_utf16().collect())),
        data_transfer: nullable_native_event_interface(
            scope,
            data_transfer.map(Into::into),
            "InputEventInit",
            "dataTransfer",
            webidl::InterfaceOptions {
                name: "DataTransfer",
                brand_check: web_api_interfaces::DataTransfer::is_instance,
            },
        )?,
        input_type: webidl::DomString16(input_type.encode_utf16().collect()),
        is_composing: false,
    };
    construct_trusted_subclass(
        scope,
        EventSubclassKind::InputEvent,
        event_type,
        SubclassInit::Input(init),
    )
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "InputEventInit")]
pub struct InputEventInit<'s> {
    #[webidl(inherit)]
    pub values: InputEventValues<'s>,
    #[webidl(sequence, interface = web_api_interfaces::StaticRange, default = Vec::new())]
    pub target_ranges: Vec<v8::Local<'s, v8::Object>>,
}
