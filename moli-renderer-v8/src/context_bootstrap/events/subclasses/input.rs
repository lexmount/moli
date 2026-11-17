use super::*;
use crate::context_bootstrap::events::ui::UiEventInit;
use crate::context_bootstrap::file_api::is_branded_data_transfer_object;
use crate::context_bootstrap::range;
use crate::util::get_private_value;
use crate::{web_api_interfaces, webidl};
use webidl::WebIdlDictionarySerialize;

const INPUT_EVENT_TARGET_RANGES_SLOT: &str = "__moliInputEventTargetRanges";

pub(crate) fn clear_input_event_target_ranges<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
) {
    let event = event_backing(scope, event);
    if web_api_interfaces::InputEvent::is_instance(scope, event) {
        crate::util::set_private_value(
            scope,
            event,
            INPUT_EVENT_TARGET_RANGES_SLOT,
            v8::undefined(scope).into(),
        );
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "InputEventInit", snapshot)]
pub(super) struct InputEventInit<'s> {
    #[webidl(inherit)]
    ui: UiEventInit<'s>,
    #[webidl(nullable, converter = "raw")]
    data: Option<webidl::DomString16>,
    #[webidl(nullable, interface = web_api_interfaces::DataTransfer, brand_check = is_branded_data_transfer_object)]
    data_transfer: Option<v8::Local<'s, v8::Object>>,
    #[webidl(default = webidl::DomString16(Vec::new()), converter = "raw")]
    input_type: webidl::DomString16,
    #[webidl(default = false)]
    is_composing: bool,
    #[webidl(sequence, interface = web_api_interfaces::StaticRange, default = Vec::new())]
    #[webidl(snapshot_slot = INPUT_EVENT_TARGET_RANGES_SLOT, snapshot_state_with = live_range_value)]
    target_ranges: Vec<v8::Local<'s, v8::Object>>,
}

fn live_range_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: &v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    range::live_range_from_static_range(scope, *range).map(Into::into)
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
        self.initialize_snapshot(scope, event).is_some()
    }
}

pub(in crate::context_bootstrap) fn input_event_get_target_ranges_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
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
            let Some(snapshot) = range::static_range_snapshot(scope, range) else {
                return;
            };
            snapshots.push(snapshot.into());
        }
    }
    rv.set(v8::Array::new_with_elements(scope, &snapshots).into());
}
