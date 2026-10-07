use super::*;
use crate::context_bootstrap::events::{EventInit, modifiers::EventModifierInit, ui::UiEventInit};
use crate::context_bootstrap::file_api::is_branded_data_transfer_object;
use crate::context_bootstrap::{range, touch_runtime};
use crate::context_bootstrap::file_api::is_branded_data_transfer_object;
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

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "CapturedMouseEventInit", snapshot)]
pub(super) struct CapturedMouseEventInit {
    #[webidl(inherit)]
    base: EventInit,
    #[webidl(with = captured_mouse_coordinate)]
    surface_x: i32,
    #[webidl(with = captured_mouse_coordinate)]
    surface_y: i32,
}

// Preserve the existing extension's RangeError policy while the derive controls
// Get order and propagates abrupt completion without reading later members.
fn captured_mouse_coordinate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<i32, webidl::WebIdlError> {
    let context = webidl::Context::member("CapturedMouseEventInit", name);
    let value = match webidl::property_result(scope, object, name, context)? {
        Some(value) if !value.is_undefined() => value,
        _ => return Ok(-1),
    };
    let value = match webidl::convert::<webidl::EnforceRangeLong>(scope, value, context) {
        Ok(value) => i32::from(value),
        Err(error) if error.is_pending_exception() => return Err(error),
        Err(error) => {
            crate::util::throw_range_error(scope, &error.to_string());
            return Err(webidl::WebIdlError::pending_exception(context));
        }
    };
    if value < -1 {
        crate::util::throw_range_error(
            scope,
            "CapturedMouseEvent coordinates must be -1 or non-negative.",
        );
        return Err(webidl::WebIdlError::pending_exception(context));
    }
    Ok(value)
}

pub(super) enum InteractionEventInit<'s> {
    Input(InputEventInit<'s>),
    Touch(TouchEventInit<'s>),
    Captured(CapturedMouseEventInit),
}

impl<'s> InteractionEventInit<'s> {
    pub(super) fn parse(
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
        kind: EventSubclassKind,
    ) -> Option<Self> {
        use super::super::parse_event_dictionary;
        Some(match kind {
            EventSubclassKind::InputEvent => {
                Self::Input(parse_event_dictionary(scope, args, "InputEvent")?)
            }
            EventSubclassKind::TouchEvent => {
                Self::Touch(parse_event_dictionary(scope, args, "TouchEvent")?)
            }
            EventSubclassKind::CapturedMouseEvent => {
                let init: CapturedMouseEventInit =
                    parse_event_dictionary(scope, args, "CapturedMouseEvent")?;
                if (init.surface_x == -1) != (init.surface_y == -1) {
                    crate::util::throw_range_error(
                        scope,
                        "CapturedMouseEvent coordinates must both be -1 or both be non-negative.",
                    );
                    return None;
                }
                Self::Captured(init)
            }
            _ => unreachable!("interaction event kind"),
        })
    }

    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        match self {
            Self::Input(init) => init.ui.event_flags(),
            Self::Touch(init) => init.modifiers.event_flags(),
            Self::Captured(init) => (init.base.bubbles, init.base.cancelable, init.base.composed),
        }
    }

    pub(super) fn initialize(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) -> bool {
        match self {
            Self::Input(init) => init.initialize_snapshot(scope, event).is_some(),
            Self::Captured(init) => init.initialize_snapshot(scope, event).is_some(),
            Self::Touch(init) => {
                init.modifiers.initialize(scope, event);
                touch_runtime::initialize_touch_event_lists(
                    scope,
                    event,
                    &init.touches,
                    &init.target_touches,
                    &init.changed_touches,
                );
                true
            }
        }
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
