use super::super::util::{throw_type_error, v8_string};
use super::context_host_ptr_from_global_bridge;
use super::{initialize_simple_event_target, simple_object_event_set_handler};
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;
use crate::native_bridge::throw_dom_exception;
use crate::util::{get_private_value, set_private_value, v8_string_to_u16_string, v8str};
use crate::web_api_interfaces;
use crate::webidl;
use crate::webvtt::cue_text::{CueNodeKind, parse_cue_text};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

mod region;
mod values;
pub(crate) use region::new_vtt_region_for_parsed;
pub(super) use region::vtt_region_constructor_callback;
use values::{
    AlignSetting, DirectionSetting, LineAlignSetting, LineAndPositionSetting, PositionAlignSetting,
};

pub(crate) const TEXT_TRACK_CUE_TRACK_SLOT: &str = "__moliTextTrackCueTrack";
const TEXT_TRACK_CUE_START_TIME_SLOT: &str = "__moliTextTrackCueStartTime";
const TEXT_TRACK_CUE_END_TIME_SLOT: &str = "__moliTextTrackCueEndTime";
const TEXT_TRACK_CUE_ID_SLOT: &str = "__moliTextTrackCueId";
const TEXT_TRACK_CUE_PAUSE_ON_EXIT_SLOT: &str = "__moliTextTrackCuePauseOnExit";
const TEXT_TRACK_CUE_VERTICAL_SLOT: &str = "__moliTextTrackCueVertical";
const TEXT_TRACK_CUE_SNAP_TO_LINES_SLOT: &str = "__moliTextTrackCueSnapToLines";
const TEXT_TRACK_CUE_LINE_SLOT: &str = "__moliTextTrackCueLine";
const TEXT_TRACK_CUE_LINE_ALIGN_SLOT: &str = "__moliTextTrackCueLineAlign";
const TEXT_TRACK_CUE_POSITION_SLOT: &str = "__moliTextTrackCuePosition";
const TEXT_TRACK_CUE_POSITION_ALIGN_SLOT: &str = "__moliTextTrackCuePositionAlign";
const TEXT_TRACK_CUE_REGION_SLOT: &str = "__moliTextTrackCueRegion";
const TEXT_TRACK_CUE_SIZE_SLOT: &str = "__moliTextTrackCueSize";
const TEXT_TRACK_CUE_ALIGN_SLOT: &str = "__moliTextTrackCueAlign";
const TEXT_TRACK_CUE_ONENTER_SLOT: &str = "__moliTextTrackCueOnEnter";
const TEXT_TRACK_CUE_ONEXIT_SLOT: &str = "__moliTextTrackCueOnExit";
const TEXT_TRACK_CUE_TEXT_SLOT: &str = "__moliTextTrackCueText";
const MEDIA_ERROR_CODE_SLOT: &str = "__moliMediaErrorCode";
const MEDIA_ERROR_MESSAGE_SLOT: &str = "__moliMediaErrorMessage";

pub(crate) fn text_track_cue_times<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cue: v8::Local<'s, v8::Object>,
) -> Option<(f64, f64)> {
    let cue = moli_webapi_declare::web_api_object_target(scope, cue)?;
    let start = get_private_value(scope, cue, TEXT_TRACK_CUE_START_TIME_SLOT)
        .and_then(|value| v8::Local::<v8::Number>::try_from(value).ok())?
        .value();
    let end = get_private_value(scope, cue, TEXT_TRACK_CUE_END_TIME_SLOT)
        .and_then(|value| v8::Local::<v8::Number>::try_from(value).ok())?
        .value();
    Some((start, end))
}

pub(crate) fn text_track_cue_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cue: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let cue = moli_webapi_declare::web_api_object_target(scope, cue)?;
    get_private_value(scope, cue, TEXT_TRACK_CUE_TRACK_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(crate) fn text_track_cue_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cue: v8::Local<'s, v8::Object>,
) -> Option<Vec<u16>> {
    let cue = moli_webapi_declare::web_api_object_target(scope, cue)?;
    let value = get_private_value(scope, cue, TEXT_TRACK_CUE_ID_SLOT)?;
    webidl::convert::<webidl::DomString16>(
        scope,
        value,
        webidl::Context::member("TextTrackCue", "id"),
    )
    .ok()
    .map(|value| value.0)
}

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

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "VTTCue.region")]
struct RegionSetterArgs<'s> {
    #[webidl(required, nullable, interface = web_api_interfaces::VTTRegion)]
    region: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "TextTrackCue.endTime")]
struct EndTimeSetterArgs {
    #[webidl(required, converter = "unrestricted_double")]
    end_time: f64,
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

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::VTTCue)]
struct VttCueObjectDeclaration<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TextTrackCue, enumerable, receiver)]
struct TextTrackCueTemplateDeclaration {
    #[webapi(accessor_property, getter = text_track_cue_track_getter)]
    track: (),

    #[webapi(
        accessor_property,
        getter = text_track_cue_id_getter,
        setter = text_track_cue_id_setter
    )]
    id: (),

    #[webapi(
        accessor_property = "startTime",
        getter = text_track_cue_start_time_getter,
        setter = text_track_cue_start_time_setter
    )]
    start_time: (),

    #[webapi(
        accessor_property = "endTime",
        getter = text_track_cue_end_time_getter,
        setter = text_track_cue_end_time_setter
    )]
    end_time: (),

    #[webapi(
        accessor_property = "pauseOnExit",
        getter = text_track_cue_pause_on_exit_getter,
        setter = text_track_cue_pause_on_exit_setter
    )]
    pause_on_exit: (),

    #[webapi(
        accessor_property,
        getter = text_track_cue_onenter_getter,
        setter = text_track_cue_onenter_setter
    )]
    onenter: (),

    #[webapi(
        accessor_property,
        getter = text_track_cue_onexit_getter,
        setter = text_track_cue_onexit_setter
    )]
    onexit: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::VTTCue, enumerable, receiver)]
struct VttCueTemplateDeclaration {
    #[webapi(accessor_property, getter = text_track_cue_slot_getter, setter = text_track_cue_region_setter, data = v8str(scope, TEXT_TRACK_CUE_REGION_SLOT))]
    region: (),
    #[webapi(
        accessor_property,
        getter = text_track_cue_vertical_getter,
        setter = text_track_cue_vertical_setter
    )]
    vertical: (),

    #[webapi(
        accessor_property = "snapToLines",
        getter = text_track_cue_snap_to_lines_getter,
        setter = text_track_cue_snap_to_lines_setter
    )]
    snap_to_lines: (),

    #[webapi(
        accessor_property,
        getter = text_track_cue_line_getter,
        setter = text_track_cue_line_setter
    )]
    line: (),

    #[webapi(accessor_property = "lineAlign", getter = text_track_cue_slot_getter, setter = text_track_cue_line_align_setter, data = v8str(scope, TEXT_TRACK_CUE_LINE_ALIGN_SLOT))]
    line_align: (),

    #[webapi(
        accessor_property,
        getter = text_track_cue_position_getter,
        setter = text_track_cue_position_setter
    )]
    position: (),

    #[webapi(accessor_property = "positionAlign", getter = text_track_cue_slot_getter, setter = text_track_cue_position_align_setter, data = v8str(scope, TEXT_TRACK_CUE_POSITION_ALIGN_SLOT))]
    position_align: (),

    #[webapi(
        accessor_property,
        getter = text_track_cue_size_getter,
        setter = text_track_cue_size_setter
    )]
    size: (),

    #[webapi(
        accessor_property,
        getter = text_track_cue_align_getter,
        setter = text_track_cue_align_setter
    )]
    align: (),

    #[webapi(
        method = "getCueAsHTML",
        length = 0,
        enumerable,
        callback = text_track_cue_get_cue_as_html_callback
    )]
    get_cue_as_html: (),

    #[webapi(
        accessor_property,
        getter = text_track_cue_text_getter,
        setter = text_track_cue_text_setter
    )]
    text: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaError, enumerable)]
struct MediaErrorConstantsDeclaration {
    #[webapi(constant = "MEDIA_ERR_ABORTED", value = 1u32)]
    media_err_aborted: (),
    #[webapi(constant = "MEDIA_ERR_NETWORK", value = 2u32)]
    media_err_network: (),
    #[webapi(constant = "MEDIA_ERR_DECODE", value = 3u32)]
    media_err_decode: (),
    #[webapi(constant = "MEDIA_ERR_SRC_NOT_SUPPORTED", value = 4u32)]
    media_err_src_not_supported: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaError, enumerable)]
struct MediaErrorPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = media_error_code_getter_callback,
        enumerable
    )]
    code: (),
    #[webapi(
        accessor_property,
        getter = media_error_message_getter_callback,
        enumerable
    )]
    message: (),
}

fn media_error_code_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(code) = get_private_value(scope, args.this(), MEDIA_ERROR_CODE_SLOT) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(code);
}

fn media_error_message_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(message) = get_private_value(scope, args.this(), MEDIA_ERROR_MESSAGE_SLOT) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(message);
}

pub(crate) fn new_media_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    code: u32,
    message: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = super::ensure_intrinsic_interface_prototype(scope, "MediaError").ok()?;
    let object = v8::Object::new(scope);
    if object.set_prototype(scope, prototype.into()) != Some(true) {
        return None;
    }
    set_private_value(
        scope,
        object,
        MEDIA_ERROR_CODE_SLOT,
        v8::Integer::new_from_unsigned(scope, code).into(),
    );
    set_private_value(
        scope,
        object,
        MEDIA_ERROR_MESSAGE_SLOT,
        v8_string(scope, message)?.into(),
    );
    Some(object)
}

pub(super) fn text_track_cue_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    throw_type_error(
        scope,
        "Failed to construct 'TextTrackCue': Illegal constructor.",
    );
}

pub(super) fn vtt_cue_constructor_callback<'s>(
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

fn validate_cue_end_time(scope: &mut v8::PinScope<'_, '_>, value: f64) -> bool {
    if value.is_nan() || value == f64::NEG_INFINITY {
        throw_type_error(
            scope,
            "A cue end time must not be NaN or negative Infinity.",
        );
        return false;
    }
    true
}

fn initialize_vtt_cue_state<'s>(
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

/// Parser-created cues use native prototypes and slots without calling author
/// constructors, getters, or setters.
pub(crate) fn new_vtt_cue_for_parsed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: &crate::webvtt::ParsedCue,
    region: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let prototype = super::ensure_intrinsic_interface_prototype(scope, "VTTCue").ok()?;
    let cue = VttCueObjectDeclaration::new(prototype).bind(scope).ok()?;
    let text = v8_string(scope, &parsed.text)?;
    initialize_vtt_cue_state(scope, cue, parsed.start, parsed.end, text);
    if let Some(region) = region {
        set_private_value(scope, cue, TEXT_TRACK_CUE_REGION_SLOT, region.into());
    }
    set_private_value(
        scope,
        cue,
        TEXT_TRACK_CUE_ID_SLOT,
        v8_string(scope, &parsed.id)?.into(),
    );
    match parsed.line {
        crate::webvtt::ParsedCueLine::Auto => {}
        crate::webvtt::ParsedCueLine::Number(value) => {
            set_private_number(scope, cue, TEXT_TRACK_CUE_LINE_SLOT, value);
        }
        crate::webvtt::ParsedCueLine::Percent(value) => {
            set_private_number(scope, cue, TEXT_TRACK_CUE_LINE_SLOT, value);
            set_private_bool(scope, cue, TEXT_TRACK_CUE_SNAP_TO_LINES_SLOT, false);
        }
    }
    for (slot, value) in [
        (TEXT_TRACK_CUE_POSITION_SLOT, parsed.position),
        (TEXT_TRACK_CUE_SIZE_SLOT, parsed.size),
    ] {
        if let Some(value) = value {
            set_private_number(scope, cue, slot, value);
        }
    }
    for (slot, value) in [
        (TEXT_TRACK_CUE_ALIGN_SLOT, parsed.align),
        (TEXT_TRACK_CUE_VERTICAL_SLOT, parsed.vertical),
        (TEXT_TRACK_CUE_LINE_ALIGN_SLOT, Some(parsed.line_align)),
        (
            TEXT_TRACK_CUE_POSITION_ALIGN_SLOT,
            Some(parsed.position_align),
        ),
    ] {
        if let Some(value) = value {
            set_private_string(scope, cue, slot, value);
        }
    }
    Some(cue)
}

fn text_track_cue_slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let slot = v8::Local::<v8::String>::try_from(args.data())
        .expect("cue attribute native callback data")
        .to_rust_string_lossy(scope);
    rv.set(get_private_value(scope, receiver, &slot).expect("cue initialized attribute"));
}

fn text_track_cue_region_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let Some(parsed) = webidl::parse_args::<RegionSetterArgs>(scope, &args) else {
        return;
    };
    let value = parsed
        .region
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into());
    set_private_value(scope, receiver, TEXT_TRACK_CUE_REGION_SLOT, value);
}

fn set_enum_attribute<'s, E: webidl::WebIdlEnum>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
    member: &'static str,
    slot: &'static str,
) {
    let value = match webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::member("VTTCue", member),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if E::parse_token(&value).is_some() {
        set_private_string(scope, receiver, slot, &value);
    }
}

fn text_track_cue_line_align_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    set_enum_attribute::<LineAlignSetting>(
        scope,
        receiver,
        args.get(0),
        "lineAlign",
        TEXT_TRACK_CUE_LINE_ALIGN_SLOT,
    );
}

fn text_track_cue_position_align_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    set_enum_attribute::<PositionAlignSetting>(
        scope,
        receiver,
        args.get(0),
        "positionAlign",
        TEXT_TRACK_CUE_POSITION_ALIGN_SLOT,
    );
}

pub(crate) fn set_text_track_cue_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cue: v8::Local<'s, v8::Object>,
    track: v8::Local<'s, v8::Value>,
) {
    let cue =
        moli_webapi_declare::web_api_object_target(scope, cue).expect("native cue track state");
    set_private_value(scope, cue, TEXT_TRACK_CUE_TRACK_SLOT, track);
}

fn text_track_cue_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("cue receiver validated by WebIDL")
}

fn text_track_cue_track_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    rv.set(
        get_private_value(scope, receiver, TEXT_TRACK_CUE_TRACK_SLOT)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

fn text_track_cue_start_time_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = private_number(scope, receiver, TEXT_TRACK_CUE_START_TIME_SLOT);
    rv.set(v8::Number::new(scope, value).into());
}

fn text_track_cue_start_time_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let Some(value) = finite_double_property(scope, args.get(0), "TextTrackCue", "startTime")
    else {
        return;
    };
    set_private_number(scope, receiver, TEXT_TRACK_CUE_START_TIME_SLOT, value);
    crate::native_bridge::element::resort_text_track_cues_for_cue(scope, receiver);
}

fn text_track_cue_end_time_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = private_number(scope, receiver, TEXT_TRACK_CUE_END_TIME_SLOT);
    rv.set(v8::Number::new(scope, value).into());
}

fn text_track_cue_end_time_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let Some(parsed) = webidl::parse_args::<EndTimeSetterArgs>(scope, &args) else {
        return;
    };
    if !validate_cue_end_time(scope, parsed.end_time) {
        return;
    }
    set_private_number(
        scope,
        receiver,
        TEXT_TRACK_CUE_END_TIME_SLOT,
        parsed.end_time,
    );
    crate::native_bridge::element::resort_text_track_cues_for_cue(scope, receiver);
}

fn text_track_cue_text_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    rv.set(
        get_private_value(scope, receiver, TEXT_TRACK_CUE_TEXT_SLOT)
            .expect("cue initialized string"),
    );
}

fn text_track_cue_text_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = match webidl::convert::<webidl::DomString16>(
        scope,
        args.get(0),
        webidl::Context::member("VTTCue", "text"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if let Some(value) = v8::String::new_from_two_byte(scope, &value, v8::NewStringType::Normal) {
        set_private_value(scope, receiver, TEXT_TRACK_CUE_TEXT_SLOT, value.into());
    }
}

fn text_track_cue_get_cue_as_html_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let Some(context) = receiver.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let text = get_private_value(scope, receiver, TEXT_TRACK_CUE_TEXT_SLOT)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .map(|value| v8_string_to_u16_string(scope, value))
        .unwrap_or_default();
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let document = crate::native_bridge::WindowEnvironmentSettings::for_current_realm(scope)
        .map(|settings| settings.document_handle())
        .unwrap_or_else(|| runtime.document_handle());
    let fragment = runtime
        .dom_host_mut()
        .create_document_fragment_for_document(document);
    append_vtt_cue_fragment(
        scope,
        runtime_ptr,
        runtime,
        document,
        fragment,
        text.as_slice(),
    );
    if let Some(fragment) =
        runtime
            .native_bridge_mut()
            .wrap_handle_for_receiver(scope, runtime_ptr, receiver, fragment)
    {
        rv.set(fragment.into());
    }
}

fn append_vtt_cue_fragment(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    runtime: &mut JsContextHost,
    document: DomHandle,
    fragment: DomHandle,
    source: &[u16],
) {
    let mut handles = Vec::new();
    for node in parse_cue_text(source) {
        let child = match node.kind {
            CueNodeKind::Text(text) => runtime.create_text_node_for_document(
                document,
                moli_dom::native::DomStringValue::from_utf16(&text),
            ),
            CueNodeKind::Timestamp(timestamp) => {
                runtime.create_processing_instruction_for_document(document, "timestamp", timestamp)
            }
            CueNodeKind::Element {
                tag,
                classes,
                annotation,
            } => {
                let child = runtime
                    .dom_host_mut()
                    .create_element_without_attributes_for_document(
                        document,
                        tag.element_name().to_owned(),
                        "http://www.w3.org/1999/xhtml".to_owned(),
                        None,
                    );
                for (name, units) in [
                    (!classes.is_empty()).then_some(("class", classes)),
                    tag.annotation_attribute().map(|name| (name, annotation)),
                ]
                .into_iter()
                .flatten()
                {
                    let value = String::from_utf16_lossy(&units);
                    let _ = runtime.set_attribute_utf16_units_appending_to_current_reaction_queue(
                        scope,
                        runtime_ptr,
                        child,
                        name,
                        &value,
                        units,
                    );
                }
                child
            }
        };
        let parent = node.parent.map(|index| handles[index]).unwrap_or(fragment);
        let _ = runtime.append_child(scope, runtime_ptr, parent, child);
        handles.push(child);
    }
}

fn text_track_cue_id_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    rv.set(
        get_private_value(scope, receiver, TEXT_TRACK_CUE_ID_SLOT).expect("cue initialized string"),
    );
}

fn text_track_cue_id_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = match webidl::convert::<webidl::DomString16>(
        scope,
        args.get(0),
        webidl::Context::member("TextTrackCue", "id"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if let Some(value) = v8::String::new_from_two_byte(scope, &value, v8::NewStringType::Normal) {
        set_private_value(scope, receiver, TEXT_TRACK_CUE_ID_SLOT, value.into());
    }
}

fn text_track_cue_vertical_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = private_string(scope, receiver, TEXT_TRACK_CUE_VERTICAL_SLOT);
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

fn text_track_cue_vertical_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    set_enum_attribute::<DirectionSetting>(
        scope,
        receiver,
        args.get(0),
        "vertical",
        TEXT_TRACK_CUE_VERTICAL_SLOT,
    );
}

fn text_track_cue_snap_to_lines_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = private_bool(scope, receiver, TEXT_TRACK_CUE_SNAP_TO_LINES_SLOT);
    rv.set(v8::Boolean::new(scope, value).into());
}

fn text_track_cue_snap_to_lines_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = match webidl::convert::<webidl::Boolean>(
        scope,
        args.get(0),
        webidl::Context::member("TextTrackCue", "snapToLines"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    set_private_bool(scope, receiver, TEXT_TRACK_CUE_SNAP_TO_LINES_SLOT, value);
}

fn text_track_cue_line_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = get_private_value(scope, receiver, TEXT_TRACK_CUE_LINE_SLOT)
        .unwrap_or_else(|| v8str(scope, "auto").into());
    rv.set(value);
}

fn text_track_cue_line_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = match webidl::convert::<LineAndPositionSetting>(
        scope,
        args.get(0),
        webidl::Context::member("VTTCue", "line"),
    ) {
        Ok(value) => value,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let value = moli_webapi_declare::WebApiValue::to_v8_value(&value, scope)
        .expect("converted cue setting");
    set_private_value(scope, receiver, TEXT_TRACK_CUE_LINE_SLOT, value);
}

fn text_track_cue_position_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = get_private_value(scope, receiver, TEXT_TRACK_CUE_POSITION_SLOT)
        .unwrap_or_else(|| v8str(scope, "auto").into());
    rv.set(value);
}

fn text_track_cue_position_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = match webidl::convert::<LineAndPositionSetting>(
        scope,
        args.get(0),
        webidl::Context::member("VTTCue", "position"),
    ) {
        Ok(value) => value,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if let LineAndPositionSetting::Number(number) = value
        && !(0.0..=100.0).contains(&number)
    {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "VTTCue position must be between 0 and 100.",
        );
        return;
    }
    let value = moli_webapi_declare::WebApiValue::to_v8_value(&value, scope)
        .expect("converted cue setting");
    set_private_value(scope, receiver, TEXT_TRACK_CUE_POSITION_SLOT, value);
}

fn text_track_cue_size_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = private_number(scope, receiver, TEXT_TRACK_CUE_SIZE_SLOT);
    rv.set(v8::Number::new(scope, value).into());
}

fn text_track_cue_size_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let Some(value) = finite_double_property(scope, args.get(0), "TextTrackCue", "size") else {
        return;
    };
    if !(0.0..=100.0).contains(&value) {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "VTTCue size must be between 0 and 100.",
        );
        return;
    }
    set_private_number(scope, receiver, TEXT_TRACK_CUE_SIZE_SLOT, value);
}

fn text_track_cue_align_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = private_string(scope, receiver, TEXT_TRACK_CUE_ALIGN_SLOT);
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8str(scope, "center"))
            .into(),
    );
}

fn text_track_cue_align_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    set_enum_attribute::<AlignSetting>(
        scope,
        receiver,
        args.get(0),
        "align",
        TEXT_TRACK_CUE_ALIGN_SLOT,
    );
}

fn text_track_cue_pause_on_exit_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = private_bool(scope, receiver, TEXT_TRACK_CUE_PAUSE_ON_EXIT_SLOT);
    rv.set(v8::Boolean::new(scope, value).into());
}

fn text_track_cue_pause_on_exit_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    let value = match webidl::convert::<webidl::Boolean>(
        scope,
        args.get(0),
        webidl::Context::member("TextTrackCue", "pauseOnExit"),
    ) {
        Ok(value) => value.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    set_private_bool(scope, receiver, TEXT_TRACK_CUE_PAUSE_ON_EXIT_SLOT, value);
}

fn text_track_cue_onenter_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    event_handler_getter(scope, args, rv, TEXT_TRACK_CUE_ONENTER_SLOT);
}

fn text_track_cue_onenter_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    event_handler_setter(scope, args, "enter", TEXT_TRACK_CUE_ONENTER_SLOT);
}

fn text_track_cue_onexit_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    event_handler_getter(scope, args, rv, TEXT_TRACK_CUE_ONEXIT_SLOT);
}

fn text_track_cue_onexit_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    event_handler_setter(scope, args, "exit", TEXT_TRACK_CUE_ONEXIT_SLOT);
}

fn event_handler_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    slot: &'static str,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    rv.set(get_private_value(scope, receiver, slot).unwrap_or_else(|| v8::null(scope).into()));
}

fn event_handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    event_type: &str,
    slot: &'static str,
) {
    let receiver = text_track_cue_receiver(scope, &args);
    simple_object_event_set_handler(scope, receiver, event_type, slot, args.get(0));
}

fn finite_double_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    owner: &'static str,
    property: &'static str,
) -> Option<f64> {
    match webidl::convert::<webidl::Double>(scope, value, webidl::Context::member(owner, property))
    {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn private_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> f64 {
    get_private_value(scope, object, slot)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0)
}

fn set_private_number(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: f64,
) {
    set_private_value(scope, object, slot, v8::Number::new(scope, value).into());
}

fn private_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> String {
    get_private_value(scope, object, slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

fn set_private_string(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: &str,
) {
    if let Some(value) = v8_string(scope, value) {
        set_private_value(scope, object, slot, value.into());
    }
}

fn private_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> bool {
    get_private_value(scope, object, slot)
        .map(|value| value.boolean_value(scope))
        .unwrap_or(false)
}

fn set_private_bool(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: bool,
) {
    set_private_value(scope, object, slot, v8::Boolean::new(scope, value).into());
}

pub(super) fn media_error_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    throw_type_error(
        scope,
        "Failed to construct 'MediaError': Illegal constructor.",
    );
}

pub(super) fn install_media_cue_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    match interface_name {
        "MediaError" => {
            MediaErrorConstantsDeclaration::initialize_template(scope, template);
            MediaErrorConstantsDeclaration::initialize_prototype_template(scope, prototype);
            MediaErrorPrototypeDeclaration::initialize_prototype_template(scope, prototype);
        }
        "TextTrackCue" => {
            TextTrackCueTemplateDeclaration::initialize_prototype_template(scope, prototype)
        }
        "VTTCue" => VttCueTemplateDeclaration::initialize_prototype_template(scope, prototype),
        "VTTRegion" => region::install(scope, prototype),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{CueNodeKind, parse_cue_text};

    fn vtt_cue_text_content(source: &str) -> String {
        let units = source.encode_utf16().collect::<Vec<_>>();
        let text = parse_cue_text(&units)
            .into_iter()
            .flat_map(|node| match node.kind {
                CueNodeKind::Text(text) => text,
                _ => Vec::new(),
            })
            .collect::<Vec<_>>();
        String::from_utf16(&text).unwrap()
    }

    #[test]
    fn vtt_cue_text_content_decodes_entities_and_strips_tags() {
        assert_eq!(
            vtt_cue_text_content("This &amp; is <b>bold</b> and <00:00:01.000>painted&nbsp;on."),
            "This & is bold and painted\u{00a0}on."
        );
        assert_eq!(
            vtt_cue_text_content("This cue has a less than < character.\nignored"),
            "This cue has a less than "
        );
        assert_eq!(
            vtt_cue_text_content("<h1>Bear</h1>\n<p>look <a href=\"x\">here</a>.</p>"),
            "Bear\nlook here."
        );
    }
}
