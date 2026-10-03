use crate::{
    context_bootstrap::{
        exposed_interfaces::ensure_intrinsic_interface_constructor, mark_event_trusted,
    },
    util::v8_string,
};

mod constructors;
mod dispatch;

pub(super) use self::constructors::construct_click_event;
pub(crate) use self::constructors::construct_focus_event;
pub(crate) use self::constructors::{
    TextEditInputType, TouchEventPoint, construct_activation_pointer_event,
    construct_clipboard_event, construct_command_event, construct_drag_event_with_related_target,
    construct_form_data_event, construct_input_event, construct_interest_event,
    construct_keyboard_event, construct_mouse_event_with_detail_and_modifiers,
    construct_mouse_event_with_modifiers, construct_mouse_event_with_related_target_and_modifiers,
    construct_pointer_event, construct_pointer_event_with_modifiers,
    construct_pointer_event_with_related_target,
    construct_pointer_event_with_related_target_and_modifiers, construct_simple_event,
    construct_simple_event_for_target, construct_submit_event, construct_toggle_event,
    construct_touch_event, construct_touch_event_with_points, construct_wheel_event,
};
pub(crate) use self::dispatch::{
    NodePublicEventDispatchOutcome, dispatch_beforeinput, dispatch_public_event,
};

fn prepare_native_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: v8::Local<'s, v8::Object>,
) -> Option<()> {
    // Missing native dictionary members use IDL defaults, never author getters.
    init.set_prototype(scope, v8::null(scope).into())?
        .then_some(())
}

pub(in crate::native_bridge::element) fn construct_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    ctor_name: &str,
    event_type: &str,
    init: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    prepare_native_init(scope, init)?;
    let ctor = ensure_intrinsic_interface_constructor(scope, ctor_name).ok()?;
    let event_type = v8_string(scope, event_type)?;
    let event = ctor.new_instance(scope, &[event_type.into(), init.into()])?;
    mark_event_trusted(scope, event);
    Some(event)
}
