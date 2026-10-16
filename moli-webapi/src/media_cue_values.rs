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

pub const TEXT_TRACK_CUE_LINE_ALIGN_SLOT: &str = "__moliTextTrackCueLineAlign";
pub const TEXT_TRACK_CUE_POSITION_ALIGN_SLOT: &str = "__moliTextTrackCuePositionAlign";
pub const TEXT_TRACK_CUE_REGION_SLOT: &str = "__moliTextTrackCueRegion";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VTTCue")]
struct VttCueConstructorArgs {
    #[webidl(required, name = "startTime", converter = "double")]
    start_time: f64,
    #[webidl(required, name = "endTime", converter = "unrestricted_double")]
    end_time: f64,
    #[webidl(required, converter = "raw")]
    text: webidl::DomString16,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::VTTCue)]
struct VttCuePrivateStateDeclaration<'s> {
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

    #[webapi(slot = TEXT_TRACK_CUE_LINE_ALIGN_SLOT, constructor_default = "start")]
    _line_align: &'static str,

    #[webapi(slot = TEXT_TRACK_CUE_POSITION_SLOT, constructor_default = "auto")]
    _position: &'static str,

    #[webapi(slot = TEXT_TRACK_CUE_POSITION_ALIGN_SLOT, constructor_default = "auto")]
    _position_align: &'static str,

    #[webapi(slot = TEXT_TRACK_CUE_REGION_SLOT, init = "null")]
    _region: (),

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
    text: v8::Local<'s, v8::String>,
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

    if !validate_cue_end_time(scope, parsed.end_time) {
        return;
    }
    let Some(text) =
        v8::String::new_from_two_byte(scope, &parsed.text.0, v8::NewStringType::Normal)
    else {
        return;
    };
    initialize_vtt_cue_state(scope, args.this(), parsed.start_time, parsed.end_time, text);
    rv.set(args.this().into());
}

pub const TEXT_TRACK_LIST_ITEMS_SLOT: &str = "__moliTextTrackListItems";
pub const TEXT_TRACK_LIST_ONADDTRACK_SLOT: &str = "__moliTextTrackListOnAddTrack";
pub const TEXT_TRACK_LIST_ONREMOVETRACK_SLOT: &str = "__moliTextTrackListOnRemoveTrack";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TextTrackCueList)]
struct TextTrackCueListObjectDeclaration<'scope> {
    #[webapi(slot = TEXT_TRACK_LIST_ITEMS_SLOT)]
    items: Vec<v8::Local<'scope, v8::Value>>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::TextTrackList)]
struct TextTrackListObjectDeclaration<'scope> {
    #[webapi(slot = TEXT_TRACK_LIST_ITEMS_SLOT)]
    items: Vec<v8::Local<'scope, v8::Value>>,

    #[webapi(slot = TEXT_TRACK_LIST_ONADDTRACK_SLOT, init = "null")]
    _onaddtrack: (),

    #[webapi(slot = TEXT_TRACK_LIST_ONREMOVETRACK_SLOT, init = "null")]
    _onremovetrack: (),
}

pub fn new_text_track_cue_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    TextTrackCueListObjectDeclaration::new(Vec::new())
        .bind(scope)
        .ok()
}

pub fn new_text_track_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Object>> {
    TextTrackListObjectDeclaration::new(Vec::new())
        .bind(scope)
        .ok()
}

pub fn validate_cue_end_time(scope: &mut v8::PinScope<'_, '_>, value: f64) -> bool {
    if value.is_nan() || value == f64::NEG_INFINITY {
        throw_type_error(
            scope,
            "A cue end time must not be NaN or negative Infinity.",
        );
        return false;
    }
    true
}

pub fn initialize_vtt_cue_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cue: v8::Local<'s, v8::Object>,
    start: f64,
    end: f64,
    text: v8::Local<'s, v8::String>,
) {
    initialize_simple_event_target(scope, cue);
    VttCuePrivateStateDeclaration::new(start, end, text)
        .initialize(scope, cue)
        .expect("VTTCue private state declaration should initialize");
}
