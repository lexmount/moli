pub(in crate::context_bootstrap) use moli_webapi::events::device::{
    device_motion_event_constructor, device_orientation_event_constructor,
};

mod permission;
pub(crate) use permission::DevicePermissionTask;

pub(in crate::context_bootstrap) fn install_device_event_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    moli_webapi::events::device::install_device_event_template_bindings(scope, template, name);
    match name {
        "DeviceOrientationEvent" => permission::install_orientation(scope, template),
        "DeviceMotionEvent" => permission::install_motion(scope, template),
        _ => {}
    }
}
