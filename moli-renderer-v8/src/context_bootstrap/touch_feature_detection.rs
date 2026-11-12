use crate::util::{get_private_value, set_private_value};

const WINDOW_TOUCH_FEATURE_DETECTION_SLOT: &str = "__moliTouchEventFeatureDetection";

pub(crate) const TOUCH_EVENT_HANDLER_PROPERTIES: &[&str] =
    &["ontouchstart", "ontouchend", "ontouchmove", "ontouchcancel"];

/// Each realm receives its LocalWindow's frozen capability snapshot, not the
/// live CDP override. Changing emulation must not mutate author properties.
pub(crate) fn initialize(scope: &mut v8::PinScope<'_, '_>, enabled: bool) {
    let global = scope.get_current_context().global(scope);
    set_private_value(
        scope,
        global,
        WINDOW_TOUCH_FEATURE_DETECTION_SLOT,
        v8::Boolean::new(scope, enabled).into(),
    );
}

pub(crate) fn enabled(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, WINDOW_TOUCH_FEATURE_DETECTION_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}
