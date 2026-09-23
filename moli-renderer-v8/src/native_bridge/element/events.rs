use crate::{context_bootstrap::mark_event_trusted, util::v8_string};

mod constructors;
mod dispatch;

pub(crate) use self::constructors::construct_focus_event;
pub(crate) use self::constructors::{
    TextEditInputType, TouchEventPoint, construct_command_event, construct_drag_event,
    construct_input_event, construct_interest_event, construct_keyboard_event,
    construct_mouse_event_with_detail_and_modifiers, construct_mouse_event_with_modifiers,
    construct_mouse_event_with_related_target_and_modifiers, construct_pointer_event,
    construct_pointer_event_with_modifiers, construct_pointer_event_with_related_target,
    construct_pointer_event_with_related_target_and_modifiers, construct_simple_event,
    construct_submit_event, construct_toggle_event, construct_touch_event,
    construct_touch_event_with_points, construct_wheel_event,
};
pub(super) use self::constructors::{
    construct_click_event, construct_click_event_with_detail_and_modifiers,
};
pub(crate) use self::dispatch::{NodePublicEventDispatchOutcome, dispatch_public_event};

fn event_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
) -> Option<v8::Local<'s, v8::Function>> {
    let global = scope.get_current_context().global(scope);
    let ctor = global.get(scope, v8_string(scope, name)?.into())?;
    v8::Local::<v8::Function>::try_from(ctor).ok()
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
