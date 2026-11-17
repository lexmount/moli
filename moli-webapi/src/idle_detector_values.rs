//! IdleDetector construction values; permission, start and abort ownership stay with the host.
use crate::{event_target::SIMPLE_EVENT_TARGET_LISTENERS_SLOT, interfaces as web_api_interfaces};
use moli_v8_util::throw_type_error;
use moli_webapi_declare::WebApiObject;

pub const IDLE_DETECTOR_STARTED_SLOT: &str = "__lmIdleDetectorStarted";
pub const IDLE_DETECTOR_USER_ACTIVE_SLOT: &str = "__lmIdleDetectorUserActive";
pub const IDLE_DETECTOR_SCREEN_UNLOCKED_SLOT: &str = "__lmIdleDetectorScreenUnlocked";
pub const IDLE_DETECTOR_ABORT_SIGNAL_SLOT: &str = "__lmIdleDetectorAbortSignal";
pub const IDLE_DETECTOR_ABORT_ALGORITHM_SLOT: &str = "__lmIdleDetectorAbortAlgorithm";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::IdleDetector)]
struct IdleDetectorObjectDeclaration {
    #[webapi(slot = IDLE_DETECTOR_STARTED_SLOT)]
    started: bool,

    #[webapi(slot = IDLE_DETECTOR_USER_ACTIVE_SLOT)]
    is_user_active: bool,

    #[webapi(slot = IDLE_DETECTOR_SCREEN_UNLOCKED_SLOT)]
    is_screen_unlocked: bool,

    #[webapi(slot = SIMPLE_EVENT_TARGET_LISTENERS_SLOT, init = "null_object")]
    event_listeners: (),

    #[webapi(slot = IDLE_DETECTOR_ABORT_SIGNAL_SLOT, init = "null")]
    abort_signal: (),

    #[webapi(slot = IDLE_DETECTOR_ABORT_ALGORITHM_SLOT, init = "null")]
    abort_algorithm: (),
}

pub fn idle_detector_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'IdleDetector': Please use the 'new' operator.",
        );
        return;
    }

    let detector = args.this();
    if IdleDetectorObjectDeclaration::new(false, true, true)
        .initialize(scope, detector)
        .is_err()
    {
        rv.set_undefined();
        return;
    }
    rv.set(detector.into());
}
