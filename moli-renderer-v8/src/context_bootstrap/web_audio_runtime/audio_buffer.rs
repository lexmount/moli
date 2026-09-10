//! Owned PCM channels shared by AudioBuffer's constructor and context factories.
//!
//! The channel views are private, GC-traced values. Metadata remains valid when
//! a caller transfers a channel's ArrayBuffer; all copies use intrinsic view
//! bounds after argument conversion, never author-overridable JS properties.

use super::*;
use crate::util::callback_data_index_value;

const CHANNELS: &str = "__moliAudioBufferChannels";
const LENGTH: &str = "__moliAudioBufferLength";
const SAMPLE_RATE: &str = "__moliAudioBufferSampleRate";

#[derive(WebApiObject)]
#[webapi(interface = "AudioBuffer")]
struct AudioBufferData<'s> {
    #[webapi(slot = CHANNELS)]
    channels: v8::Local<'s, v8::Array>,
    #[webapi(slot = LENGTH)]
    length: u32,
    #[webapi(slot = SAMPLE_RATE)]
    sample_rate: f64,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "AudioBufferOptions")]
struct AudioBufferOptions {
    // WebIDL dictionary members are converted in lexicographic order.
    #[webidl(required)]
    length: u32,
    #[webidl(default = 1)]
    number_of_channels: u32,
    #[webidl(required, converter = "double")]
    sample_rate: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioBuffer")]
struct ConstructorArgs<'s> {
    #[webidl(required)]
    options: v8::Local<'s, v8::Object>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "BaseAudioContext.createBuffer")]
struct CreateBufferArgs {
    #[webidl(required)]
    number_of_channels: u32,
    #[webidl(required)]
    length: u32,
    #[webidl(required, converter = "double")]
    sample_rate: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioBuffer.getChannelData")]
struct GetChannelDataArgs {
    #[webidl(required)]
    channel: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioBuffer channel copy")]
struct CopyChannelArgs {
    #[webidl(index = 1, required)]
    channel: u32,
    #[webidl(index = 2, default = 0)]
    offset: u32,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "AudioBuffer", constructor_callback = constructor, constructor_length = 1, enumerable)]
struct AudioBufferTemplate {
    #[webapi(accessor_property, getter = metadata, data = callback_data_index_value(scope, 0))]
    length: (),
    #[webapi(accessor_property, getter = metadata, data = callback_data_index_value(scope, 1))]
    duration: (),
    #[webapi(accessor_property, getter = metadata, data = callback_data_index_value(scope, 2))]
    sample_rate: (),
    #[webapi(accessor_property, getter = metadata, data = callback_data_index_value(scope, 3))]
    number_of_channels: (),
    #[webapi(method, length = 1, callback = get_channel_data)]
    get_channel_data: (),
    #[webapi(method, length = 2, callback = copy_from_channel)]
    copy_from_channel: (),
    #[webapi(method, length = 2, callback = copy_to_channel)]
    copy_to_channel: (),
}

pub(in crate::context_bootstrap) fn build_constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    AudioBufferTemplate::build(scope)
}

fn constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'AudioBuffer': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<ConstructorArgs>(scope, &args) else {
        return;
    };
    let options = match webidl::parse_dictionary_object::<AudioBufferOptions>(scope, parsed.options)
    {
        Ok(options) => options,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    let Some(data) = allocate(
        scope,
        options.number_of_channels,
        options.length,
        options.sample_rate,
    ) else {
        return;
    };
    // Initialize the object allocated for newTarget, preserving subclasses.
    data.initialize(scope, args.this())
        .expect("AudioBuffer should initialize");
    rv.set(args.this().into());
}

pub(super) fn create_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_base_audio_context(scope, args.this()) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<CreateBufferArgs>(scope, &args) else {
        return;
    };
    if let Some(buffer) = new_buffer(
        scope,
        parsed.number_of_channels,
        parsed.length,
        parsed.sample_rate,
    ) {
        rv.set(buffer.into());
    }
}

pub(super) fn new_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    number_of_channels: u32,
    length: u32,
    sample_rate: f64,
) -> Option<v8::Local<'s, v8::Object>> {
    let data = allocate(scope, number_of_channels, length, sample_rate)?;
    Some(data.bind(scope).expect("AudioBuffer should bind"))
}

fn allocate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    number_of_channels: u32,
    length: u32,
    sample_rate: f64,
) -> Option<AudioBufferData<'s>> {
    // AudioBufferOptions uses restricted float, not double. Round before the
    // algorithm's range checks, including values near the supported endpoints.
    let sample_rate = sample_rate as f32 as f64;
    if !sample_rate.is_finite() {
        throw_type_error(scope, "AudioBuffer sampleRate must be a finite float.");
        return None;
    }
    if !(1..=32).contains(&number_of_channels)
        || !(3000.0..=768_000.0).contains(&sample_rate)
        || length == 0
    {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "AudioBuffer requires 1 to 32 channels, a positive length, and a sample rate between 3000 and 768000.",
        );
        return None;
    }
    let Some(byte_length) = (length as usize)
        .checked_mul(size_of::<f32>())
        .filter(|_| length as usize <= v8::Float32Array::MAX_LENGTH)
    else {
        allocation_failed(scope);
        return None;
    };
    let mut channels = Vec::with_capacity(number_of_channels as usize);
    for _ in 0..number_of_channels {
        let mut bytes = Vec::new();
        if bytes.try_reserve_exact(byte_length).is_err() {
            allocation_failed(scope);
            return None;
        }
        bytes.resize(byte_length, 0);
        let store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
        let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
        let Some(view) = v8::Float32Array::new(scope, buffer, 0, length as usize) else {
            allocation_failed(scope);
            return None;
        };
        channels.push(view.into());
    }
    // Define own dense elements directly. Array.prototype setters must not see
    // this internal collection, nor can callers obtain or mutate it.
    let channels = v8::Array::new_with_elements(scope, &channels);
    Some(AudioBufferData::new(channels, length, sample_rate))
}

fn allocation_failed(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "NotSupportedError",
        9,
        "Unable to allocate AudioBuffer channel data.",
    );
}

fn require_channels<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    let channels = web_audio_array_slot(scope, receiver, CHANNELS);
    if channels.is_none() {
        throw_type_error(scope, "Illegal invocation: expected an AudioBuffer.");
    }
    channels
}

fn channel_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    channels: v8::Local<'s, v8::Array>,
    channel: u32,
) -> Option<v8::Local<'s, v8::Float32Array>> {
    if channel >= channels.length() {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "AudioBuffer channel index is out of range.",
        );
        return None;
    }
    channels
        .get_index(scope, channel)
        .and_then(|value| v8::Local::<v8::Float32Array>::try_from(value).ok())
}

fn metadata<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(channels) = require_channels(scope, args.this()) else {
        return;
    };
    let length = web_audio_number_slot(scope, args.this(), LENGTH).expect("AudioBuffer length");
    let sample_rate =
        web_audio_number_slot(scope, args.this(), SAMPLE_RATE).expect("AudioBuffer sample rate");
    let value = match args.data().int32_value(scope) {
        Some(0) => length,
        Some(1) => length / sample_rate,
        Some(2) => sample_rate,
        Some(3) => channels.length() as f64,
        _ => unreachable!("AudioBuffer metadata field"),
    };
    rv.set(v8::Number::new(scope, value).into());
}

fn get_channel_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(channels) = require_channels(scope, args.this()) else {
        return;
    };
    let Some(parsed) = webidl::parse_args::<GetChannelDataArgs>(scope, &args) else {
        return;
    };
    if let Some(view) = channel_view(scope, channels, parsed.channel) {
        rv.set(view.into());
    }
}

fn copy_from_channel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    copy_channel(scope, &args, true);
}

fn copy_to_channel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    copy_channel(scope, &args, false);
}

fn copy_channel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    from_channel: bool,
) {
    let Some(channels) = require_channels(scope, args.this()) else {
        return;
    };
    if args.length() < 2 {
        throw_type_error(scope, "AudioBuffer channel copy requires two arguments.");
        return;
    }
    let Ok(array) = v8::Local::<v8::Float32Array>::try_from(args.get(0)) else {
        throw_type_error(scope, "AudioBuffer channel copy requires a Float32Array.");
        return;
    };
    if let Some(store) = array.get_backing_store()
        && (store.is_shared() || store.is_resizable_by_user_javascript())
    {
        throw_type_error(
            scope,
            "AudioBuffer channel copy requires a non-shared, fixed-length buffer.",
        );
        return;
    }
    // Numeric coercion can transfer either view's backing buffer. Acquire the
    // intrinsic lengths and stores only after every argument has been converted.
    let Some(parsed) = webidl::parse_args::<CopyChannelArgs>(scope, args) else {
        return;
    };
    if array.length() == 0 {
        return;
    }
    let Some(channel) = channel_view(scope, channels, parsed.channel) else {
        return;
    };
    let offset = parsed.offset as usize;
    let length = array.length().min(channel.length().saturating_sub(offset));
    if length == 0 {
        return;
    }
    let Some(channel_store) = channel.get_backing_store() else {
        return;
    };
    let Some(array_store) = array.get_backing_store() else {
        return;
    };
    let Some(channel_data) = channel_store.data() else {
        return;
    };
    let Some(array_data) = array_store.data() else {
        return;
    };
    // SAFETY: both stores are live and non-shared, and intrinsic view lengths
    // bound these ranges. No JS runs between the bounds check and copy. `copy`
    // (memmove) also handles views overlapping the same channel's backing store.
    unsafe {
        let channel_ptr = channel_data
            .as_ptr()
            .cast::<u8>()
            .add(channel.byte_offset() + offset * size_of::<f32>());
        let array_ptr = array_data.as_ptr().cast::<u8>().add(array.byte_offset());
        let (src, dst) = if from_channel {
            (channel_ptr, array_ptr)
        } else {
            (array_ptr, channel_ptr)
        };
        std::ptr::copy(src, dst, length * size_of::<f32>());
    }
}

pub(super) fn write_channel<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    buffer: v8::Local<'s, v8::Object>,
    channel: u32,
    samples: &[f32],
) -> Option<()> {
    let channels = require_channels(scope, buffer)?;
    let view = channel_view(scope, channels, channel)?;
    let length = samples.len().min(view.length());
    if length == 0 {
        return Some(());
    }
    let store = view.get_backing_store()?;
    let data = store.data()?;
    // SAFETY: newly allocated channels are fixed, non-shared buffers. The source
    // is independent Rust-owned PCM data and length is bounded by both ranges.
    unsafe {
        std::ptr::copy_nonoverlapping(
            samples.as_ptr().cast::<u8>(),
            data.as_ptr().cast::<u8>().add(view.byte_offset()),
            length * size_of::<f32>(),
        );
    }
    Some(())
}
