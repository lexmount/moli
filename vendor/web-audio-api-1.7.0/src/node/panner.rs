use std::any::Any;
use std::collections::HashMap;
use std::f32::consts::PI;
use std::sync::{Mutex, OnceLock};

use float_eq::float_eq;
use hrtf::{HrirSphere, HrtfContext, HrtfProcessor, Vec3};

use crate::context::{AudioContextRegistration, AudioParamId, BaseAudioContext};
use crate::param::{AudioParam, AudioParamDescriptor};
use crate::render::{
    AudioParamValues, AudioProcessor, AudioRenderQuantum, AudioWorkletGlobalScope,
};
use crate::RENDER_QUANTUM_SIZE;

use super::{AudioNode, AudioNodeOptions, ChannelConfig, ChannelCountMode, ChannelInterpretation};

/// Assert that the given value number is a valid value for coneOuterGain
///
/// # Panics
///
/// This function will panic if:
/// - the given value is not finite and lower than zero
#[track_caller]
#[inline(always)]
#[allow(clippy::manual_range_contains)]
pub(crate) fn assert_valid_cone_outer_gain(value: f64) {
    assert!(
        value >= 0. && value <= 1.,
        "InvalidStateError - coneOuterGain must be in the range [0, 1]"
    );
}

/// Load the HRTF processor for the given sample_rate
///
/// The included data contains the impulse responses at 44100 Hertz, so it needs to be resampled
/// for other values (which can easily take 100s of milliseconds). Therefore cache the result (per
/// sample rate) in a global variable and clone it every time a new panner is created.
pub(crate) fn load_hrtf_processor(sample_rate: u32) -> (HrtfProcessor, usize) {
    static INSTANCE: OnceLock<Mutex<HashMap<u32, (HrtfProcessor, usize)>>> = OnceLock::new();
    let cache = INSTANCE.get_or_init(|| Mutex::new(HashMap::new()));

    // To avoid poisening the cache mutex, don't use the `entry()` API on HashMap
    {
        if let Some(value) = cache.lock().unwrap().get(&sample_rate) {
            return value.clone();
        }
    }

    // The following snippet might panic
    let resource = include_bytes!("../../resources/IRC_1003_C.bin");
    let hrir_sphere = HrirSphere::new(&resource[..], sample_rate).unwrap();
    let len = hrir_sphere.len();

    let interpolation_steps = 1; // TODO?
    let samples_per_step = RENDER_QUANTUM_SIZE / interpolation_steps;
    let processor = HrtfProcessor::new(hrir_sphere, interpolation_steps, samples_per_step);

    let value = (processor, len);
    cache.lock().unwrap().insert(sample_rate, value.clone());

    value
}

/// Spatialization algorithm used to position the audio in 3D space
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum PanningModelType {
    #[default]
    EqualPower,
    HRTF,
}

impl From<u8> for PanningModelType {
    fn from(i: u8) -> Self {
        match i {
            0 => PanningModelType::EqualPower,
            1 => PanningModelType::HRTF,
            _ => unreachable!(),
        }
    }
}

/// Algorithm to reduce the volume of an audio source as it moves away from the listener
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum DistanceModelType {
    Linear,
    #[default]
    Inverse,
    Exponential,
}

impl From<u8> for DistanceModelType {
    fn from(i: u8) -> Self {
        match i {
            0 => DistanceModelType::Linear,
            1 => DistanceModelType::Inverse,
            2 => DistanceModelType::Exponential,
            _ => unreachable!(),
        }
    }
}

/// Options for constructing a [`PannerNode`]
// dictionary PannerOptions : AudioNodeOptions {
//   PanningModelType panningModel = "equalpower";
//   DistanceModelType distanceModel = "inverse";
//   float positionX = 0;
//   float positionY = 0;
//   float positionZ = 0;
//   float orientationX = 1;
//   float orientationY = 0;
//   float orientationZ = 0;
//   double refDistance = 1;
//   double maxDistance = 10000;
//   double rolloffFactor = 1;
//   double coneInnerAngle = 360;
//   double coneOuterAngle = 360;
//   double coneOuterGain = 0;
// };
#[derive(Clone, Debug)]
pub struct PannerOptions {
    pub panning_model: PanningModelType,
    pub distance_model: DistanceModelType,
    pub position_x: f32,
    pub position_y: f32,
    pub position_z: f32,
    pub orientation_x: f32,
    pub orientation_y: f32,
    pub orientation_z: f32,
    pub ref_distance: f64,
    pub max_distance: f64,
    pub rolloff_factor: f64,
    pub cone_inner_angle: f64,
    pub cone_outer_angle: f64,
    pub cone_outer_gain: f64,
    pub audio_node_options: AudioNodeOptions,
}

impl Default for PannerOptions {
    fn default() -> Self {
        PannerOptions {
            panning_model: PanningModelType::default(),
            distance_model: DistanceModelType::default(),
            position_x: 0.,
            position_y: 0.,
            position_z: 0.,
            orientation_x: 1.,
            orientation_y: 0.,
            orientation_z: 0.,
            ref_distance: 1.,
            max_distance: 10000.,
            rolloff_factor: 1.,
            cone_inner_angle: 360.,
            cone_outer_angle: 360.,
            cone_outer_gain: 0.,
            audio_node_options: AudioNodeOptions {
                channel_count: 2,
                channel_count_mode: ChannelCountMode::ClampedMax,
                channel_interpretation: ChannelInterpretation::Speakers,
            },
        }
    }
}

enum ControlMessage {
    DistanceModel(DistanceModelType),
    // Box this payload - one large variant can penalize the memory layout of this enum
    PanningModel(Box<Option<HrtfState>>),
    RefDistance(f64),
    MaxDistance(f64),
    RollOffFactor(f64),
    ConeInnerAngle(f64),
    ConeOuterAngle(f64),
    ConeOuterGain(f64),
}

/// Assert that the channel count is valid for the PannerNode
/// see <https://webaudio.github.io/web-audio-api/#audionode-channelcount-constraints>
///
/// # Panics
///
/// This function panics if given count is greater than 2
///
#[track_caller]
#[inline(always)]
fn assert_valid_channel_count(count: usize) {
    assert!(
        count <= 2,
        "NotSupportedError - PannerNode channel count cannot be greater than two"
    );
}

/// Assert that the channel count is valid for the PannerNode
/// see <https://webaudio.github.io/web-audio-api/#audionode-channelcountmode-constraints>
///
/// # Panics
///
/// This function panics if given count mode is [`ChannelCountMode::Max`]
///
#[track_caller]
#[inline(always)]
fn assert_valid_channel_count_mode(mode: ChannelCountMode) {
    assert_ne!(
        mode,
        ChannelCountMode::Max,
        "NotSupportedError - PannerNode channel count mode cannot be set to max"
    );
}

/// Internal state of the HRTF renderer
struct HrtfState {
    len: usize,
    processor: HrtfProcessor,
    output_interleaved: Vec<(f32, f32)>,
    prev_sample_vector: Vec3,
    prev_left_samples: Vec<f32>,
    prev_right_samples: Vec<f32>,
    prev_distance_gain: f32,
}

impl HrtfState {
    fn new(processor: HrtfProcessor, len: usize) -> Self {
        Self {
            len,
            processor,
            output_interleaved: vec![(0., 0.); RENDER_QUANTUM_SIZE],
            prev_sample_vector: Vec3::new(0., 0., 1.),
            prev_left_samples: vec![],  // will resize accordingly
            prev_right_samples: vec![], // will resize accordingly
            prev_distance_gain: 0.,
        }
    }

    fn process(
        &mut self,
        source: &[f32],
        new_distance_gain: f32,
        projected_source: [f32; 3],
    ) -> &[(f32, f32)] {
        // reset state of output buffer
        self.output_interleaved.fill((0., 0.));

        let new_sample_vector = Vec3 {
            x: projected_source[0],
            z: projected_source[1],
            y: projected_source[2],
        };

        let context = HrtfContext {
            source,
            output: &mut self.output_interleaved,
            new_sample_vector,
            prev_sample_vector: self.prev_sample_vector,
            prev_left_samples: &mut self.prev_left_samples,
            prev_right_samples: &mut self.prev_right_samples,
            new_distance_gain,
            prev_distance_gain: self.prev_distance_gain,
        };

        self.processor.process_samples(context);

        self.prev_sample_vector = new_sample_vector;
        self.prev_distance_gain = new_distance_gain;

        &self.output_interleaved
    }

    fn tail_time_samples(&self) -> usize {
        self.len
    }
}

/// `PannerNode` positions / spatializes an incoming audio stream in three-dimensional space.
///
/// - MDN documentation: <https://developer.mozilla.org/en-US/docs/Web/API/PannerNode>
/// - specification: <https://www.w3.org/TR/webaudio/#pannernode> and
///   <https://www.w3.org/TR/webaudio/#Spatialization>
/// - see also: [`BaseAudioContext::create_panner`]
///
/// # Usage
/// ```no_run
/// use web_audio_api::context::{BaseAudioContext, AudioContext};
/// use web_audio_api::node::AudioNode;
/// use web_audio_api::node::AudioScheduledSourceNode;
///
/// // Setup a new audio context
/// let context = AudioContext::default();
///
/// // Create a friendly tone
/// let mut tone = context.create_oscillator();
/// tone.frequency().set_value_at_time(300.0f32, 0.);
/// tone.start();
///
/// // Connect tone > panner node > destination node
/// let panner = context.create_panner();
/// tone.connect(&panner);
/// panner.connect(&context.destination());
///
/// // The panner node is 1 unit in front of listener
/// panner.position_z().set_value_at_time(1., 0.);
///
/// // And sweeps 10 units left to right, every second
/// let mut moving = context.create_oscillator();
/// moving.start();
/// moving.frequency().set_value_at_time(1., 0.);
/// let gain = context.create_gain();
/// gain.gain().set_value_at_time(10., 0.);
/// moving.connect(&gain);
/// gain.connect(panner.position_x());
///
/// // enjoy listening
/// std::thread::sleep(std::time::Duration::from_secs(4));
/// ```
///
/// # Examples
///
/// - `cargo run --release --example spatial`
/// - `cargo run --release --example panner_cone`
#[derive(Debug)]
pub struct PannerNode {
    registration: AudioContextRegistration,
    channel_config: ChannelConfig,
    position_x: AudioParam,
    position_y: AudioParam,
    position_z: AudioParam,
    orientation_x: AudioParam,
    orientation_y: AudioParam,
    orientation_z: AudioParam,
    cone_inner_angle: f64,
    cone_outer_angle: f64,
    cone_outer_gain: f64,
    distance_model: DistanceModelType,
    ref_distance: f64,
    max_distance: f64,
    rolloff_factor: f64,
    panning_model: PanningModelType,
}

impl AudioNode for PannerNode {
    fn registration(&self) -> &AudioContextRegistration {
        &self.registration
    }

    fn channel_config(&self) -> &ChannelConfig {
        &self.channel_config
    }

    fn number_of_inputs(&self) -> usize {
        1
    }

    fn number_of_outputs(&self) -> usize {
        1
    }

    // same limitations as for the StereoPannerNode
    // see: https://webaudio.github.io/web-audio-api/#panner-channel-limitations
    fn set_channel_count(&self, count: usize) {
        assert_valid_channel_count(count);
        self.channel_config.set_count(count, self.registration());
    }

    fn set_channel_count_mode(&self, mode: ChannelCountMode) {
        assert_valid_channel_count_mode(mode);
        self.channel_config
            .set_count_mode(mode, self.registration());
    }
}

impl PannerNode {
    /// returns a `PannerNode` instance
    ///
    /// # Arguments
    ///
    /// * `context` - audio context in which the audio node will live.
    /// * `options` - stereo panner options
    ///
    /// # Panics
    ///
    /// Will panic if:
    ///
    /// * `options.channel_config.count` is greater than 2
    /// * `options.channel_config.mode` is `ChannelCountMode::Max`
    ///
    /// Can panic when loading HRIR-sphere
    #[allow(clippy::missing_panics_doc)]
    pub fn new<C: BaseAudioContext>(context: &C, options: PannerOptions) -> Self {
        let mut node = context.base().register(|registration| {
            use crate::spatial::PARAM_OPTS;

            let PannerOptions {
                position_x,
                position_y,
                position_z,
                orientation_x,
                orientation_y,
                orientation_z,
                distance_model,
                ref_distance,
                max_distance,
                rolloff_factor,
                cone_inner_angle,
                cone_outer_angle,
                cone_outer_gain,
                audio_node_options: channel_config,
                panning_model,
            } = options;

            assert!(
                ref_distance >= 0.,
                "RangeError - refDistance cannot be negative"
            );
            assert!(
                max_distance > 0.,
                "RangeError - maxDistance must be strictly positive"
            );
            assert!(
                rolloff_factor >= 0.,
                "RangeError - rolloffFactor cannot be negative"
            );
            assert_valid_cone_outer_gain(cone_outer_gain);
            assert_valid_channel_count(channel_config.channel_count);
            assert_valid_channel_count_mode(channel_config.channel_count_mode);

            // position params
            let (param_px, render_px) = context.create_audio_param(PARAM_OPTS, &registration);
            let (param_py, render_py) = context.create_audio_param(PARAM_OPTS, &registration);
            let (param_pz, render_pz) = context.create_audio_param(PARAM_OPTS, &registration);
            param_px.set_value(position_x);
            param_py.set_value(position_y);
            param_pz.set_value(position_z);

            // orientation params
            let orientation_x_opts = AudioParamDescriptor {
                default_value: 1.0,
                ..PARAM_OPTS
            };
            let (param_ox, render_ox) =
                context.create_audio_param(orientation_x_opts, &registration);
            let (param_oy, render_oy) = context.create_audio_param(PARAM_OPTS, &registration);
            let (param_oz, render_oz) = context.create_audio_param(PARAM_OPTS, &registration);
            param_ox.set_value(orientation_x);
            param_oy.set_value(orientation_y);
            param_oz.set_value(orientation_z);

            let render = PannerRenderer {
                position_x: render_px,
                position_y: render_py,
                position_z: render_pz,
                orientation_x: render_ox,
                orientation_y: render_oy,
                orientation_z: render_oz,
                distance_model,
                ref_distance,
                max_distance,
                rolloff_factor,
                cone_inner_angle,
                cone_outer_angle,
                cone_outer_gain,
                hrtf_state: None,
                tail_time_counter: 0,
            };

            let node = PannerNode {
                registration,
                channel_config: channel_config.into(),
                position_x: param_px,
                position_y: param_py,
                position_z: param_pz,
                orientation_x: param_ox,
                orientation_y: param_oy,
                orientation_z: param_oz,
                distance_model,
                ref_distance,
                max_distance,
                rolloff_factor,
                cone_inner_angle,
                cone_outer_angle,
                cone_outer_gain,
                panning_model,
            };

            // instruct to BaseContext to add the AudioListener if it has not already
            context.base().ensure_audio_listener_present();

            (node, Box::new(render))
        });

        // after the node is registered, connect the AudioListener
        context
            .base()
            .connect_listener_to_panner(node.registration().id());

        // load the HRTF sphere if requested
        node.set_panning_model(options.panning_model);

        node
    }

    pub fn position_x(&self) -> &AudioParam {
        &self.position_x
    }

    pub fn position_y(&self) -> &AudioParam {
        &self.position_y
    }

    pub fn position_z(&self) -> &AudioParam {
        &self.position_z
    }

    pub fn set_position(&self, x: f32, y: f32, z: f32) {
        self.position_x.set_value(x);
        self.position_y.set_value(y);
        self.position_z.set_value(z);
    }

    pub fn orientation_x(&self) -> &AudioParam {
        &self.orientation_x
    }

    pub fn orientation_y(&self) -> &AudioParam {
        &self.orientation_y
    }

    pub fn orientation_z(&self) -> &AudioParam {
        &self.orientation_z
    }

    pub fn set_orientation(&self, x: f32, y: f32, z: f32) {
        self.orientation_x.set_value(x);
        self.orientation_y.set_value(y);
        self.orientation_z.set_value(z);
    }

    pub fn distance_model(&self) -> DistanceModelType {
        self.distance_model
    }

    pub fn set_distance_model(&mut self, value: DistanceModelType) {
        self.distance_model = value;
        self.registration
            .post_message(ControlMessage::DistanceModel(value));
    }

    pub fn ref_distance(&self) -> f64 {
        self.ref_distance
    }

    /// Set the refDistance attribute
    ///
    /// # Panics
    ///
    /// Panics if the provided value is negative.
    pub fn set_ref_distance(&mut self, value: f64) {
        assert!(value >= 0., "RangeError - refDistance cannot be negative");
        self.ref_distance = value;
        self.registration
            .post_message(ControlMessage::RefDistance(value));
    }

    pub fn max_distance(&self) -> f64 {
        self.max_distance
    }

    /// Set the maxDistance attribute
    ///
    /// # Panics
    ///
    /// Panics if the provided value is negative.
    pub fn set_max_distance(&mut self, value: f64) {
        assert!(
            value > 0.,
            "RangeError - maxDistance must be strictly positive"
        );
        self.max_distance = value;
        self.registration
            .post_message(ControlMessage::MaxDistance(value));
    }

    pub fn rolloff_factor(&self) -> f64 {
        self.rolloff_factor
    }

    /// Set the rolloffFactor attribute
    ///
    /// # Panics
    ///
    /// Panics if the provided value is negative.
    pub fn set_rolloff_factor(&mut self, value: f64) {
        assert!(value >= 0., "RangeError - rolloffFactor cannot be negative");
        self.rolloff_factor = value;
        self.registration
            .post_message(ControlMessage::RollOffFactor(value));
    }

    pub fn cone_inner_angle(&self) -> f64 {
        self.cone_inner_angle
    }

    pub fn set_cone_inner_angle(&mut self, value: f64) {
        self.cone_inner_angle = value;
        self.registration
            .post_message(ControlMessage::ConeInnerAngle(value));
    }

    pub fn cone_outer_angle(&self) -> f64 {
        self.cone_outer_angle
    }

    pub fn set_cone_outer_angle(&mut self, value: f64) {
        self.cone_outer_angle = value;
        self.registration
            .post_message(ControlMessage::ConeOuterAngle(value));
    }

    pub fn cone_outer_gain(&self) -> f64 {
        self.cone_outer_gain
    }

    /// Set the coneOuterGain attribute
    ///
    /// # Panics
    ///
    /// Panics if the provided value is not in the range [0, 1]
    pub fn set_cone_outer_gain(&mut self, value: f64) {
        assert_valid_cone_outer_gain(value);
        self.cone_outer_gain = value;
        self.registration
            .post_message(ControlMessage::ConeOuterGain(value));
    }

    pub fn panning_model(&self) -> PanningModelType {
        self.panning_model
    }

    #[allow(clippy::missing_panics_doc)] // loading the provided HRTF will not panic
    pub fn set_panning_model(&mut self, value: PanningModelType) {
        let hrtf_option = match value {
            PanningModelType::EqualPower => None,
            PanningModelType::HRTF => {
                let sample_rate = self.context().sample_rate() as u32;
                let (processor, len) = load_hrtf_processor(sample_rate);
                Some(HrtfState::new(processor, len))
            }
        };

        self.panning_model = value;
        self.registration
            .post_message(ControlMessage::PanningModel(Box::new(hrtf_option)));
    }
}

#[derive(Copy, Clone)]
struct SpatialParams {
    dist_gain: f32,
    cone_gain: f32,
    azimuth: f32,
    elevation: f32,
}

struct PannerRenderer {
    position_x: AudioParamId,
    position_y: AudioParamId,
    position_z: AudioParamId,
    orientation_x: AudioParamId,
    orientation_y: AudioParamId,
    orientation_z: AudioParamId,
    distance_model: DistanceModelType,
    ref_distance: f64,
    max_distance: f64,
    rolloff_factor: f64,
    cone_inner_angle: f64,
    cone_outer_angle: f64,
    cone_outer_gain: f64,
    hrtf_state: Option<HrtfState>, // use EqualPower panning model if `None`
    tail_time_counter: usize,
}

impl AudioProcessor for PannerRenderer {
    fn process(
        &mut self,
        inputs: &[AudioRenderQuantum],
        outputs: &mut [AudioRenderQuantum],
        params: AudioParamValues<'_>,
        _scope: &AudioWorkletGlobalScope,
    ) -> bool {
        // Single input/output node
        let input = &inputs[0];
        let output = &mut outputs[0];

        // early exit for silence
        if input.is_silent() {
            // HRTF panner has tail time equal to the max length of the impulse response buffers
            // (12 ms)
            let tail_time = match &self.hrtf_state {
                None => false,
                Some(hrtf_state) => hrtf_state.tail_time_samples() > self.tail_time_counter,
            };
            if !tail_time {
                output.make_silent();
                return false;
            }

            self.tail_time_counter += RENDER_QUANTUM_SIZE;
        }

        // for borrow reasons, take the hrtf_state out of self
        let mut hrtf_state = self.hrtf_state.take();

        // source parameters (Panner)
        let source_position_x = params.get(&self.position_x);
        let source_position_y = params.get(&self.position_y);
        let source_position_z = params.get(&self.position_z);
        let source_orientation_x = params.get(&self.orientation_x);
        let source_orientation_y = params.get(&self.orientation_y);
        let source_orientation_z = params.get(&self.orientation_z);

        // listener parameters (AudioListener)
        let [listener_position_x, listener_position_y, listener_position_z, listener_forward_x, listener_forward_y, listener_forward_z, listener_up_x, listener_up_y, listener_up_z] =
            params.listener_params();

        // build up the a-rate iterator for spatial variables
        let mut a_rate_params = source_position_x
            .iter()
            .cycle()
            .zip(source_position_y.iter().cycle())
            .zip(source_position_z.iter().cycle())
            .zip(source_orientation_x.iter().cycle())
            .zip(source_orientation_y.iter().cycle())
            .zip(source_orientation_z.iter().cycle())
            .zip(listener_position_x.iter().cycle())
            .zip(listener_position_y.iter().cycle())
            .zip(listener_position_z.iter().cycle())
            .zip(listener_forward_x.iter().cycle())
            .zip(listener_forward_y.iter().cycle())
            .zip(listener_forward_z.iter().cycle())
            .zip(listener_up_x.iter().cycle())
            .zip(listener_up_y.iter().cycle())
            .zip(listener_up_z.iter().cycle())
            .map(|tuple| {
                // unpack giant tuple
                let ((((((sp_so_lp, lfx), lfy), lfz), lux), luy), luz) = tuple;
                let (((sp_so, lpx), lpy), lpz) = sp_so_lp;
                let (((sp, sox), soy), soz) = sp_so;
                let ((spx, spy), spz) = sp;

                // define base vectors in 3D
                let source_position = [*spx, *spy, *spz];
                let source_orientation = [*sox, *soy, *soz];
                let listener_position = [*lpx, *lpy, *lpz];
                let listener_forward = [*lfx, *lfy, *lfz];
                let listener_up = [*lux, *luy, *luz];

                // determine distance and cone gain
                let dist_gain = self.dist_gain(source_position, listener_position);
                let cone_gain =
                    self.cone_gain(source_position, source_orientation, listener_position);

                // azimuth and elevation of the panner in frame of reference of the listener
                let (azimuth, elevation) = crate::spatial::azimuth_and_elevation(
                    source_position,
                    listener_position,
                    listener_forward,
                    listener_up,
                );

                SpatialParams {
                    dist_gain,
                    cone_gain,
                    azimuth,
                    elevation,
                }
            });

        if let Some(hrtf_state) = &mut hrtf_state {
            // HRTF panning - always k-rate so take a single value from the a-rate iter
            let SpatialParams {
                dist_gain,
                cone_gain,
                azimuth,
                elevation,
            } = a_rate_params.next().unwrap();

            let new_distance_gain = cone_gain * dist_gain;

            // convert az/el to cartesian coordinates to determine unit direction
            let az_rad = azimuth * PI / 180.;
            let el_rad = elevation * PI / 180.;
            let x = az_rad.sin() * el_rad.cos();
            let z = az_rad.cos() * el_rad.cos();
            let y = el_rad.sin();
            let mut projected_source = [x, y, z];

            if float_eq!(&projected_source[..], &[0.; 3][..], abs_all <= 1E-6) {
                projected_source = [0., 0., 1.];
            }

            // Currently, only mono-to-stereo panning is supported (todo issue #241).
            // Stereo-to-stereo is typically implemented by using 2 HRTF-kernels, feeding each
            // channels into their respective kernel, and summing the result per ear.  This will
            // usually double the output volume as compared to mono-to-stereo.  Hence we double
            // the input signal for stereo inputs to correct for our lack of implementation.
            *output = input.clone();
            let mut overall_gain_correction = 1.;
            if output.number_of_channels() == 2 {
                overall_gain_correction *= 2.; // stereo-to-stereo panning typically doubles volume
                output.mix(1, ChannelInterpretation::Speakers);
            }

            let output_interleaved =
                hrtf_state.process(output.channel_data(0), new_distance_gain, projected_source);

            output.set_number_of_channels(2);
            let [left, right] = output.stereo_mut();

            output_interleaved
                .iter()
                .zip(&mut left[..])
                .zip(&mut right[..])
                .for_each(|((p, l), r)| {
                    *l = overall_gain_correction * p.0;
                    *r = overall_gain_correction * p.1;
                });
        } else {
            // EqualPower panning

            // Optimize for static Panner & Listener
            let single_valued = source_position_x.len() == 1
                && source_position_y.len() == 1
                && source_position_z.len() == 1
                && source_orientation_x.len() == 1
                && source_orientation_y.len() == 1
                && source_orientation_z.len() == 1
                && listener_position_x.len() == 1
                && listener_position_y.len() == 1
                && listener_position_z.len() == 1
                && listener_forward_x.len() == 1
                && listener_forward_y.len() == 1
                && listener_forward_z.len() == 1
                && listener_up_x.len() == 1
                && listener_up_y.len() == 1
                && listener_up_z.len() == 1;

            if single_valued {
                let param_value = a_rate_params.next().unwrap();
                match input.number_of_channels() {
                    1 => {
                        *output = input.clone();
                        output.mix(2, ChannelInterpretation::Speakers);
                        let [left, right] = output.stereo_mut();
                        left.iter_mut()
                            .zip(&mut right[..])
                            .for_each(|(l, r)| apply_mono_to_stereo_gain(param_value, l, r));
                    }
                    2 => {
                        output.set_number_of_channels(2);
                        let [left, right] = output.stereo_mut();
                        input
                            .channel_data(0)
                            .iter()
                            .copied()
                            .zip(input.channel_data(1).iter().copied())
                            .zip(&mut left[..])
                            .zip(&mut right[..])
                            .for_each(|(((il, ir), ol), or)| {
                                apply_stereo_to_stereo_gain(param_value, il, ir, ol, or)
                            });
                    }
                    _ => unreachable!(),
                }
            } else {
                match input.number_of_channels() {
                    1 => {
                        *output = input.clone();
                        output.mix(2, ChannelInterpretation::Speakers);
                        let [left, right] = output.stereo_mut();
                        a_rate_params
                            .zip(&mut left[..])
                            .zip(&mut right[..])
                            .for_each(|((p, l), r)| apply_mono_to_stereo_gain(p, l, r));
                    }
                    2 => {
                        output.set_number_of_channels(2);
                        let [left, right] = output.stereo_mut();
                        a_rate_params
                            .zip(input.channel_data(0).iter().copied())
                            .zip(input.channel_data(1).iter().copied())
                            .zip(&mut left[..])
                            .zip(&mut right[..])
                            .for_each(|((((p, il), ir), ol), or)| {
                                apply_stereo_to_stereo_gain(p, il, ir, ol, or)
                            });
                    }
                    _ => unreachable!(),
                }
            }
        }

        // put the hrtf_state back into self (borrow reasons)
        self.hrtf_state = hrtf_state;

        // tail time only for HRTF panning
        self.hrtf_state.is_some()
    }

    fn onmessage(&mut self, msg: &mut dyn Any) {
        if let Some(control) = msg.downcast_mut::<ControlMessage>() {
            match control {
                ControlMessage::DistanceModel(value) => self.distance_model = *value,
                ControlMessage::RefDistance(value) => self.ref_distance = *value,
                ControlMessage::MaxDistance(value) => self.max_distance = *value,
                ControlMessage::RollOffFactor(value) => self.rolloff_factor = *value,
                ControlMessage::ConeInnerAngle(value) => self.cone_inner_angle = *value,
                ControlMessage::ConeOuterAngle(value) => self.cone_outer_angle = *value,
                ControlMessage::ConeOuterGain(value) => self.cone_outer_gain = *value,
                ControlMessage::PanningModel(value) => self.hrtf_state = value.take(),
            }

            return;
        }

        log::warn!("PannerRenderer: Dropping incoming message {msg:?}");
    }
}

impl PannerRenderer {
    fn cone_gain(
        &self,
        source_position: [f32; 3],
        source_orientation: [f32; 3],
        listener_position: [f32; 3],
    ) -> f32 {
        let abs_inner_angle = self.cone_inner_angle.abs() as f32 / 2.;
        let abs_outer_angle = self.cone_outer_angle.abs() as f32 / 2.;
        if abs_inner_angle >= 180. && abs_outer_angle >= 180. {
            1. // no cone specified
        } else {
            let cone_outer_gain = self.cone_outer_gain as f32;

            let abs_angle =
                crate::spatial::angle(source_position, source_orientation, listener_position);

            if abs_angle < abs_inner_angle {
                1. // No attenuation
            } else if abs_angle >= abs_outer_angle {
                cone_outer_gain // Max attenuation
            } else {
                // Between inner and outer cones: inner -> outer, x goes from 0 -> 1
                let x = (abs_angle - abs_inner_angle) / (abs_outer_angle - abs_inner_angle);
                (1. - x) + cone_outer_gain * x
            }
        }
    }

    fn dist_gain(&self, source_position: [f32; 3], listener_position: [f32; 3]) -> f32 {
        let distance_model = self.distance_model;
        let ref_distance = self.ref_distance;
        let distance = crate::spatial::distance(source_position, listener_position) as f64;

        let dist_gain = match distance_model {
            DistanceModelType::Linear => {
                let rolloff_factor = self.rolloff_factor.clamp(0., 1.);
                let max_distance = self.max_distance;
                let d2ref = ref_distance.min(max_distance);
                let d2max = ref_distance.max(max_distance);
                let d_clamped = distance.clamp(d2ref, d2max);
                1. - rolloff_factor * (d_clamped - d2ref) / (d2max - d2ref)
            }
            DistanceModelType::Inverse => {
                let rolloff_factor = self.rolloff_factor.max(0.);
                if distance > 0. {
                    ref_distance
                        / (ref_distance
                            + rolloff_factor * (ref_distance.max(distance) - ref_distance))
                } else {
                    1.
                }
            }
            DistanceModelType::Exponential => {
                let rolloff_factor = self.rolloff_factor.max(0.);
                (distance.max(ref_distance) / ref_distance).powf(-rolloff_factor)
            }
        };
        dist_gain as f32
    }
}

fn apply_mono_to_stereo_gain(spatial_params: SpatialParams, l: &mut f32, r: &mut f32) {
    let SpatialParams {
        dist_gain,
        cone_gain,
        azimuth,
        ..
    } = spatial_params;

    // Determine left/right ear gain. Clamp azimuth to range of [-180, 180].
    let mut azimuth = azimuth.clamp(-180., 180.);

    // Then wrap to range [-90, 90].
    if azimuth < -90. {
        azimuth = -180. - azimuth;
    } else if azimuth > 90. {
        azimuth = 180. - azimuth;
    }

    // x is the horizontal plane orientation of the sound
    let x = (azimuth + 90.) / 180.;
    let gain_l = (x * PI / 2.).cos();
    let gain_r = (x * PI / 2.).sin();

    // multiply signal with gain per ear
    *l *= gain_l * dist_gain * cone_gain;
    *r *= gain_r * dist_gain * cone_gain;
}

fn apply_stereo_to_stereo_gain(
    spatial_params: SpatialParams,
    il: f32,
    ir: f32,
    ol: &mut f32,
    or: &mut f32,
) {
    let SpatialParams {
        dist_gain,
        cone_gain,
        azimuth,
        ..
    } = spatial_params;

    // Determine left/right ear gain. Clamp azimuth to range of [-180, 180].
    let mut azimuth = azimuth.clamp(-180., 180.);

    // Then wrap to range [-90, 90].
    if azimuth < -90. {
        azimuth = -180. - azimuth;
    } else if azimuth > 90. {
        azimuth = 180. - azimuth;
    }

    // x is the horizontal plane orientation of the sound
    let x = if azimuth <= 0. {
        (azimuth + 90.) / 90.
    } else {
        azimuth / 90.
    };
    let gain_l = (x * PI / 2.).cos();
    let gain_r = (x * PI / 2.).sin();

    // multiply signal with gain per ear
    if azimuth <= 0. {
        *ol = (il + ir * gain_l) * dist_gain * cone_gain;
        *or = ir * gain_r * dist_gain * cone_gain;
    } else {
        *ol = il * gain_l * dist_gain * cone_gain;
        *or = (ir + il * gain_r) * dist_gain * cone_gain;
    }
}

#[cfg(test)]
mod tests {
    use float_eq::{assert_float_eq, assert_float_ne};

    use crate::context::{BaseAudioContext, OfflineAudioContext};
    use crate::node::{AudioBufferSourceNode, AudioBufferSourceOptions, AudioScheduledSourceNode};
    use crate::AudioBuffer;

    use super::*;

    #[test]
    fn test_audioparam_value_applies_immediately() {
        let context = OfflineAudioContext::new(1, 128, 48000.);
        let options = PannerOptions {
            position_x: 12.,
            ..Default::default()
        };
        let src = PannerNode::new(&context, options);
        assert_float_eq!(src.position_x.value(), 12., abs_all <= 0.);
    }

    #[test]
    fn test_equal_power_mono_to_stereo() {
        let sample_rate = 44100.;
        let length = RENDER_QUANTUM_SIZE * 4;
        let mut context = OfflineAudioContext::new(2, length, sample_rate);

        // 128 input samples of value 1.
        let input = AudioBuffer::from(vec![vec![1.; RENDER_QUANTUM_SIZE]], sample_rate);
        let mut src = AudioBufferSourceNode::new(&context, AudioBufferSourceOptions::default());
        src.set_buffer(input);
        src.start();

        let options = PannerOptions {
            panning_model: PanningModelType::EqualPower,
            ..PannerOptions::default()
        };
        let panner = PannerNode::new(&context, options);
        assert_eq!(panner.panning_model(), PanningModelType::EqualPower);
        panner.set_channel_count(1);
        panner.position_x().set_value(1.); // sound comes from the right

        src.connect(&panner);
        panner.connect(&context.destination());

        let output = context.start_rendering_sync();
        let original = vec![1.; RENDER_QUANTUM_SIZE];
        let zero = vec![0.; RENDER_QUANTUM_SIZE];

        // assert first quantum fully panned to the right
        assert_float_eq!(
            output.get_channel_data(0)[..128],
            &zero[..],
            abs_all <= 1E-6
        );
        assert_float_eq!(
            output.get_channel_data(1)[..128],
            &original[..],
            abs_all <= 1E-6
        );

        // assert no tail-time
        assert_float_eq!(
            output.get_channel_data(0)[128..256],
            &zero[..],
            abs_all <= 1E-6
        );
        assert_float_eq!(
            output.get_channel_data(1)[128..256],
            &zero[..],
            abs_all <= 1E-6
        );
    }

    #[test]
    fn test_equal_power_azimuth_mono_to_stereo() {
        let sample_rate = 44100.;
        let length = RENDER_QUANTUM_SIZE;
        let mut context = OfflineAudioContext::new(2, length, sample_rate);

        // 128 input samples of value 1.
        let input = AudioBuffer::from(vec![vec![1.; RENDER_QUANTUM_SIZE]], sample_rate);
        let mut src = AudioBufferSourceNode::new(&context, AudioBufferSourceOptions::default());
        src.set_buffer(input);
        src.start();

        let options = PannerOptions {
            panning_model: PanningModelType::EqualPower,
            ..PannerOptions::default()
        };
        let panner = PannerNode::new(&context, options);
        assert_eq!(panner.panning_model(), PanningModelType::EqualPower);
        panner.position_y().set_value(1.); // sound comes from above

        src.connect(&panner);
        panner.connect(&context.destination());

        let output = context.start_rendering_sync();
        let sqrt2 = vec![(1.0f32 / 2.).sqrt(); RENDER_QUANTUM_SIZE];

        // assert both ears receive equal volume
        assert_float_eq!(
            output.get_channel_data(0)[..128],
            &sqrt2[..],
            abs_all <= 1E-6
        );
        assert_float_eq!(
            output.get_channel_data(1)[..128],
            &sqrt2[..],
            abs_all <= 1E-6
        );
    }

    #[test]
    fn test_equal_power_stereo_to_stereo() {
        let sample_rate = 44100.;
        let length = RENDER_QUANTUM_SIZE;
        let mut context = OfflineAudioContext::new(2, length, sample_rate);

        // put listener at (10, 0, 0), directed at (1, 0, 0)
        let listener = context.listener();
        listener.position_x().set_value(10.);
        listener.position_y().set_value(0.);
        listener.position_z().set_value(0.);
        listener.forward_x().set_value(1.);
        listener.forward_y().set_value(0.);
        listener.forward_z().set_value(0.);
        listener.up_x().set_value(0.);
        listener.up_y().set_value(0.);
        listener.up_z().set_value(1.);

        // 128 input samples of value 1, stereo
        let input = AudioBuffer::from(
            vec![vec![1.; RENDER_QUANTUM_SIZE], vec![1.; RENDER_QUANTUM_SIZE]],
            sample_rate,
        );
        let mut src = AudioBufferSourceNode::new(&context, AudioBufferSourceOptions::default());
        src.set_buffer(input);
        src.start();

        // add panner at (10, 10, 0) - no cone/direction
        let panner = context.create_panner();
        panner.position_x().set_value(10.);
        panner.position_y().set_value(10.);
        panner.position_z().set_value(0.);

        src.connect(&panner);
        panner.connect(&context.destination());

        let output = context.start_rendering_sync();

        // left channel should full signal (both channels summed) = 2.,
        // but distance = 10 so times 0.1
        assert_float_eq!(
            output.get_channel_data(0)[..RENDER_QUANTUM_SIZE],
            &[0.2; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.001
        );
        // right channel should silent
        assert_float_eq!(
            output.get_channel_data(1)[..RENDER_QUANTUM_SIZE],
            &[0.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.001
        );
    }

    #[test]
    fn test_hrtf() {
        let sample_rate = 44100.;
        let length = RENDER_QUANTUM_SIZE * 4;
        let mut context = OfflineAudioContext::new(2, length, sample_rate);

        // 128 input samples of value 1.
        let input = AudioBuffer::from(vec![vec![1.; RENDER_QUANTUM_SIZE]], sample_rate);
        let mut src = AudioBufferSourceNode::new(&context, AudioBufferSourceOptions::default());
        src.set_buffer(input);
        src.start();

        let options = PannerOptions {
            panning_model: PanningModelType::HRTF,
            ..PannerOptions::default()
        };
        let panner = PannerNode::new(&context, options);
        assert_eq!(panner.panning_model(), PanningModelType::HRTF);
        panner.position_x().set_value(1.); // sound comes from the right

        src.connect(&panner);
        panner.connect(&context.destination());

        let output = context.start_rendering_sync();
        let original = vec![1.; RENDER_QUANTUM_SIZE];

        // assert first quantum not equal to input buffer (both left and right)
        assert_float_ne!(
            output.get_channel_data(0)[..128],
            &original[..],
            abs_all <= 1E-6
        );
        assert_float_ne!(
            output.get_channel_data(1)[..128],
            &original[..],
            abs_all <= 1E-6
        );

        // assert some samples non-zero in the tail time
        let left = output.channel_data(0).as_slice();
        assert!(left[128..256].iter().any(|v| *v >= 1E-6));

        let right = output.channel_data(1).as_slice();
        assert!(right[128..256].iter().any(|v| *v >= 1E-6));
    }

    #[test]
    fn test_hrtf_loads_at_minimum_sample_rate() {
        let (_processor, len) = load_hrtf_processor(crate::MIN_SAMPLE_RATE as u32);

        assert!(len > 0);
        assert!(
            len < 512,
            "minimum-rate HRTF should use the resampled HRIR length, got {len}"
        );
    }

    #[test]
    fn test_hrtf_renders_at_minimum_sample_rate() {
        let sample_rate = crate::MIN_SAMPLE_RATE;
        let length = RENDER_QUANTUM_SIZE * 4;
        let mut context = OfflineAudioContext::new(2, length, sample_rate);

        let input = AudioBuffer::from(vec![vec![1.; RENDER_QUANTUM_SIZE]], sample_rate);
        let mut src = AudioBufferSourceNode::new(&context, AudioBufferSourceOptions::default());
        src.set_buffer(input);
        src.start();

        let options = PannerOptions {
            panning_model: PanningModelType::HRTF,
            ..PannerOptions::default()
        };
        let panner = PannerNode::new(&context, options);
        panner.position_x().set_value(1.);

        src.connect(&panner);
        panner.connect(&context.destination());

        let output = context.start_rendering_sync();
        let left = output.channel_data(0).as_slice();
        let right = output.channel_data(1).as_slice();

        assert!(left.iter().all(|v| v.is_finite()));
        assert!(right.iter().all(|v| v.is_finite()));
        assert!(left.iter().any(|v| *v != 0.));
        assert!(right.iter().any(|v| *v != 0.));
    }

    #[test]
    fn test_arate_position_automation_varies_within_quantum() {
        // Regression test for when `single_valued` method only inspected the
        // nine *listener* parameter buffers and ignored the panner's own
        // position/orientation params. A static listener is the common case,
        // so the check almost always passed and any automation on the panner's
        // positionX/Y/Z (a-rate per spec) was silently evaluated once per
        // quantum, i.e. degraded to k-rate: the output moves in 128-frame
        // stair steps instead of per-sample.
        let mut context = OfflineAudioContext::new(2, 256, 48000.);

        let mut src = context.create_constant_source();
        src.offset().set_value(1.);
        src.start();

        let panner = context.create_panner();
        // Sweep the source across the listener within the very first quantum.
        panner.position_x().set_value_at_time(-10., 0.);
        panner
            .position_x()
            .linear_ramp_to_value_at_time(10., 128. / 48000.);

        src.connect(&panner);
        panner.connect(&context.destination());

        let result = context.start_rendering_sync();
        let left = result.get_channel_data(0);

        // With per-sample (a-rate) evaluation the left-channel gain must vary
        // inside the first quantum; the k-rate degradation renders the whole
        // quantum with the frame-0 position and the samples are all equal.
        let varies = left.windows(2).take(127).any(|w| w[0] != w[1]);
        assert!(
            varies,
            "position automation was evaluated once per quantum (k-rate degradation)"
        );
    }
}
