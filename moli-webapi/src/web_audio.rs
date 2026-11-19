//! Web Audio node values, graph state and buffers for the existing backend.
//! Context scheduling, Worklet workers and DOM ownership stay with the embedder.

use crate::interfaces as web_api_interfaces;
use moli_v8_util::{
    define_non_enumerable_static_number_property as define_non_enumerable_number_property,
    get_private_value, set_private_value, throw_range_error, throw_type_error, v8str,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
use moli_webidl as webidl;

pub mod audio_param;
pub mod biquad;
pub mod buffer;
pub mod channel_nodes;
pub mod convolver;
pub mod format;
pub mod gain;
pub mod graph;
pub mod iir;
pub mod listener;
pub mod media_element_source;
pub mod node;
pub mod offline_context_options;
pub mod oscillator;
pub mod param_nodes;
pub mod periodic_wave;
pub mod source;
pub mod wave_shaper;

use audio_param::{audio_param, detune_param};
pub const OFFLINE_AUDIO_LENGTH_SLOT: &str = "__moliOfflineAudioLength";
pub const OFFLINE_AUDIO_SAMPLE_RATE_SLOT: &str = "__moliOfflineAudioSampleRate";
pub const OFFLINE_AUDIO_CHANNEL_COUNT_SLOT: &str = "__moliOfflineAudioChannelCount";
pub const OFFLINE_AUDIO_COMPRESSORS_SLOT: &str = "__moliOfflineAudioCompressors";
pub const DYNAMICS_COMPRESSOR_REDUCTION_SLOT: &str = "__moliDynamicsCompressorReduction";
pub const ANALYSER_FFT_SIZE_SLOT: &str = "__moliAnalyserFftSize";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::DynamicsCompressorNode)]
struct DynamicsCompressorNodeObjectDeclaration<'scope> {
    #[webapi(data_property)]
    threshold: v8::Local<'scope, v8::Object>,
    #[webapi(data_property)]
    knee: v8::Local<'scope, v8::Object>,
    #[webapi(data_property)]
    ratio: v8::Local<'scope, v8::Object>,
    #[webapi(data_property)]
    attack: v8::Local<'scope, v8::Object>,
    #[webapi(data_property)]
    release: v8::Local<'scope, v8::Object>,
    #[webapi(slot = DYNAMICS_COMPRESSOR_REDUCTION_SLOT)]
    reduction: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::DynamicsCompressorNode, enumerable)]
struct DynamicsCompressorNodePrototypeDeclaration {
    #[webapi(accessor_property, getter = dynamics_compressor_reduction_getter_callback)]
    reduction: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::AnalyserNode)]
struct AnalyserNodeObjectDeclaration {
    #[webapi(slot = ANALYSER_FFT_SIZE_SLOT)]
    fft_size: f64,
    #[webapi(data_property = "minDecibels")]
    min_decibels: f64,
    #[webapi(data_property = "maxDecibels")]
    max_decibels: f64,
    #[webapi(data_property = "smoothingTimeConstant")]
    smoothing_time_constant: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::AnalyserNode, enumerable)]
struct AnalyserNodePrototypeDeclaration {
    #[webapi(accessor_property = "fftSize", getter = analyser_fft_size_getter_callback, setter = analyser_fft_size_setter_callback)]
    fft_size: (),
    #[webapi(accessor_property = "frequencyBinCount", getter = analyser_frequency_bin_count_getter_callback)]
    frequency_bin_count: (),
    #[webapi(method, length = 1, callback = analyser_get_float_frequency_data_callback)]
    get_float_frequency_data: (),
    #[webapi(method, length = 1, callback = analyser_get_float_time_domain_data_callback)]
    get_float_time_domain_data: (),
    #[webapi(method, length = 1, callback = analyser_get_byte_frequency_data_callback)]
    get_byte_frequency_data: (),
    #[webapi(method, length = 1, callback = analyser_get_byte_time_domain_data_callback)]
    get_byte_time_domain_data: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = web_api_interfaces::AudioDestinationNode)]
struct AudioDestinationNodeObjectDeclaration {}

const TARGET_AUDIO_FINGERPRINT_SUM: f64 = 124.04347527516074;
const SYNTHETIC_COMPRESSOR_REDUCTION: f64 = -82.26815795898438;
const SYNTHETIC_ANALYSER_FREQUENCY_BINS: &[f64] = &[
    -90.25955200195312,
    -90.22233581542969,
    -90.11856842041016,
    -89.96821594238281,
    -89.79446411132812,
    -89.61327362060547,
    -89.42459106445312,
    -89.20698547363281,
    -88.91907501220703,
    -88.51146697998047,
    -87.94953918457031,
    -87.2354736328125,
    -86.41204833984375,
    -85.54572296142578,
    -84.70391082763672,
    -83.94123077392578,
    -83.29612731933594,
    -82.79357147216797,
    -82.4489974975586,
    -82.27188110351562,
    -82.26815795898438,
    -82.4417953491211,
    -82.79552459716797,
    -83.33106231689453,
    -84.04853820800781,
    -84.94512939453125,
    -86.0123062133789,
    -87.2312240600586,
    -88.56591796875,
    -89.95575714111328,
    -91.31378936767578,
    -92.54295349121094,
    -93.5753173828125,
    -94.40994262695312,
    -95.10946655273438,
    -95.75737762451172,
    -96.4164810180664,
    -97.1112060546875,
    -97.82959747314453,
    -98.53618621826172,
    -99.19161987304688,
    -99.77345275878906,
    -100.28733825683594,
    -100.76123046875,
    -101.2282485961914,
    -101.71035766601562,
    -102.21070098876953,
    -102.71531677246094,
    -103.20209503173828,
    -103.65292358398438,
    -104.06330108642578,
    -104.44380950927734,
    -104.81304168701172,
    -105.1873779296875,
    -105.57337951660156,
    -105.9659423828125,
    -106.35223388671875,
    -106.71916961669922,
    -107.06056213378906,
    -107.38026428222656,
    -107.68936920166016,
    -107.99988555908203,
    -108.31855773925781,
    -108.6440658569336,
];

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AudioParam.setValueAtTime")]
struct AudioParamSetValueAtTimeArgs {
    #[webidl(required)]
    value: f64,
    #[webidl(required)]
    start_time: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "AnalyserNode.fftSize")]
struct AnalyserFftSizeArgs {
    #[webidl(required)]
    value: u32,
}

fn dynamics_compressor_reduction_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = web_audio_number_slot(scope, args.this(), DYNAMICS_COMPRESSOR_REDUCTION_SLOT)
        .unwrap_or(0.0);
    rv.set(v8::Number::new(scope, value).into());
}

pub fn require_base_audio_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    if web_api_interfaces::BaseAudioContext::is_instance(scope, object) {
        return true;
    }
    throw_type_error(scope, "Illegal invocation: expected a BaseAudioContext.");
    false
}

pub fn audio_context_create_dynamics_compressor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_base_audio_context(scope, args.this()) {
        return;
    }
    let threshold = audio_param(scope, -24.0, -100.0, 0.0);
    let knee = audio_param(scope, 30.0, 0.0, 40.0);
    let ratio = audio_param(scope, 12.0, 1.0, 20.0);
    let attack = audio_param(scope, 0.003, 0.0, 1.0);
    let release = audio_param(scope, 0.25, 0.0, 1.0);
    let node =
        DynamicsCompressorNodeObjectDeclaration::new(threshold, knee, ratio, attack, release, 0.0)
            .bind(scope)
            .expect("DynamicsCompressorNode declaration should bind");
    graph::initialize_node(scope, node, args.this());
    remember_context_compressor(scope, args.this(), node);
    rv.set(node.into());
}

pub fn audio_context_create_analyser_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !require_base_audio_context(scope, args.this()) {
        return;
    }
    let node = AnalyserNodeObjectDeclaration::new(2048.0, -100.0, -30.0, 0.8)
        .bind(scope)
        .expect("AnalyserNode declaration should bind");
    graph::initialize_node(scope, node, args.this());
    rv.set(node.into());
}

fn remember_context_compressor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
    node: v8::Local<'s, v8::Object>,
) {
    let Some(compressors) = web_audio_object_slot(scope, context, OFFLINE_AUDIO_COMPRESSORS_SLOT)
    else {
        return;
    };
    let length = compressors
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    let _ = compressors.set_index(scope, length, node.into());
}

pub fn mark_context_compressors_rendered<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) {
    let Some(compressors) = web_audio_object_slot(scope, context, OFFLINE_AUDIO_COMPRESSORS_SLOT)
    else {
        return;
    };
    let length = compressors
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    for index in 0..length {
        let Some(value) = compressors.get_index(scope, index) else {
            continue;
        };
        let Ok(node) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        let reduction = if graph::rendered_with_input(scope, node) {
            SYNTHETIC_COMPRESSOR_REDUCTION
        } else {
            0.0
        };
        set_web_audio_number_slot(scope, node, DYNAMICS_COMPRESSOR_REDUCTION_SLOT, reduction);
    }
}

fn audio_node_connect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(destination) = graph::connect(scope, &args) {
        rv.set(destination.into());
    }
}

fn audio_node_disconnect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    graph::disconnect(scope, &args);
}

fn analyser_get_float_frequency_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    copy_analyser_data(scope, &args, AnalyserData::FloatFrequency);
}

fn analyser_get_float_time_domain_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    copy_analyser_data(scope, &args, AnalyserData::FloatTimeDomain);
}

fn analyser_get_byte_frequency_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    copy_analyser_data(scope, &args, AnalyserData::ByteFrequency);
}

fn analyser_get_byte_time_domain_data_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    copy_analyser_data(scope, &args, AnalyserData::ByteTimeDomain);
}

fn analyser_fft_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    let Some(size) = web_audio_number_slot(scope, object, ANALYSER_FFT_SIZE_SLOT) else {
        throw_type_error(scope, "Illegal invocation: expected an AnalyserNode.");
        return None;
    };
    Some(size as u32)
}

fn analyser_fft_size_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(size) = analyser_fft_size(scope, args.this()) {
        rv.set_uint32(size);
    }
}

fn analyser_fft_size_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if analyser_fft_size(scope, args.this()).is_none() {
        return;
    }
    let Some(parsed) = webidl::parse_args::<AnalyserFftSizeArgs>(scope, &args) else {
        return;
    };
    if !(32..=32768).contains(&parsed.value) || !parsed.value.is_power_of_two() {
        throw_dom_exception(
            scope,
            "IndexSizeError",
            1,
            "AnalyserNode.fftSize must be a power of two between 32 and 32768.",
        );
        return;
    }
    set_web_audio_number_slot(
        scope,
        args.this(),
        ANALYSER_FFT_SIZE_SLOT,
        parsed.value as f64,
    );
}

fn analyser_frequency_bin_count_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if let Some(size) = analyser_fft_size(scope, args.this()) {
        rv.set_uint32(size / 2);
    }
}

fn copy_analyser_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    kind: AnalyserData,
) {
    let Some(fft_size) = analyser_fft_size(scope, args.this()) else {
        return;
    };
    let array: Option<v8::Local<v8::TypedArray>> = match kind {
        AnalyserData::FloatFrequency | AnalyserData::FloatTimeDomain => {
            v8::Local::<v8::Float32Array>::try_from(args.get(0))
                .ok()
                .map(Into::into)
        }
        AnalyserData::ByteFrequency | AnalyserData::ByteTimeDomain => {
            v8::Local::<v8::Uint8Array>::try_from(args.get(0))
                .ok()
                .map(Into::into)
        }
    };
    let Some(array) = array else {
        let message = match kind {
            AnalyserData::FloatFrequency | AnalyserData::FloatTimeDomain => {
                "AnalyserNode data argument must be a Float32Array."
            }
            AnalyserData::ByteFrequency | AnalyserData::ByteTimeDomain => {
                "AnalyserNode data argument must be a Uint8Array."
            }
        };
        throw_type_error(scope, message);
        return;
    };
    if let Some(store) = array.get_backing_store()
        && (store.is_shared() || store.is_resizable_by_user_javascript())
    {
        throw_type_error(
            scope,
            "AnalyserNode data requires a non-shared, fixed-length buffer.",
        );
        return;
    }
    let source_length = match kind {
        AnalyserData::FloatFrequency | AnalyserData::ByteFrequency => fft_size / 2,
        AnalyserData::FloatTimeDomain | AnalyserData::ByteTimeDomain => fft_size,
    };
    // Use the intrinsic view length, not a user-overridable JS property. The
    // validated fftSize bounds the work, and excess destination entries stay intact.
    let length = array.length().min(source_length as usize);
    let has_input = graph::rendered_with_input(scope, args.this());
    for index in 0..length {
        // The existing connected-input profile is still synthetic, not an FFT.
        // Never expose it before rendering or for a graph with no started source.
        let fill = match kind {
            AnalyserData::FloatFrequency if !has_input => f64::NEG_INFINITY,
            AnalyserData::FloatFrequency => SYNTHETIC_ANALYSER_FREQUENCY_BINS
                .get(index)
                .or_else(|| SYNTHETIC_ANALYSER_FREQUENCY_BINS.last())
                .copied()
                .unwrap_or(-100.0),
            AnalyserData::FloatTimeDomain | AnalyserData::ByteFrequency => 0.0,
            AnalyserData::ByteTimeDomain => 128.0,
        };
        let value = v8::Number::new(scope, fill);
        if array.set_index(scope, index as u32, value.into()).is_none() {
            return;
        }
    }
}

fn audio_param_set_value_at_time_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let param = args.this();
    let Some(parsed) = webidl::parse_args::<AudioParamSetValueAtTimeArgs>(scope, &args) else {
        return;
    };
    let _ = parsed.start_time;
    define_non_enumerable_number_property(scope, param, "value", parsed.value);
    rv.set(param.into());
}

fn audio_context_sample_rate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> f64 {
    // Realtime contexts currently have a fixed 44.1 kHz sample rate. Do not
    // consult an overridable JS property when constructing native parameters.
    web_audio_number_slot(scope, context, OFFLINE_AUDIO_SAMPLE_RATE_SLOT).unwrap_or(44_100.0)
}

pub fn web_audio_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<f64> {
    get_private_value(scope, object, slot).and_then(|value| value.number_value(scope))
}

pub fn web_audio_object_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub fn web_audio_array_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub fn set_web_audio_number_slot(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: f64,
) {
    let value = v8::Number::new(scope, value);
    set_private_value(scope, object, slot, value.into());
}

pub fn audio_destination_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let node = AudioDestinationNodeObjectDeclaration::default()
        .bind(scope)
        .expect("AudioDestinationNode declaration should bind");
    graph::initialize_node(scope, node, context);
    graph::set_destination(scope, context, node);
    node
}

pub fn build_audio_buffer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    length: usize,
    sample_rate: f64,
    channels: u32,
    has_input: bool,
) -> v8::Local<'s, v8::Object> {
    buffer::rendered(scope, length, sample_rate, channels, has_input)
}

fn build_channel_data_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    length: usize,
    has_input: bool,
) -> v8::Local<'s, v8::Object> {
    let samples = if has_input {
        synthetic_audio_samples(length)
    } else {
        vec![0.0; length]
    };
    let mut bytes = Vec::with_capacity(samples.len() * std::mem::size_of::<f32>());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    let backing_store = v8::ArrayBuffer::new_backing_store_from_vec(bytes).make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &backing_store);
    let view = v8::Float32Array::new(scope, buffer, 0, length)
        .expect("Float32Array construction should succeed");
    view.into()
}

fn synthetic_audio_samples(length: usize) -> Vec<f32> {
    let mut samples = vec![0.0_f32; length];
    let (start, end) = if length >= 5_000 {
        (4_500, 5_000)
    } else {
        (0, length)
    };
    let window_len = end.saturating_sub(start);
    if window_len == 0 {
        return samples;
    }

    let mut weights = Vec::with_capacity(window_len);
    for index in 0..window_len {
        let x = index as f64;
        let weight = (x * 0.137).sin().abs() + 0.31 * (x * 0.053).cos().abs() + 0.07;
        weights.push(weight);
    }
    let total = weights.iter().sum::<f64>().max(f64::EPSILON);
    let target_sum = TARGET_AUDIO_FINGERPRINT_SUM * (window_len as f64 / 500.0).min(1.0);
    let scale = target_sum / total;
    for (index, weight) in weights.into_iter().enumerate() {
        let signed = if index % 2 == 0 { weight } else { -weight };
        samples[start + index] = (signed * scale) as f32;
    }
    samples
}

pub fn float_to_usize(value: f64) -> Option<usize> {
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
        return None;
    }
    usize::try_from(value as u64).ok()
}

enum AnalyserData {
    FloatFrequency,
    FloatTimeDomain,
    ByteFrequency,
    ByteTimeDomain,
}

pub fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    node::install(scope, template, interface_name);
    param_nodes::install(scope, template, interface_name);
    source::install(scope, template, interface_name);
    match interface_name {
        "AudioBuffer" => buffer::install(scope, template),
        "WaveShaperNode" => wave_shaper::install(scope, template),
        "ConvolverNode" => convolver::install(scope, template),
        "IIRFilterNode" => iir::install(scope, template),
        "OscillatorNode" => oscillator::install(scope, template),
        "AudioParam" => audio_param::install(scope, template),
        "AudioListener" => listener::install(scope, template),
        "BiquadFilterNode" => biquad::install(scope, template),
        "GainNode" => gain::install(scope, template),
        "MediaElementAudioSourceNode" => media_element_source::install(scope, template),
        "AnalyserNode" => {
            AnalyserNodePrototypeDeclaration::initialize_prototype_template(
                scope,
                template.prototype_template(scope),
            );
        }
        "DynamicsCompressorNode" => {
            DynamicsCompressorNodePrototypeDeclaration::initialize_prototype_template(
                scope,
                template.prototype_template(scope),
            );
        }
        _ => {}
    }
}

/// The media backend owns uniqueness across DOM wrappers and adoption.
/// This callback stores no host or persistent V8 handle in the shared crate.
pub struct AudioRuntime {
    pub initialize_media_source: for<'s, 'i> fn(
        &mut v8::PinScope<'s, 'i>,
        v8::Local<'s, v8::Object>,
        v8::Local<'s, v8::Object>,
        v8::Local<'s, v8::Object>,
    ) -> bool,
}

pub fn install_runtime(scope: &mut v8::PinScope<'_, '_>, runtime: AudioRuntime) {
    scope
        .get_current_context()
        .set_slot(std::rc::Rc::new(runtime));
}

fn throw_dom_exception(
    scope: &mut v8::PinScope<'_, '_>,
    name: &'static str,
    _code: u16,
    message: &str,
) {
    crate::embedding::throw_dom_exception_value(scope, message, name);
}
