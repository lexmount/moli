use super::super::util::{throw_type_error, v8_string};
use super::context_host_ptr_from_global_bridge;
use super::{initialize_simple_event_target, simple_object_event_set_handler};
use crate::document_runtime::DomHandle;
use crate::native_bridge::PageScriptHost;
use crate::native_bridge::throw_dom_exception;
use crate::util::{get_private_value, set_private_value, v8str};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

mod cue_text;
use cue_text::{CueNodeKind, parse_cue_text};

pub(crate) const TEXT_TRACK_CUE_TRACK_SLOT: &str = "__moliTextTrackCueTrack";
const TEXT_TRACK_CUE_START_TIME_SLOT: &str = "__moliTextTrackCueStartTime";
const TEXT_TRACK_CUE_END_TIME_SLOT: &str = "__moliTextTrackCueEndTime";
const TEXT_TRACK_CUE_ID_SLOT: &str = "__moliTextTrackCueId";
const TEXT_TRACK_CUE_PAUSE_ON_EXIT_SLOT: &str = "__moliTextTrackCuePauseOnExit";
const TEXT_TRACK_CUE_VERTICAL_SLOT: &str = "__moliTextTrackCueVertical";
const TEXT_TRACK_CUE_SNAP_TO_LINES_SLOT: &str = "__moliTextTrackCueSnapToLines";
const TEXT_TRACK_CUE_LINE_SLOT: &str = "__moliTextTrackCueLine";
const TEXT_TRACK_CUE_POSITION_SLOT: &str = "__moliTextTrackCuePosition";
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
) -> Option<String> {
    let cue = moli_webapi_declare::web_api_object_target(scope, cue)?;
    get_private_value(scope, cue, TEXT_TRACK_CUE_ID_SLOT)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .map(|value| value.to_rust_string_lossy(scope))
}

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

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::TextTrackCue, enumerable)]
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
#[webapi(interface = web_api_interfaces::VTTCue, enumerable)]
struct VttCueTemplateDeclaration {
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

    #[webapi(
        accessor_property,
        getter = text_track_cue_position_getter,
        setter = text_track_cue_position_setter
    )]
    position: (),

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

    initialize_simple_event_target(scope, args.this());
    VttCuePrivateStateDeclaration::new(parsed.start_time, parsed.end_time, parsed.text)
        .initialize(scope, args.this())
        .expect("VTTCue private state declaration should initialize");

    rv.set(args.this().into());
}

pub(crate) fn set_text_track_cue_track<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    cue: v8::Local<'s, v8::Object>,
    track: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, cue, TEXT_TRACK_CUE_TRACK_SLOT, track);
}

fn text_track_cue_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Object>> {
    let receiver = args.this();
    if web_api_interfaces::TextTrackCue::is_instance(scope, receiver) {
        return Some(receiver);
    }
    throw_type_error(scope, "Illegal invocation");
    None
}

fn text_track_cue_track_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
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
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let value = private_number(scope, receiver, TEXT_TRACK_CUE_START_TIME_SLOT);
    rv.set(v8::Number::new(scope, value).into());
}

fn text_track_cue_start_time_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
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
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let value = private_number(scope, receiver, TEXT_TRACK_CUE_END_TIME_SLOT);
    rv.set(v8::Number::new(scope, value).into());
}

fn text_track_cue_end_time_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let Some(value) = finite_double_property(scope, args.get(0), "TextTrackCue", "endTime") else {
        return;
    };
    set_private_number(scope, receiver, TEXT_TRACK_CUE_END_TIME_SLOT, value);
    crate::native_bridge::element::resort_text_track_cues_for_cue(scope, receiver);
}

fn text_track_cue_text_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let value = private_string(scope, receiver, TEXT_TRACK_CUE_TEXT_SLOT);
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

fn text_track_cue_text_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let value = match webidl::convert::<webidl::DomString>(
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
    set_private_string(scope, receiver, TEXT_TRACK_CUE_TEXT_SLOT, &value);
}

fn text_track_cue_get_cue_as_html_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let text = get_private_value(scope, receiver, TEXT_TRACK_CUE_TEXT_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let fragment = runtime.create_document_fragment();
    append_vtt_cue_fragment(scope, runtime_ptr, runtime, fragment, &text);
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
    runtime_ptr: *mut PageScriptHost,
    runtime: &mut PageScriptHost,
    fragment: DomHandle,
    source: &str,
) {
    let units = source.encode_utf16().collect::<Vec<_>>();
    let document = runtime.document_handle();
    let mut handles = Vec::new();
    for node in parse_cue_text(&units) {
        let child = match node.kind {
            CueNodeKind::Text(text) => runtime.create_text_node(&String::from_utf16_lossy(&text)),
            CueNodeKind::Timestamp(timestamp) => runtime
                .create_processing_instruction_for_document(document, "timestamp", &timestamp),
            CueNodeKind::Element {
                tag,
                classes,
                annotation,
            } => {
                let child = runtime.create_element(tag.element_name());
                for (name, units) in [
                    (!classes.is_empty()).then_some(("class", classes)),
                    tag.annotation_attribute().map(|name| (name, annotation)),
                ]
                .into_iter()
                .flatten()
                {
                    let value = String::from_utf16_lossy(&units);
                    let _ = runtime.set_attribute(scope, runtime_ptr, child, name, &value);
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
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let value = private_string(scope, receiver, TEXT_TRACK_CUE_ID_SLOT);
    rv.set(
        v8_string(scope, &value)
            .unwrap_or_else(|| v8str(scope, ""))
            .into(),
    );
}

fn text_track_cue_id_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let value = match webidl::convert::<webidl::DomString>(
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
    set_private_string(scope, receiver, TEXT_TRACK_CUE_ID_SLOT, &value);
}

fn text_track_cue_vertical_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
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
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let Some(value) = args
        .get(0)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return;
    };
    if matches!(value.as_str(), "" | "rl" | "lr") {
        set_private_string(scope, receiver, TEXT_TRACK_CUE_VERTICAL_SLOT, &value);
    }
}

fn text_track_cue_snap_to_lines_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let value = private_bool(scope, receiver, TEXT_TRACK_CUE_SNAP_TO_LINES_SLOT);
    rv.set(v8::Boolean::new(scope, value).into());
}

fn text_track_cue_snap_to_lines_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
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
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let value = get_private_value(scope, receiver, TEXT_TRACK_CUE_LINE_SLOT)
        .unwrap_or_else(|| v8str(scope, "auto").into());
    rv.set(value);
}

fn text_track_cue_line_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let value = args.get(0);
    if value.is_string()
        && let Some(value) = value.to_string(scope)
        && value.to_rust_string_lossy(scope) == "auto"
    {
        set_private_string(scope, receiver, TEXT_TRACK_CUE_LINE_SLOT, "auto");
        return;
    }
    let Some(value) = finite_double_property(scope, value, "TextTrackCue", "line") else {
        return;
    };
    set_private_number(scope, receiver, TEXT_TRACK_CUE_LINE_SLOT, value);
}

fn text_track_cue_position_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let value = get_private_value(scope, receiver, TEXT_TRACK_CUE_POSITION_SLOT)
        .unwrap_or_else(|| v8str(scope, "auto").into());
    rv.set(value);
}

fn text_track_cue_position_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let value = args.get(0);
    if value.is_string()
        && let Some(value) = value.to_string(scope)
        && value.to_rust_string_lossy(scope) == "auto"
    {
        set_private_string(scope, receiver, TEXT_TRACK_CUE_POSITION_SLOT, "auto");
        return;
    }
    let Some(value) = finite_double_property(scope, value, "TextTrackCue", "position") else {
        return;
    };
    if !(0.0..=100.0).contains(&value) {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "VTTCue position must be between 0 and 100.",
        );
        return;
    }
    set_private_number(scope, receiver, TEXT_TRACK_CUE_POSITION_SLOT, value);
}

fn text_track_cue_size_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let value = private_number(scope, receiver, TEXT_TRACK_CUE_SIZE_SLOT);
    rv.set(v8::Number::new(scope, value).into());
}

fn text_track_cue_size_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
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
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
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
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let Some(value) = args
        .get(0)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        return;
    };
    if matches!(
        value.as_str(),
        "start" | "center" | "end" | "left" | "right"
    ) {
        set_private_string(scope, receiver, TEXT_TRACK_CUE_ALIGN_SLOT, &value);
    }
}

fn text_track_cue_pause_on_exit_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let value = private_bool(scope, receiver, TEXT_TRACK_CUE_PAUSE_ON_EXIT_SLOT);
    rv.set(v8::Boolean::new(scope, value).into());
}

fn text_track_cue_pause_on_exit_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
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
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    rv.set(get_private_value(scope, receiver, slot).unwrap_or_else(|| v8::null(scope).into()));
}

fn event_handler_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    event_type: &str,
    slot: &'static str,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    simple_object_event_set_handler(scope, receiver, event_type, slot, args.get(0));
}

use crate::webidl::finite_double_property_value as finite_double_property;

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
