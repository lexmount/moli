use bytes::Bytes;
use moli_web_mime::{MediaSourceTracks, media_source_tracks};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use super::media_queries::{
    dispatch_simple_event_target_event, initialize_simple_event_target,
    simple_event_target_handler_getter_callback as handler,
    simple_event_target_handler_setter_callback as set_handler,
};
use crate::document_runtime::DomHandle;
use crate::native_bridge::PageScriptHost;
use crate::util::{
    context_host_ptr_from_global_bridge, get_private_object, get_private_value, set_private_value,
    v8str,
};
use crate::{web_api_interfaces, webidl};

const MEDIA_SOURCE: &str = "__moliMediaSource";
const ATTACHED: &str = "__moliMediaSourceVideo";
const ENDED: &str = "__moliMediaSourceEnded";
const DURATION: &str = "__moliMediaSourceDuration";
const BUFFERS: &str = "__moliMediaSourceTrackBuffers";
const SOURCE_BUFFERS: &str = "__moliMediaSourceBuffers";
const ACTIVE_BUFFERS: &str = "__moliMediaSourceActiveBuffers";
const UPDATING: &str = "__moliSourceBufferUpdating";
const OFFSET: &str = "__moliSourceBufferTimestampOffset";
const TRACKS: &str = "__moliSourceBufferTracks";
const MAX_APPEND_BYTES: usize = 128 * 1024 * 1024;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaSource, enumerable)]
struct SourcePrototype {
    #[webapi(accessor_property, getter = source_state)]
    ready_state: (),
    #[webapi(accessor_property, getter = duration, setter = set_duration)]
    duration: (),
    #[webapi(accessor_property, getter = lists, data = v8str(scope, SOURCE_BUFFERS))]
    source_buffers: (),
    #[webapi(accessor_property, getter = lists, data = v8str(scope, ACTIVE_BUFFERS))]
    active_source_buffers: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler, data = v8str(scope, "onsourceopen"))]
    onsourceopen: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler, data = v8str(scope, "onsourceclose"))]
    onsourceclose: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler, data = v8str(scope, "onsourceended"))]
    onsourceended: (),
    #[webapi(method, length = 1, callback = add_source_buffer)]
    add_source_buffer: (),
    #[webapi(method, length = 0, callback = end_of_stream)]
    end_of_stream: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SourceBuffer, enumerable)]
struct BufferPrototype {
    #[webapi(accessor_property, getter = updating)]
    updating: (),
    #[webapi(accessor_property, getter = buffered)]
    buffered: (),
    #[webapi(accessor_property, getter = timestamp_offset, setter = set_timestamp_offset)]
    timestamp_offset: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler, data = v8str(scope, "onupdatestart"))]
    onupdatestart: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler, data = v8str(scope, "onupdate"))]
    onupdate: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler, data = v8str(scope, "onupdateend"))]
    onupdateend: (),
    #[webapi(accessor_property, getter = handler, setter = set_handler, data = v8str(scope, "onerror"))]
    onerror: (),
    #[webapi(method, length = 1, callback = append_buffer)]
    append_buffer: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::SourceBufferList, enumerable)]
struct ListPrototype {
    #[webapi(accessor_property, getter = list_length)]
    length: (),
    #[webapi(intrinsic_data_property = v8::Intrinsic::ArrayProtoValues, symbol = "iterator")]
    iterator: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::SourceBuffer, require_prototype)]
struct BufferObject<'s> {
    #[webapi(slot = MEDIA_SOURCE)]
    source: v8::Local<'s, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::SourceBufferList, require_prototype)]
struct ListObject<'s> {
    #[webapi(slot = MEDIA_SOURCE)]
    source: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSource.addSourceBuffer")]
struct AddSourceBufferArgs {
    #[webidl(required, name = "type")]
    media_type: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "SourceBuffer.appendBuffer")]
struct AppendBufferArgs<'s> {
    #[webidl(required, converter = "raw")]
    data: v8::Local<'s, v8::Value>,
}

pub(crate) fn media_source_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = args.this();
    initialize_simple_event_target(scope, source);
    set_flag(scope, source, MEDIA_SOURCE, true);
    rv.set(source.into());
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    match name {
        "MediaSource" => SourcePrototype::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        ),
        "SourceBuffer" => BufferPrototype::initialize_prototype_template(
            scope,
            template.prototype_template(scope),
        ),
        "SourceBufferList" => {
            ListPrototype::initialize_prototype_template(scope, template.prototype_template(scope))
        }
        _ => {}
    }
}

fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, object)
        .expect("validated media-source receiver")
}

pub(super) fn media_source_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = moli_webapi_declare::web_api_object_target(scope, object)?;
    flag(scope, object, MEDIA_SOURCE).then_some(object)
}

pub(crate) fn attached_media<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    let value = get_private_value(scope, source, ATTACHED)?;
    let index = v8::Local::<v8::Number>::try_from(value).ok()?.value();
    Some(DomHandle::new(index as usize))
}

pub(crate) fn attach(
    scope: &mut v8::PinScope<'_, '_>,
    source: v8::Local<'_, v8::Object>,
    media: DomHandle,
) {
    let index = v8::Number::new(scope, media.index() as f64);
    set_private_value(scope, source, ATTACHED, index.into());
}

pub(crate) fn detach(scope: &mut v8::PinScope<'_, '_>, source: v8::Local<'_, v8::Object>) {
    let undefined = v8::undefined(scope);
    for key in [ATTACHED, ENDED, DURATION, BUFFERS] {
        set_private_value(scope, source, key, undefined.into());
    }
}

pub(crate) fn publish_duration(
    scope: &mut v8::PinScope<'_, '_>,
    source: v8::Local<'_, v8::Object>,
    value: f64,
) {
    let value = v8::Number::new(scope, value);
    set_private_value(scope, source, DURATION, value.into());
}

pub(crate) fn set_updating(
    scope: &mut v8::PinScope<'_, '_>,
    buffer: v8::Local<'_, v8::Object>,
    updating: bool,
) {
    set_flag(scope, buffer, UPDATING, updating);
}

pub(crate) fn dispatch_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) {
    if let Some(event) =
        crate::native_bridge::element::construct_simple_event(scope, name, false, false, false)
    {
        dispatch_simple_event_target_event(scope, object, name, event);
    }
}

fn set_flag(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &'static str,
    value: bool,
) {
    let value = v8::Boolean::new(scope, value);
    set_private_value(scope, object, key, value.into());
}

fn flag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &str,
) -> bool {
    get_private_value(scope, object, key).is_some_and(|value| value.is_true())
}

fn state<'s>(scope: &mut v8::PinScope<'s, '_>, source: v8::Local<'s, v8::Object>) -> &'static str {
    if source_attachment(scope, source).is_none() {
        "closed"
    } else if flag(scope, source, ENDED) {
        "ended"
    } else {
        "open"
    }
}

fn source_attachment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<(*mut PageScriptHost, DomHandle)> {
    let media = attached_media(scope, source)?;
    let host = context_host_ptr_from_global_bridge(scope)?;
    unsafe { &*host }
        .owns_media_source(scope, media, source)
        .then_some((host, media))
}

fn source_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = target(scope, args.this());
    let state = state(scope, source);
    rv.set(v8str(scope, state).into());
}

fn duration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = target(scope, args.this());
    let duration = get_private_value(scope, source, DURATION)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(f64::NAN);
    rv.set_double(duration);
}

fn set_duration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(value) = args.get(0).number_value(scope) else {
        return;
    };
    if value.is_nan() || value < 0.0 {
        return crate::util::throw_type_error(scope, "MediaSource duration must be nonnegative.");
    }
    let source = target(scope, args.this());
    let Some((host, media)) = open_source(scope, source) else {
        return;
    };
    publish_duration(scope, source, value);
    unsafe { &mut *host }.set_media_source_duration(media, value);
}

fn open_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<(*mut PageScriptHost, DomHandle)> {
    if !flag(scope, source, ENDED)
        && let Some(attachment) = source_attachment(scope, source)
    {
        return Some(attachment);
    }
    invalid_state(scope);
    None
}

fn invalid_state(scope: &mut v8::PinScope<'_, '_>) {
    super::constructors::throw_dom_exception_value(
        scope,
        "The MediaSource or SourceBuffer is unavailable.",
        "InvalidStateError",
    );
}

fn add_source_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<AddSourceBufferArgs>(scope, &args) else {
        return;
    };
    if parsed.media_type.is_empty() {
        return crate::util::throw_type_error(scope, "The SourceBuffer media type is empty.");
    }
    let Some(tracks) = media_source_tracks(
        &parsed.media_type,
        moli_webapi::media_source::SUPPORTS_MSE_AAC,
    ) else {
        return super::constructors::throw_dom_exception_value(
            scope,
            "This SourceBuffer media type is unsupported.",
            "NotSupportedError",
        );
    };
    let source = target(scope, args.this());
    let Some((host, media)) = open_source(scope, source) else {
        return;
    };
    if !unsafe { &mut *host }.add_media_source_tracks(media, tracks) {
        return super::constructors::throw_dom_exception_value(
            scope,
            "The MediaSource already has a buffer for these tracks.",
            "QuotaExceededError",
        );
    }
    let Some(context) = source.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Ok(buffer) = BufferObject::new(source).bind(scope) else {
        return;
    };
    initialize_simple_event_target(scope, buffer);
    let track_code = v8::Integer::new(
        scope,
        match tracks {
            MediaSourceTracks::Video => 1,
            MediaSourceTracks::Audio => 2,
            MediaSourceTracks::VideoAndAudio => 3,
        },
    );
    set_private_value(scope, buffer, TRACKS, track_code.into());
    let buffers = source_buffers(scope, source).unwrap_or_else(|| {
        let buffers = v8::Array::new(scope, 0);
        set_private_value(scope, source, BUFFERS, buffers.into());
        buffers
    });
    buffers.set_index(scope, buffers.length(), buffer.into());
    rv.set(buffer.into());
}

fn end_of_stream<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = target(scope, args.this());
    let Some((host, media)) = open_source(scope, source) else {
        return;
    };
    if source_buffers(scope, source).is_some_and(|buffers| {
        (0..buffers.length()).any(|index| {
            buffer_at(scope, buffers, index).is_some_and(|buffer| flag(scope, buffer, UPDATING))
        })
    }) {
        return invalid_state(scope);
    }
    if let Some(duration) = unsafe { &mut *host }.end_media_source(scope, media) {
        set_flag(scope, source, ENDED, true);
        publish_duration(scope, source, duration);
    } else {
        invalid_state(scope);
    }
}

fn lists<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let source = target(scope, args.this());
    let key = args.data().to_rust_string_lossy(scope);
    if let Some(list) = get_private_object(scope, source, &key) {
        rv.set(list.into());
        return;
    }
    let Some(context) = source.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let template = v8::ObjectTemplate::new(scope);
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(index)
            .query(query_index),
    );
    let Some(list) = template.new_instance(scope) else {
        return;
    };
    if ListObject::new(source).bind_into(scope, list).is_err() {
        return;
    }
    initialize_simple_event_target(scope, list);
    set_private_value(scope, source, &key, list.into());
    rv.set(list.into());
}

fn source_buffers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_object(scope, source, BUFFERS)?.try_into().ok()
}

fn buffer_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    buffers: v8::Local<'s, v8::Array>,
    index: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    let value = buffers.get_index(scope, index)?;
    value.try_into().ok()
}

fn list_buffers<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let source = get_private_object(scope, list, MEDIA_SOURCE)?;
    source_buffers(scope, source)
}

fn list_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let list = target(scope, args.this());
    rv.set_uint32(list_buffers(scope, list).map_or(0, |buffers| buffers.length()));
}

fn index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    if let Some(buffers) = list_buffers(scope, args.holder())
        && let Some(buffer) = buffer_at(scope, buffers, index)
    {
        rv.set(buffer.into());
        v8::Intercepted::kYes
    } else {
        v8::Intercepted::kNo
    }
}

fn query_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if list_buffers(scope, args.holder()).is_some_and(|buffers| index < buffers.length()) {
        rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
        v8::Intercepted::kYes
    } else {
        v8::Intercepted::kNo
    }
}

fn updating<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let buffer = target(scope, args.this());
    rv.set_bool(flag(scope, buffer, UPDATING));
}

fn buffer_attachment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    buffer: v8::Local<'s, v8::Object>,
) -> Option<(*mut PageScriptHost, DomHandle, v8::Local<'s, v8::Object>)> {
    let source = get_private_object(scope, buffer, MEDIA_SOURCE)?;
    if let Some(buffers) = source_buffers(scope, source)
        && (0..buffers.length()).any(|index| buffer_at(scope, buffers, index) == Some(buffer))
        && let Some((host, media)) = source_attachment(scope, source)
    {
        return Some((host, media, source));
    }
    invalid_state(scope);
    None
}

fn buffer_tracks<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    buffer: v8::Local<'s, v8::Object>,
) -> MediaSourceTracks {
    match get_private_value(scope, buffer, TRACKS).and_then(|value| value.uint32_value(scope)) {
        Some(1) => MediaSourceTracks::Video,
        Some(2) => MediaSourceTracks::Audio,
        Some(3) => MediaSourceTracks::VideoAndAudio,
        _ => unreachable!("attached SourceBuffer must have a track type"),
    }
}

fn buffered<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let buffer = target(scope, args.this());
    let Some((host, media, _)) = buffer_attachment(scope, buffer) else {
        return;
    };
    let ranges = unsafe { &*host }.media_source_buffered(media, buffer_tracks(scope, buffer));
    rv.set(super::new_time_ranges_value(scope, &ranges).into());
}

fn offset<'s>(scope: &mut v8::PinScope<'s, '_>, buffer: v8::Local<'s, v8::Object>) -> f64 {
    get_private_value(scope, buffer, OFFSET)
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0)
}

fn timestamp_offset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let buffer = target(scope, args.this());
    rv.set_double(offset(scope, buffer));
}

fn set_timestamp_offset<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = match webidl::convert::<webidl::Double>(
        scope,
        args.get(0),
        webidl::Context::member("SourceBuffer", "timestampOffset"),
    ) {
        Ok(value) => value.0,
        Err(error) => return webidl::throw_error(scope, &error),
    };
    let buffer = target(scope, args.this());
    if buffer_attachment(scope, buffer).is_none() {
        return;
    }
    if flag(scope, buffer, UPDATING) {
        return invalid_state(scope);
    }
    let value = v8::Number::new(scope, value);
    set_private_value(scope, buffer, OFFSET, value.into());
}

fn append_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<AppendBufferArgs>(scope, &args) else {
        return;
    };
    let data = match webidl::convert::<webidl::NonSharedBufferSource>(
        scope,
        parsed.data,
        webidl::Context::member("SourceBuffer", "appendBuffer"),
    ) {
        Ok(data) => data,
        Err(error) => return webidl::throw_error(scope, &error),
    };
    let buffer = target(scope, args.this());
    let Some((host, media, source)) = buffer_attachment(scope, buffer) else {
        return;
    };
    if state(scope, source) != "open" || flag(scope, buffer, UPDATING) {
        return invalid_state(scope);
    }
    let value = parsed.data;
    let len = v8::Local::<v8::ArrayBuffer>::try_from(value).map_or_else(
        |_| v8::Local::<v8::ArrayBufferView>::try_from(value).map_or(0, |view| view.byte_length()),
        |buffer| buffer.byte_length(),
    );
    if len > MAX_APPEND_BYTES {
        return super::constructors::throw_dom_exception_value(
            scope,
            "The SourceBuffer limit was exceeded.",
            "QuotaExceededError",
        );
    }
    let bytes = Bytes::from(data.to_vec(scope));
    let offset = offset(scope, buffer);
    let tracks = buffer_tracks(scope, buffer);
    if unsafe { &mut *host }.append_media_source_buffer(scope, media, buffer, tracks, bytes, offset)
    {
        set_updating(scope, buffer, true);
    } else {
        invalid_state(scope);
    }
}
