use super::EventInit;
use moli_webidl as webidl;
use webidl::WebIdlDictionarySerialize;
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
            moli_v8_util::throw_range_error(scope, &error.to_string());
            return Err(webidl::WebIdlError::pending_exception(context));
        }
    };
    if value < -1 {
        moli_v8_util::throw_range_error(
            scope,
            "CapturedMouseEvent coordinates must be -1 or non-negative.",
        );
        return Err(webidl::WebIdlError::pending_exception(context));
    }
    Ok(value)
}

impl CapturedMouseEventInit {
    pub(super) fn parse<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        args: &v8::FunctionCallbackArguments<'s>,
    ) -> Option<Self> {
        let init: Self = super::parse_event_dictionary(scope, args, "CapturedMouseEvent")?;
        if (init.surface_x == -1) != (init.surface_y == -1) {
            moli_v8_util::throw_range_error(
                scope,
                "CapturedMouseEvent coordinates must both be -1 or both be non-negative.",
            );
            return None;
        }
        Some(init)
    }
    pub(super) fn event_flags(&self) -> (bool, bool, bool) {
        (self.base.bubbles, self.base.cancelable, self.base.composed)
    }
    pub(super) fn initialize<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_>,
        event: v8::Local<'s, v8::Object>,
    ) -> bool {
        self.initialize_snapshot(scope, event).is_some()
    }
}
