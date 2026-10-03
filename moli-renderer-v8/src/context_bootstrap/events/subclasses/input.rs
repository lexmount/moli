use super::*;
use crate::context_bootstrap::events::ui::UiEventInit;
use crate::context_bootstrap::file_api::is_branded_data_transfer_object;
use crate::context_bootstrap::range::{live_range_from_static_range, static_range_snapshot};
use crate::util::{get_private_value, set_private_value, v8_string_from_utf16_units};
use crate::{web_api_interfaces, webidl};

const INPUT_EVENT_TARGET_RANGES_SLOT: &str = "__moliInputEventTargetRanges";

/// Partial dictionaries contribute to the same lexicographic member order.
#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "InputEventInit")]
struct InputEventInitMembers<'s> {
    #[webidl(nullable, converter = "raw")]
    data: Option<webidl::DomString16>,
    #[webidl(nullable, interface = web_api_interfaces::DataTransfer, brand_check = is_branded_data_transfer_object)]
    data_transfer: Option<v8::Local<'s, v8::Object>>,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    input_type: webidl::DomString16,
    #[webidl(default = false)]
    is_composing: bool,
    #[webidl(sequence, interface = web_api_interfaces::StaticRange, default = Vec::new())]
    target_ranges: Vec<v8::Local<'s, v8::Object>>,
}

impl Default for InputEventInitMembers<'_> {
    fn default() -> Self {
        Self {
            data: None,
            data_transfer: None,
            input_type: webidl::DomString16(Vec::new()),
            is_composing: false,
            target_ranges: Vec::new(),
        }
    }
}

#[derive(Default)]
pub(super) struct InputEventInit<'s> {
    ui: UiEventInit<'s>,
    members: InputEventInitMembers<'s>,
}

impl<'s> webidl::WebIdlDictionary<'s> for InputEventInit<'s> {
    fn parse_dictionary(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Result<Self, webidl::WebIdlError> {
        Ok(Self {
            ui: webidl::parse_dictionary_object(scope, object)?,
            members: webidl::parse_dictionary_object(scope, object)?,
        })
    }
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct InputEventState<'s> {
    data: v8::Local<'s, v8::Value>,
    data_transfer: v8::Local<'s, v8::Value>,
    input_type: v8::Local<'s, v8::String>,
    is_composing: bool,
}

impl<'s> InputEventInit<'s> {
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        self.ui.event_flags()
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) -> bool {
        let mut ranges = Vec::with_capacity(self.members.target_ranges.len());
        for range in self.members.target_ranges {
            let Some(range) = live_range_from_static_range(scope, range) else {
                return false;
            };
            ranges.push(range.into());
        }
        let ranges = v8::Array::new_with_elements(scope, &ranges);
        set_private_value(scope, event, INPUT_EVENT_TARGET_RANGES_SLOT, ranges.into());
        self.ui.initialize(scope, event);
        let data = match self.members.data {
            Some(data) => v8_string_from_utf16_units(scope, &data.0)
                .expect("InputEvent data")
                .into(),
            None => v8::null(scope).into(),
        };
        let data_transfer = self
            .members
            .data_transfer
            .map(|transfer| transfer.into())
            .unwrap_or_else(|| v8::null(scope).into());
        let input_type = v8_string_from_utf16_units(scope, &self.members.input_type.0)
            .expect("InputEvent inputType");
        InputEventState::new(data, data_transfer, input_type, self.members.is_composing)
            .initialize(scope, event)
            .expect("InputEvent state should initialize");
        true
    }
}

pub(in crate::context_bootstrap) fn input_event_get_target_ranges_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    // The generated binding validates the InputEvent receiver. The stored list
    // is private and retains its live native ranges in V8's traced object graph.
    let event = event_backing(scope, args.this());
    let mut snapshots = Vec::new();
    if let Some(ranges) = get_private_value(scope, event, INPUT_EVENT_TARGET_RANGES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    {
        snapshots.reserve(ranges.length() as usize);
        for index in 0..ranges.length() {
            let Some(range) = ranges
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            else {
                return;
            };
            let Some(snapshot) = static_range_snapshot(scope, range) else {
                return;
            };
            snapshots.push(snapshot.into());
        }
    }
    rv.set(v8::Array::new_with_elements(scope, &snapshots).into());
}
