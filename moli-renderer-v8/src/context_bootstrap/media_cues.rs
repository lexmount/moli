use super::super::util::{throw_type_error, v8_string};
use super::context_host_ptr_from_global_bridge;
use super::mark_simple_event_target_slot;
use crate::document_runtime::DomHandle;
use crate::native_bridge::JsContextHost;
use crate::native_bridge::throw_dom_exception;
use crate::util::{get_private_value, set_private_value, v8str};
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

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
#[webapi(interface = "VTTCue")]
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
#[webapi(name = "TextTrackCue", enumerable)]
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
#[webapi(name = "VTTCue", enumerable)]
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
#[webapi(name = "MediaError", enumerable)]
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
#[webapi(name = "MediaError", enumerable)]
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
    let global = scope.get_current_context().global(scope);
    let constructor = global.get(scope, v8str(scope, "MediaError").into())?;
    let constructor = v8::Local::<v8::Object>::try_from(constructor).ok()?;
    let prototype = constructor.get(scope, v8str(scope, "prototype").into())?;
    let object = v8::Object::new(scope);
    if object.set_prototype(scope, prototype) != Some(true) {
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

    mark_simple_event_target_slot(scope, args.this(), "__moliTextTrackCueListeners");
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
    if get_private_value(scope, receiver, TEXT_TRACK_CUE_START_TIME_SLOT).is_some() {
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
    let text = receiver
        .get(scope, v8str(scope, "text").into())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let fragment = runtime.create_document_fragment();
    append_vtt_cue_fragment(scope, runtime_ptr, runtime, fragment, &text);
    if let Some(fragment) = runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, fragment)
    {
        rv.set(fragment.into());
    }
}

fn append_vtt_cue_fragment(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    runtime: &mut JsContextHost,
    fragment: DomHandle,
    source: &str,
) {
    if source.is_empty() {
        let text_node = runtime.create_text_node("");
        let _ = runtime.append_child(scope, runtime_ptr, fragment, text_node);
        return;
    }
    let mut stack = vec![VttCueFragmentNode {
        handle: fragment,
        tag: VttCueFragmentTag::Root,
    }];
    let mut text = String::new();
    let mut cursor = 0usize;
    while cursor < source.len() {
        let remaining = &source[cursor..];
        if let Some(entity) = remaining.strip_prefix('&')
            && let Some(end) = entity.find(';')
        {
            let name = &entity[..end];
            if let Some(decoded) = decode_vtt_cue_entity(name) {
                text.push(decoded);
                cursor += end + 2;
                continue;
            }
        }
        if remaining.starts_with('<') {
            let Some(end) = remaining.find('>') else {
                break;
            };
            flush_vtt_cue_text(scope, runtime_ptr, runtime, &stack, &mut text);
            let tag = &remaining[1..end];
            apply_vtt_cue_tag(scope, runtime_ptr, runtime, &mut stack, tag);
            cursor += end + 1;
            continue;
        }
        let Some(ch) = remaining.chars().next() else {
            break;
        };
        text.push(ch);
        cursor += ch.len_utf8();
    }
    flush_vtt_cue_text(scope, runtime_ptr, runtime, &stack, &mut text);
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum VttCueFragmentTag {
    Root,
    Element(&'static str),
}

#[derive(Clone, Copy, Debug)]
struct VttCueFragmentNode {
    handle: DomHandle,
    tag: VttCueFragmentTag,
}

fn flush_vtt_cue_text(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    runtime: &mut JsContextHost,
    stack: &[VttCueFragmentNode],
    text: &mut String,
) {
    if text.is_empty() {
        return;
    }
    let text_node = runtime.create_text_node(text);
    let parent = stack
        .last()
        .map(|node| node.handle)
        .unwrap_or_else(|| stack[0].handle);
    let _ = runtime.append_child(scope, runtime_ptr, parent, text_node);
    text.clear();
}

fn apply_vtt_cue_tag(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    runtime: &mut JsContextHost,
    stack: &mut Vec<VttCueFragmentNode>,
    raw: &str,
) {
    if let Some(close_name) = raw.strip_prefix('/') {
        if close_name.starts_with(|ch: char| ch.is_ascii_whitespace()) {
            return;
        }
        let close_name = close_name
            .split(|ch: char| ch == '.' || ch.is_ascii_whitespace())
            .next()
            .unwrap_or("");
        let close_name = canonical_vtt_cue_fragment_tag(close_name);
        if stack
            .last()
            .is_some_and(|node| node.tag == VttCueFragmentTag::Element(close_name))
            && stack.len() > 1
        {
            stack.pop();
        }
        return;
    }

    if raw.starts_with(|ch: char| ch.is_ascii_whitespace()) {
        return;
    }
    let parsed = parse_vtt_cue_open_tag(raw);
    let Some(parsed) = parsed else {
        return;
    };
    if parsed.tag == "rt"
        && !stack
            .iter()
            .any(|node| node.tag == VttCueFragmentTag::Element("ruby"))
    {
        return;
    }
    let element_name = if matches!(parsed.tag, "c" | "v") {
        "span"
    } else {
        parsed.tag
    };
    let child = runtime.create_element(element_name);
    if let Some(class_name) = parsed.class_name
        && !class_name.is_empty()
    {
        let _ = runtime.set_attribute(scope, runtime_ptr, child, "class", &class_name);
    }
    if let Some(title) = parsed.title
        && !title.is_empty()
    {
        let _ = runtime.set_attribute(scope, runtime_ptr, child, "title", &title);
    }
    let parent = stack
        .last()
        .map(|node| node.handle)
        .unwrap_or_else(|| stack[0].handle);
    let _ = runtime.append_child(scope, runtime_ptr, parent, child);
    stack.push(VttCueFragmentNode {
        handle: child,
        tag: VttCueFragmentTag::Element(parsed.tag),
    });
}

struct ParsedVttCueOpenTag {
    tag: &'static str,
    class_name: Option<String>,
    title: Option<String>,
}

fn parse_vtt_cue_open_tag(raw: &str) -> Option<ParsedVttCueOpenTag> {
    let mut boundary = raw.len();
    for (index, ch) in raw.char_indices() {
        if ch == '.' || ch.is_ascii_whitespace() {
            boundary = index;
            break;
        }
    }
    let tag = canonical_vtt_cue_fragment_tag(&raw[..boundary]);
    if !matches!(tag, "b" | "i" | "u" | "ruby" | "rt" | "c" | "v") {
        return None;
    }
    let rest = &raw[boundary..];
    let (class_part, title_part) = if let Some(title_start) = rest.find(char::is_whitespace) {
        (&rest[..title_start], rest[title_start..].trim())
    } else {
        (rest, "")
    };
    let class_name = parse_vtt_cue_classes(class_part);
    let title = (tag == "v" && !title_part.is_empty()).then(|| title_part.to_owned());
    Some(ParsedVttCueOpenTag {
        tag,
        class_name,
        title,
    })
}

fn canonical_vtt_cue_fragment_tag(name: &str) -> &'static str {
    match name {
        "b" => "b",
        "i" => "i",
        "u" => "u",
        "ruby" => "ruby",
        "rt" => "rt",
        "c" => "c",
        "v" => "v",
        _ => "",
    }
}

fn parse_vtt_cue_classes(raw: &str) -> Option<String> {
    let classes = raw
        .split('.')
        .skip(1)
        .filter(|class_name| !class_name.is_empty())
        .collect::<Vec<_>>();
    (!classes.is_empty()).then(|| classes.join(" "))
}

#[cfg(test)]
fn vtt_cue_text_content(source: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0usize;
    while cursor < source.len() {
        let remaining = &source[cursor..];
        if let Some(entity) = remaining.strip_prefix('&')
            && let Some(end) = entity.find(';')
        {
            let name = &entity[..end];
            if let Some(decoded) = decode_vtt_cue_entity(name) {
                output.push(decoded);
                cursor += end + 2;
                continue;
            }
        }
        if remaining.starts_with('<') {
            let Some(end) = remaining.find('>') else {
                break;
            };
            cursor += end + 1;
            continue;
        }
        let Some(ch) = remaining.chars().next() else {
            break;
        };
        output.push(ch);
        cursor += ch.len_utf8();
    }
    output
}

fn decode_vtt_cue_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "lrm" => Some('\u{200e}'),
        "rlm" => Some('\u{200f}'),
        "nbsp" => Some('\u{00a0}'),
        _ => None,
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
    event_handler_setter(scope, args, TEXT_TRACK_CUE_ONENTER_SLOT);
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
    event_handler_setter(scope, args, TEXT_TRACK_CUE_ONEXIT_SLOT);
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
    slot: &'static str,
) {
    let Some(receiver) = text_track_cue_receiver(scope, &args) else {
        return;
    };
    let value = args.get(0);
    if value.is_function() {
        set_private_value(scope, receiver, slot, value);
    } else {
        set_private_value(scope, receiver, slot, v8::null(scope).into());
    }
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
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::vtt_cue_text_content;

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
