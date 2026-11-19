use crate::{context_bootstrap::mark_event_trusted, util::v8_string};

mod constructors;
mod dispatch;

pub(crate) use self::constructors::construct_focus_event;
#[cfg(any(test, feature = "devtools"))]
pub(crate) use self::constructors::construct_keyboard_event;
pub(super) use self::constructors::{ClickEventSource, construct_click_event};
pub(crate) use self::constructors::{
    InputEventCoordinates, TextEditInputType, construct_command_event, construct_form_data_event,
    construct_input_event, construct_interest_event, construct_simple_event,
    construct_simple_event_for_target, construct_submit_event, construct_toggle_event,
};
#[cfg(any(feature = "devtools", test))]
pub(crate) use self::constructors::{TouchEventPoint, construct_touch_event_with_points};
#[cfg(feature = "devtools")]
pub(crate) use self::constructors::{
    construct_drag_event_with_related_target, construct_drop_input_event,
    construct_mouse_event_for_target, construct_mouse_event_with_detail_for_target,
    construct_mouse_event_with_related_target_for_target, construct_pointer_event_with_modifiers,
    construct_pointer_event_with_related_target_and_modifiers, construct_wheel_event_for_target,
};
#[cfg(feature = "devtools")]
pub(crate) use self::constructors::{
    construct_pointer_event, construct_pointer_event_with_related_target,
    construct_text_input_event_for_target,
};
pub(crate) use self::dispatch::{
    NodePublicEventDispatchOutcome, dispatch_beforeinput, dispatch_public_event,
};

fn event_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Option<v8::Local<'s, v8::Function>> {
    // Browser-generated events must not consult author-replaceable Window properties.
    crate::context_bootstrap::exposed_interfaces::ensure_intrinsic_interface_constructor(
        scope, name,
    )
    .ok()
}

pub(in crate::native_bridge::element) fn construct_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    ctor_name: &str,
    event_type: &str,
    init: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let ctor = event_constructor(scope, ctor_name)?;
    let event_type = v8_string(scope, event_type)?;
    let event = ctor.new_instance(scope, &[event_type.into(), init.into()])?;
    mark_event_trusted(scope, event);
    Some(event)
}
