//! AudioBuffer metadata, channel storage and Web IDL entrypoints.

use super::*;

const LENGTH: &str = "__moliAudioBufferLength";
const SAMPLE_RATE: &str = "__moliAudioBufferSampleRate";
const CHANNELS: &str = "__moliAudioBufferChannels";
const VIEWS: &str = "__moliAudioBufferViews";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AudioBuffer, enumerable, receiver)]
struct BufferPrototypeDeclaration {
    #[webapi(accessor_property, getter = metadata_getter, data = v8str(scope, LENGTH))]
    length: (),
    #[webapi(accessor_property, getter = metadata_getter, data = v8str(scope, SAMPLE_RATE))]
    sample_rate: (),
    #[webapi(accessor_property, getter = duration_getter)]
    duration: (),
    #[webapi(accessor_property, getter = channel_count_getter)]
    number_of_channels: (),
    #[webapi(method, length = 1, callback = get_channel_data)]
    get_channel_data: (),
    #[webapi(method, length = 2, callback = copy_from_channel)]
    copy_from_channel: (),
    #[webapi(method, length = 2, callback = copy_to_channel)]
    copy_to_channel: (),
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    BufferPrototypeDeclaration::initialize_prototype_template(
        scope,
        template.prototype_template(scope),
    );
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AudioBufferOptions")]
struct Options {
    #[webidl(required)]
    length: u32,
    #[webidl(default = 1)]
    number_of_channels: u32,
    #[webidl(required)]
    sample_rate: f32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "BaseAudioContext.createBuffer")]
struct CreateArgs {
    #[webidl(required)]
    number_of_channels: u32,
    #[webidl(required)]
    length: u32,
    #[webidl(required)]
    sample_rate: f32,
}

fn validate(scope: &mut v8::PinScope<'_, '_>, options: &Options) -> bool {
    format::validate(
        scope,
        options.number_of_channels,
        options.length,
        options.sample_rate,
    )
}

fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    buffer: v8::Local<'s, v8::Object>,
    length: u32,
    sample_rate: f64,
    channel_data: &[v8::Local<'s, v8::Float32Array>],
) {
    web_api_interfaces::initialize(scope, buffer, "AudioBuffer")
        .expect("AudioBuffer brand should initialize");
    set_web_audio_number_slot(scope, buffer, LENGTH, f64::from(length));
    set_web_audio_number_slot(scope, buffer, SAMPLE_RATE, sample_rate);
    let channels = private_array(scope, channel_data.len() as i32);
    let views = private_array(scope, channel_data.len() as i32);
    for (index, view) in channel_data.iter().enumerate() {
        // Keep storage through a private ArrayBuffer that is never returned to
        // script. Transferring a public channel view cannot lose native metadata.
        let store = view
            .get_backing_store()
            .expect("Channel data should have storage");
        let keeper = v8::ArrayBuffer::with_backing_store(scope, &store);
        let _ = channels.set_index(scope, index as u32, keeper.into());
        let _ = views.set_index(scope, index as u32, (*view).into());
    }
    set_private_value(scope, buffer, CHANNELS, channels.into());
    set_private_value(scope, buffer, VIEWS, views.into());
}

fn private_array<'s>(scope: &mut v8::PinScope<'s, '_>, length: i32) -> v8::Local<'s, v8::Array> {
    let array = v8::Array::new(scope, length);
    let null = v8::null(scope);
    let _ = array.set_prototype(scope, null.into());
    array
}

fn initialize_empty<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    buffer: v8::Local<'s, v8::Object>,
    options: Options,
) {
    let channels: Vec<_> = (0..options.number_of_channels)
        .map(|_| {
            let data = build_channel_data_view(scope, options.length as usize, false);
            v8::Local::<v8::Float32Array>::try_from(data)
                .expect("Channel data should be Float32Array")
        })
        .collect();
    initialize(
        scope,
        buffer,
        options.length,
        f64::from(options.sample_rate),
        &channels,
    );
}

pub(in crate::context_bootstrap) fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if !args.is_construct_call() || args.length() == 0 {
        throw_type_error(scope, "AudioBuffer requires new and an options dictionary.");
        return;
    }
    let options = match webidl::parse_dictionary::<Options>(
        scope,
        args.get(0),
        webidl::Context::argument("AudioBuffer", 1),
    ) {
        Ok(Some(options)) => options,
        Ok(None) => {
            throw_type_error(scope, "AudioBufferOptions requires length and sampleRate.");
            return;
        }
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    if validate(scope, &options) {
        initialize_empty(scope, args.this(), options);
        rv.set(args.this().into());
    }
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<CreateArgs>(scope, &args) else {
        return;
    };
    let options = Options {
        length: parsed.length,
        number_of_channels: parsed.number_of_channels,
        sample_rate: parsed.sample_rate,
    };
    if !validate(scope, &options) {
        return;
    }
    let realm = args
        .this()
        .get_creation_context(scope)
        .expect("Audio context should have a creation realm");
    let scope = &mut v8::ContextScope::new(scope, realm);
    let buffer = new_object(scope);
    initialize_empty(scope, buffer, options);
    rv.set(buffer.into());
}

fn new_object<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let prototype = super::super::exposed_interfaces::ensure_intrinsic_interface_prototype(
        scope,
        "AudioBuffer",
    )
    .expect("AudioBuffer prototype should exist");
    let buffer = v8::Object::new(scope);
    let _ = buffer.set_prototype(scope, prototype.into());
    buffer
}

pub(super) fn rendered<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    length: usize,
    sample_rate: f64,
    channels: u32,
    has_input: bool,
) -> v8::Local<'s, v8::Object> {
    let data: Vec<_> = (0..channels)
        .map(|_| {
            let data = build_channel_data_view(scope, length, has_input);
            v8::Local::<v8::Float32Array>::try_from(data)
                .expect("Channel data should be Float32Array")
        })
        .collect();
    let buffer = new_object(scope);
    initialize(scope, buffer, length as u32, sample_rate, &data);
    buffer
}

pub(super) fn sample_rate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    buffer: v8::Local<'s, v8::Object>,
) -> f64 {
    web_audio_number_slot(scope, buffer, SAMPLE_RATE).unwrap_or(0.0)
}

pub(super) fn channel_count<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    buffer: v8::Local<'s, v8::Object>,
) -> u32 {
    web_audio_array_slot(scope, buffer, CHANNELS).map_or(0, |channels| channels.length())
}

fn metadata_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let slot = args.data().to_rust_string_lossy(scope);
    if let Some(value) = get_private_value(scope, args.this(), &slot) {
        rv.set(value);
    }
}

fn duration_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let length = web_audio_number_slot(scope, args.this(), LENGTH).unwrap_or(0.0);
    rv.set_double(length / sample_rate(scope, args.this()));
}

fn channel_count_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    rv.set_uint32(channel_count(scope, args.this()));
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioBuffer.getChannelData")]
struct ChannelArgs {
    #[webidl(required)]
    channel: u32,
}

fn get_channel_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<ChannelArgs>(scope, &args) else {
        return;
    };
    if !valid_channel(scope, args.this(), parsed.channel) {
        return;
    }
    let views = web_audio_array_slot(scope, args.this(), VIEWS)
        .expect("AudioBuffer should have channel views");
    let existing = views
        .get_index(scope, parsed.channel)
        .expect("Channel view slot should exist");
    if !existing.is_undefined() {
        rv.set(existing);
        return;
    }
    let channels = web_audio_array_slot(scope, args.this(), CHANNELS)
        .expect("AudioBuffer should have storage");
    let keeper = channels
        .get_index(scope, parsed.channel)
        .and_then(|v| v8::Local::<v8::ArrayBuffer>::try_from(v).ok())
        .expect("Channel storage should be an ArrayBuffer");
    let store = keeper.get_backing_store();
    let realm = args
        .this()
        .get_creation_context(scope)
        .expect("AudioBuffer should have a creation realm");
    let scope = &mut v8::ContextScope::new(scope, realm);
    let array_buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    let length = web_audio_number_slot(scope, args.this(), LENGTH).unwrap_or(0.0) as usize;
    let view = v8::Float32Array::new(scope, array_buffer, 0, length)
        .expect("Channel view should allocate");
    let _ = views.set_index(scope, parsed.channel, view.into());
    rv.set(view.into());
}

pub(super) fn nullable_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Result<Option<v8::Local<'s, v8::Object>>, webidl::WebIdlError> {
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    let buffer = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| webidl::WebIdlError::cannot_convert(context, "AudioBuffer"))?;
    if !web_api_interfaces::AudioBuffer::is_instance(scope, buffer) {
        return Err(webidl::WebIdlError::cannot_convert(context, "AudioBuffer"));
    }
    Ok(Some(buffer))
}

/// Snapshot content before detaching public views. Later writes through a new
/// getChannelData view affect the buffer, not content already acquired by a node.
pub(super) fn acquire<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    buffer: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Array> {
    let result = private_array(scope, 0);
    let Some(channels) = web_audio_array_slot(scope, buffer, CHANNELS) else {
        return result;
    };
    let views = web_audio_array_slot(scope, buffer, VIEWS).expect("AudioBuffer should have views");
    for index in 0..views.length() {
        if let Some(view) = views
            .get_index(scope, index)
            .and_then(|v| v8::Local::<v8::Float32Array>::try_from(v).ok())
            && view.buffer(scope).is_some_and(|b| b.was_detached())
        {
            return result;
        }
    }
    for index in 0..channels.length() {
        let keeper = channels
            .get_index(scope, index)
            .and_then(|v| v8::Local::<v8::ArrayBuffer>::try_from(v).ok())
            .expect("Channel storage should be an ArrayBuffer");
        let store = keeper.get_backing_store();
        let bytes: Vec<_> = store.iter().map(std::cell::Cell::get).collect();
        let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
        let snapshot = v8::ArrayBuffer::with_backing_store(scope, &store);
        let _ = result.set_index(scope, index, snapshot.into());
    }
    for index in 0..views.length() {
        if let Some(view) = views
            .get_index(scope, index)
            .and_then(|v| v8::Local::<v8::Float32Array>::try_from(v).ok())
        {
            let _ = view
                .buffer(scope)
                .expect("Channel view should have a buffer")
                .detach(None);
        }
        let value = v8::undefined(scope);
        let _ = views.set_index(scope, index, value.into());
    }
    result
}

fn valid_channel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    buffer: v8::Local<'s, v8::Object>,
    index: u32,
) -> bool {
    if index >= channel_count(scope, buffer) {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "AudioBuffer channel index is out of range.",
        );
        return false;
    }
    true
}

pub(super) fn float32_array_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    context: webidl::Context,
) -> Result<v8::Local<'s, v8::Float32Array>, webidl::WebIdlError> {
    let view = v8::Local::<v8::Float32Array>::try_from(value)
        .map_err(|_| webidl::WebIdlError::cannot_convert(context, "Float32Array"))?;
    let store = view
        .buffer(scope)
        .expect("Float32Array should have a buffer")
        .get_backing_store();
    if store.is_shared() || store.is_resizable_by_user_javascript() {
        return Err(webidl::WebIdlError::cannot_convert(
            context,
            "non-shared, fixed-length Float32Array",
        ));
    }
    Ok(view)
}

fn copy_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Float32Array>, webidl::WebIdlError> {
    float32_array_value(
        scope,
        args.get(index),
        webidl::Context::argument("AudioBuffer channel copy", index as usize + 1),
    )
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioBuffer channel copy")]
struct CopyArgs<'s> {
    #[webidl(required, with = copy_array)]
    array: v8::Local<'s, v8::Float32Array>,
    #[webidl(required)]
    channel: u32,
    #[webidl(default = 0)]
    offset: u32,
}

fn copy_channel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    to_channel: bool,
) {
    let Some(parsed) = webidl::parse_args::<CopyArgs>(scope, &args) else {
        return;
    };
    if !valid_channel(scope, args.this(), parsed.channel) {
        return;
    }
    let length = web_audio_number_slot(scope, args.this(), LENGTH).unwrap_or(0.0) as usize;
    let count = length
        .saturating_sub(parsed.offset as usize)
        .min(parsed.array.length());
    if count == 0 {
        return;
    }
    let channels = web_audio_array_slot(scope, args.this(), CHANNELS)
        .expect("AudioBuffer should have storage");
    let channel = channels
        .get_index(scope, parsed.channel)
        .and_then(|v| v8::Local::<v8::ArrayBuffer>::try_from(v).ok())
        .expect("Channel storage should be an ArrayBuffer");
    let channel_store = channel.get_backing_store();
    let array_store = parsed
        .array
        .get_backing_store()
        .expect("Copy array should have storage");
    let array_offset = parsed.array.byte_offset();
    let channel_offset = parsed.offset as usize * std::mem::size_of::<f32>();
    let byte_count = count * std::mem::size_of::<f32>();
    let (source, start) = if to_channel {
        (&array_store, array_offset)
    } else {
        (&channel_store, channel_offset)
    };
    // Take a snapshot before writing so overlapping views have memmove semantics.
    let bytes: Vec<_> = source[start..start + byte_count]
        .iter()
        .map(std::cell::Cell::get)
        .collect();
    let (destination, start) = if to_channel {
        (&channel_store, channel_offset)
    } else {
        (&array_store, array_offset)
    };
    for (target, value) in destination[start..start + byte_count].iter().zip(bytes) {
        target.set(value);
    }
}

fn copy_from_channel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    copy_channel(scope, args, false);
}
fn copy_to_channel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s>,
) {
    copy_channel(scope, args, true);
}
