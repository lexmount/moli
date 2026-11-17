//! Timed text construction and value state; native tracks and cue DOM remain host-owned.
use crate::{event_target::initialize_simple_event_target, interfaces as web_api_interfaces};
use moli_v8_util::throw_type_error;
use moli_webapi_declare::WebApiObject;
use moli_webidl as webidl;

pub const TEXT_TRACK_CUE_TRACK_SLOT: &str = "__moliTextTrackCueTrack";

pub const TEXT_TRACK_CUE_START_TIME_SLOT: &str = "__moliTextTrackCueStartTime";

pub const TEXT_TRACK_CUE_END_TIME_SLOT: &str = "__moliTextTrackCueEndTime";

pub const TEXT_TRACK_CUE_ID_SLOT: &str = "__moliTextTrackCueId";

pub const TEXT_TRACK_CUE_PAUSE_ON_EXIT_SLOT: &str = "__moliTextTrackCuePauseOnExit";

pub const TEXT_TRACK_CUE_VERTICAL_SLOT: &str = "__moliTextTrackCueVertical";

pub const TEXT_TRACK_CUE_SNAP_TO_LINES_SLOT: &str = "__moliTextTrackCueSnapToLines";

pub const TEXT_TRACK_CUE_LINE_SLOT: &str = "__moliTextTrackCueLine";

pub const TEXT_TRACK_CUE_POSITION_SLOT: &str = "__moliTextTrackCuePosition";

pub const TEXT_TRACK_CUE_SIZE_SLOT: &str = "__moliTextTrackCueSize";

pub const TEXT_TRACK_CUE_ALIGN_SLOT: &str = "__moliTextTrackCueAlign";

pub const TEXT_TRACK_CUE_ONENTER_SLOT: &str = "__moliTextTrackCueOnEnter";

pub const TEXT_TRACK_CUE_ONEXIT_SLOT: &str = "__moliTextTrackCueOnExit";

pub const TEXT_TRACK_CUE_TEXT_SLOT: &str = "__moliTextTrackCueText";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VTTCue")]
struct VttCueConstructorArgs {
    #[webidl(required, name = "startTime", converter = "double")]
    start_time: f64,
    #[webidl(required, name = "endTime", converter = "double")]
    end_time: f64,
    #[webidl(required)]
    text: String,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::VTTCue)]
struct VttCuePrivateStateDeclaration {
    #[webapi(slot = TEXT_TRACK_CUE_START_TIME_SLOT)]
    _start_time: f64,

    #[webapi(slot = TEXT_TRACK_CUE_END_TIME_SLOT)]
    _end_time: f64,

    #[webapi(slot = TEXT_TRACK_CUE_ID_SLOT, constructor_default = "")]
    _id: &'static str,

    #[webapi(slot = TEXT_TRACK_CUE_PAUSE_ON_EXIT_SLOT, constructor_default = false)]
    _pause_on_exit: bool,

    #[webapi(slot = TEXT_TRACK_CUE_VERTICAL_SLOT, constructor_default = "")]
    _vertical: &'static str,

    #[webapi(slot = TEXT_TRACK_CUE_SNAP_TO_LINES_SLOT, constructor_default = true)]
    _snap_to_lines: bool,

    #[webapi(slot = TEXT_TRACK_CUE_LINE_SLOT, constructor_default = "auto")]
    _line: &'static str,

    #[webapi(slot = TEXT_TRACK_CUE_POSITION_SLOT, constructor_default = "auto")]
    _position: &'static str,

    #[webapi(slot = TEXT_TRACK_CUE_SIZE_SLOT, constructor_default = 100.0)]
    _size: f64,

    #[webapi(slot = TEXT_TRACK_CUE_ALIGN_SLOT, constructor_default = "center")]
    _align: &'static str,

    #[webapi(slot = TEXT_TRACK_CUE_ONENTER_SLOT, init = "null")]
    _onenter_slot: (),

    #[webapi(slot = TEXT_TRACK_CUE_ONEXIT_SLOT, init = "null")]
    _onexit_slot: (),

    #[webapi(slot = TEXT_TRACK_CUE_TRACK_SLOT, init = "null")]
    _track_slot: (),

    #[webapi(slot = TEXT_TRACK_CUE_TEXT_SLOT)]
    text: String,
}

pub fn vtt_cue_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'VTTCue': Please use the 'new' operator.",
        );
        return;
    }

    let Some(parsed) = webidl::parse_args::<VttCueConstructorArgs>(scope, &args) else {
        return;
    };

    initialize_simple_event_target(scope, args.this());
    VttCuePrivateStateDeclaration::new(parsed.start_time, parsed.end_time, parsed.text)
        .initialize(scope, args.this())
        .expect("VTTCue private state declaration should initialize");

    rv.set(args.this().into());
}
