//! The IIR filter control and renderer parts
use arrayvec::ArrayVec;
use num_complex::Complex;

use std::f64::consts::PI;

use crate::context::{AudioContextRegistration, BaseAudioContext};
use crate::render::{
    AudioParamValues, AudioProcessor, AudioRenderQuantum, AudioWorkletGlobalScope,
};
use crate::MAX_CHANNELS;

use super::{AudioNode, AudioNodeOptions, ChannelConfig};

/// Filter order is limited to 20
const MAX_IIR_COEFFS_LEN: usize = 20;

/// Assert that the feedforward coefficients are valid
/// see <https://webaudio.github.io/web-audio-api/#dom-baseaudiocontext-createiirfilter-feedforward>
///
/// # Panics
///
/// This function panics if:
/// - coefs length is 0 and greater than 20
/// - all coefs are zeros
///
#[track_caller]
#[inline(always)]
fn assert_valid_feedforward_coefs(coefs: &[f64]) {
    assert!(
        !coefs.is_empty() && coefs.len() <= MAX_IIR_COEFFS_LEN,
        "NotSupportedError - IIR Filter feedforward coefficients should have length >= 0 and <= {}",
        MAX_IIR_COEFFS_LEN,
    );

    assert!(
        !coefs.iter().all(|&f| f == 0.),
        "InvalidStateError - IIR Filter feedforward coefficients cannot be all zeros"
    );
}

/// Assert that the feedforward coefficients are valid
/// see <https://webaudio.github.io/web-audio-api/#dom-baseaudiocontext-createiirfilter-feedforward>
///
/// # Panics
///
/// This function panics if:
/// - coefs length is 0 and greater than 20
/// - first coef is zero
///
#[track_caller]
#[inline(always)]
fn assert_valid_feedback_coefs(coefs: &[f64]) {
    assert!(
        !coefs.is_empty() && coefs.len() <= MAX_IIR_COEFFS_LEN,
        "NotSupportedError - IIR Filter feedback coefficients should have length >= 0 and <= {}",
        MAX_IIR_COEFFS_LEN,
    );

    assert_ne!(
        coefs[0], 0.,
        "InvalidStateError - IIR Filter feedback first coefficient cannot be zero"
    );
}

/// Options for constructing a [`IIRFilterNode`]
// dictionary IIRFilterOptions : AudioNodeOptions {
//   required sequence<double> feedforward;
//   required sequence<double> feedback;
// };
#[derive(Clone, Debug)]
pub struct IIRFilterOptions {
    /// audio node options
    pub audio_node_options: AudioNodeOptions,
    /// feedforward coefficients
    pub feedforward: Vec<f64>, // go for Option<Vec<f32>> w/ default to None?
    /// feedback coefficients
    pub feedback: Vec<f64>, // go for Option<Vec<f32>> w/ default to None?
}

/// IIRFilterNode is an AudioNode processor implementing a general IIR
/// (infinite impulse response)Filter.
///
/// In general, you should prefer using a BiquadFilterNode for the following reasons:
/// - Generally less sensitive to numeric issues
/// - Filter parameters can be automated
/// - Can be used to create all even-ordered IIR filters
///
/// However, odd-ordered filters cannot be created with BiquadFilterNode, so if
/// your application require such filters and/or automation is not needed, then IIR
/// filters may be appropriate. In short, use this if you know what you are doing!
///
/// Note that once created, the coefficients of the IIR filter cannot be changed.
///
/// - MDN documentation: <https://developer.mozilla.org/en-US/docs/Web/API/IIRFilterNode>
/// - specification: <https://webaudio.github.io/web-audio-api/#IIRFilterNode>
/// - see also: [`BaseAudioContext::create_iir_filter`]
///
/// # Usage
///
/// ```no_run
/// use std::fs::File;
/// use web_audio_api::context::{AudioContext, BaseAudioContext};
/// use web_audio_api::node::{AudioNode, AudioScheduledSourceNode};
///
/// // create context and grab some audio buffer
/// let context = AudioContext::default();
/// let file = File::open("samples/think-stereo-48000.wav").unwrap();
/// let buffer = context.decode_audio_data_sync(file).unwrap();
///
/// // these coefficients correspond to a lowpass filter at 200Hz (calculated from biquad)
/// let feedforward = vec![
///     0.0002029799640409502,
///     0.0004059599280819004,
///     0.0002029799640409502,
/// ];
///
/// let feedback = vec![
///     1.0126964557853775,
///     -1.9991880801438362,
///     0.9873035442146225,
/// ];
///
/// // create the IIR filter node
/// let iir = context.create_iir_filter(feedforward, feedback);
/// iir.connect(&context.destination());
///
/// // play the buffer and pipe it into the filter
/// let mut src = context.create_buffer_source();
/// src.connect(&iir);
/// src.set_buffer(buffer);
/// src.set_loop(true);
/// src.start();
/// ```
///
/// # Examples
///
/// - `cargo run --release --example iir`
///
#[derive(Debug)]
pub struct IIRFilterNode {
    /// Represents the node instance and its associated audio context
    registration: AudioContextRegistration,
    /// Infos about audio node channel configuration
    channel_config: ChannelConfig,
    /// numerator filter's coefficients
    feedforward: Vec<f64>,
    /// denomintor filter's coefficients
    feedback: Vec<f64>,
}

impl AudioNode for IIRFilterNode {
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
}

impl IIRFilterNode {
    /// Creates an `IirFilterNode`
    ///
    /// # Arguments
    ///
    /// - `context` - Audio context in which the node will live
    /// - `options` - node options
    ///
    /// # Panics
    ///
    /// This function panics if:
    /// - coefs length is 0 and greater than 20
    /// - feedforward coefs are all zeros
    /// - feedback first coef is zero
    ///
    pub fn new<C: BaseAudioContext>(context: &C, options: IIRFilterOptions) -> Self {
        context.base().register(move |registration| {
            let IIRFilterOptions {
                feedforward,
                feedback,
                audio_node_options: channel_config,
            } = options;

            assert_valid_feedforward_coefs(&feedforward);
            assert_valid_feedback_coefs(&feedback);

            let render = IirFilterRenderer::new(feedforward.clone(), feedback.clone());

            let node = Self {
                registration,
                channel_config: channel_config.into(),
                feedforward,
                feedback,
            };

            (node, Box::new(render))
        })
    }

    /// Returns the frequency response for the specified frequencies
    ///
    /// # Arguments
    ///
    /// - `frequency_hz` - frequencies for which frequency response of the filter should be calculated
    /// - `mag_response` - magnitude of the frequency response of the filter
    /// - `phase_response` - phase of the frequency response of the filter
    ///
    /// # Panics
    ///
    /// This function will panic if arguments' lengths don't match
    ///
    pub fn get_frequency_response(
        &self,
        frequency_hz: &[f32],
        mag_response: &mut [f32],
        phase_response: &mut [f32],
    ) {
        assert!(
            frequency_hz.len() == mag_response.len() && mag_response.len() == phase_response.len(),
            "InvalidAccessError - Parameter lengths must match",
        );

        let sample_rate = self.context().sample_rate() as f64;
        let nquist = sample_rate / 2.;

        for (i, &f) in frequency_hz.iter().enumerate() {
            let freq = f64::from(f);
            // <https://webaudio.github.io/web-audio-api/#dom-iirfilternode-getfrequencyresponse>
            // > If a value in the frequencyHz parameter is not within [0, sampleRate/2],
            // > where sampleRate is the value of the sampleRate property of the AudioContext,
            // > the corresponding value at the same index of the magResponse/phaseResponse
            // > array MUST be NaN.
            if freq < 0. || freq > nquist {
                mag_response[i] = f32::NAN;
                phase_response[i] = f32::NAN;
            } else {
                let z = -2.0 * PI * freq / sample_rate;
                let mut num: Complex<f64> = Complex::new(0., 0.);
                let mut denom: Complex<f64> = Complex::new(0., 0.);

                for (idx, &b) in self.feedforward.iter().enumerate() {
                    num += Complex::from_polar(b, idx as f64 * z);
                }

                for (idx, &a) in self.feedback.iter().enumerate() {
                    denom += Complex::from_polar(a, idx as f64 * z);
                }

                let response = num / denom;

                let (mag, phase) = response.to_polar();
                mag_response[i] = mag as f32;
                phase_response[i] = phase as f32;
            }
        }
    }
}

/// Renderer associated with the `IirFilterNode`
struct IirFilterRenderer {
    /// Normalized filter's coeffs -- `(b[n], a[n])`
    norm_coeffs: ArrayVec<(f64, f64), MAX_IIR_COEFFS_LEN>,
    /// Per-channel filter state. Only the first `norm_coeffs.len()` entries are active.
    states: ArrayVec<[f64; MAX_IIR_COEFFS_LEN], MAX_CHANNELS>,
}

impl IirFilterRenderer {
    /// Build an `IirFilterNode` renderer
    ///
    /// # Arguments
    ///
    /// * `config` - renderer config
    fn new(mut feedforward: Vec<f64>, mut feedback: Vec<f64>) -> Self {
        // make sure feedback and feedforward have same length, fill with 0. to match
        match (feedforward.len(), feedback.len()) {
            (feedforward_len, feedback_len) if feedforward_len < feedback_len => {
                feedforward = feedforward
                    .into_iter()
                    .chain(std::iter::repeat(0.))
                    .take(feedback_len)
                    .collect();
            }
            (feedforward_len, feedback_len) if feedforward_len > feedback_len => {
                feedback = feedback
                    .into_iter()
                    .chain(std::iter::repeat(0.))
                    .take(feedforward_len)
                    .collect();
            }
            _ => (),
        };

        let a0 = feedback[0];
        let mut norm_coeffs: ArrayVec<(f64, f64), MAX_IIR_COEFFS_LEN> =
            feedforward.into_iter().zip(feedback).collect();

        norm_coeffs.iter_mut().for_each(|(b, a)| {
            *b /= a0;
            *a /= a0;
        });

        // eagerly assume stereo input, will adjust during rendering if needed
        let mut states = ArrayVec::new();
        states.push([0.; MAX_IIR_COEFFS_LEN]);
        states.push([0.; MAX_IIR_COEFFS_LEN]);

        Self {
            norm_coeffs,
            states,
        }
    }
}

impl AudioProcessor for IirFilterRenderer {
    fn process(
        &mut self,
        inputs: &[AudioRenderQuantum],
        outputs: &mut [AudioRenderQuantum],
        _params: AudioParamValues<'_>,
        _scope: &AudioWorkletGlobalScope,
    ) -> bool {
        // single input/output node
        let input = &inputs[0];
        let output = &mut outputs[0];

        // handle tail time
        if input.is_silent() {
            let mut ended = true;

            // if all values in states are 0., we have nothing left to process
            self.states.iter().all(|state| {
                if state[..self.norm_coeffs.len()]
                    .iter()
                    .any(|&v| v.is_normal())
                {
                    ended = false;
                }
                // if ended is false, `iter().all` will stop early
                ended
            });

            if ended {
                output.make_silent();
                return false;
            }
        }

        // eventually resize state according to input number of channels
        // if in tail time we should continue with previous number of channels
        if !input.is_silent() {
            // @todo - handle channel change cleanly, could cause discontinuities
            // see https://github.com/WebAudio/web-audio-api/issues/1719
            // see https://webaudio.github.io/web-audio-api/#channels-tail-time
            let num_channels = input.number_of_channels();

            if num_channels != self.states.len() {
                self.states.truncate(num_channels);
                for _ in self.states.len()..num_channels {
                    self.states.push([0.; MAX_IIR_COEFFS_LEN]);
                }
            }

            output.set_number_of_channels(num_channels);
        } else {
            let num_channels = self.states.len();
            output.set_number_of_channels(num_channels);
        }

        // apply filter
        for (channel_number, output_channel) in output.channels_mut().iter_mut().enumerate() {
            let input_channel = if input.is_silent() {
                input.channel_data(0)
            } else {
                input.channel_data(channel_number)
            };
            let channel_state = &mut self.states[channel_number];

            for (&i, o) in input_channel.iter().zip(output_channel.iter_mut()) {
                let input = f64::from(i);
                let b0 = self.norm_coeffs[0].0;
                let last_state = channel_state[0];
                let mut output = b0.mul_add(input, last_state);
                // do not propagate denormals into graph and state
                if !output.is_normal() {
                    output = 0.;
                }

                // update states for next call
                for (i, (b, a)) in self.norm_coeffs.iter().skip(1).enumerate() {
                    let state = channel_state[i + 1];
                    channel_state[i] = b * input - a * output + state;
                }

                #[cfg(debug_assertions)]
                if output.is_nan() || output.is_infinite() {
                    log::debug!("An unstable filter is processed.");
                }

                *o = output as f32;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use float_eq::assert_float_eq;
    use std::fs::File;

    use crate::context::{BaseAudioContext, OfflineAudioContext};
    use crate::node::{AudioNode, AudioScheduledSourceNode, BiquadFilterType};
    use crate::AudioBuffer;

    use super::*;

    const LENGTH: usize = 512;

    #[test]
    fn test_constructor_and_factory() {
        {
            let context = OfflineAudioContext::new(2, LENGTH, 44_100.);

            let options = IIRFilterOptions {
                feedback: vec![1.; 3],
                feedforward: vec![1.; 3],
                audio_node_options: AudioNodeOptions::default(),
            };

            let _biquad = IIRFilterNode::new(&context, options);
        }

        {
            let context = OfflineAudioContext::new(2, LENGTH, 44_100.);

            let feedforward = vec![1.; 3];
            let feedback = vec![1.; 3];
            let _biquad = context.create_iir_filter(feedforward, feedback);
        }
    }

    #[test]
    #[should_panic]
    fn test_invalid_feedforward_size() {
        let feedforward = vec![1.; 21];
        assert_valid_feedforward_coefs(&feedforward);
    }

    #[test]
    #[should_panic]
    fn test_invalid_feedforward_values() {
        let feedforward = vec![0.; 5];
        assert_valid_feedforward_coefs(&feedforward);
    }

    #[test]
    fn test_valid_feedforward_values() {
        let feedforward = vec![1.; 5];
        assert_valid_feedforward_coefs(&feedforward);
    }

    #[test]
    #[should_panic]
    fn test_invalid_feedback_size() {
        let feedback = vec![1.; 21];
        assert_valid_feedback_coefs(&feedback);
    }

    #[test]
    #[should_panic]
    fn test_invalid_feedback_values() {
        let mut feedback = vec![1.; 5];
        feedback[0] = 0.;
        assert_valid_feedback_coefs(&feedback);
    }

    #[test]
    fn test_valid_feedback_values() {
        let feedback = vec![1.; 5];
        assert_valid_feedback_coefs(&feedback);
    }

    #[test]
    #[should_panic]
    fn test_frequency_response_arguments() {
        let context = OfflineAudioContext::new(2, 555, 44_100.);
        let options = IIRFilterOptions {
            feedback: vec![1.; 10],
            feedforward: vec![1.; 10],
            audio_node_options: AudioNodeOptions::default(),
        };
        let iir = IIRFilterNode::new(&context, options);

        let frequency_hz = [0.];
        let mut mag_response = [0., 1.0];
        let mut phase_response = [0.];

        iir.get_frequency_response(&frequency_hz, &mut mag_response, &mut phase_response);
    }

    #[test]
    #[should_panic]
    fn test_frequency_response_arguments_2() {
        let context = OfflineAudioContext::new(2, 555, 44_100.);
        let options = IIRFilterOptions {
            feedback: vec![1.; 10],
            feedforward: vec![1.; 10],
            audio_node_options: AudioNodeOptions::default(),
        };
        let iir = IIRFilterNode::new(&context, options);

        let frequency_hz = [0.];
        let mut mag_response = [0.];
        let mut phase_response = [0., 1.0];

        iir.get_frequency_response(&frequency_hz, &mut mag_response, &mut phase_response);
    }

    #[test]
    fn test_one_zero_with_feedback_feedforward_different_length() {
        // cf. the-audio-api/the-iirfilternode-interface/iirfilter.html
        let sample_rate = 24000.;
        let mut context = OfflineAudioContext::new(1, 8000, sample_rate);
        let buffer = AudioBuffer::from(vec![vec![1.]], sample_rate);
        let mut source = context.create_buffer_source();
        source.set_buffer(buffer);

        let iir = context.create_iir_filter(vec![0.5, 0.5], vec![1.]);
        source.connect(&iir).connect(&context.destination());
        source.start();

        let output = context.start_rendering_sync();
        let result = output.get_channel_data(0);
        let mut expected = [0.; 8000];
        expected[0] = 0.5;
        expected[1] = 0.5;

        assert_float_eq!(result[..], expected, abs_all <= 0.);
    }

    #[test]
    #[allow(clippy::redundant_clone)]
    fn test_output_against_biquad() {
        let context = OfflineAudioContext::new(1, 1, 44_100.);
        let file = File::open("samples/white.ogg").unwrap();
        let noise = context.decode_audio_data_sync(file).unwrap();

        fn compare_output(
            noise: AudioBuffer,
            filter_type: BiquadFilterType,
            feedback: Vec<f64>,
            feedforward: Vec<f64>,
        ) {
            let frequency = 2000.;
            let q = 1.;
            let gain = 3.;
            // output of biquad and iir filters applied to white noise should thus be equal
            let biquad_res = {
                let mut context = OfflineAudioContext::new(1, 1000, 44_100.);

                let mut biquad = context.create_biquad_filter();
                biquad.connect(&context.destination());
                biquad.set_type(filter_type);
                biquad.frequency().set_value(frequency);
                biquad.q().set_value(q);
                biquad.gain().set_value(gain);

                let mut src = context.create_buffer_source();
                src.connect(&biquad);
                src.set_buffer(noise.clone());
                src.start();

                context.start_rendering_sync()
            };

            let iir_res = {
                let mut context = OfflineAudioContext::new(1, 1000, 44_100.);

                let iir = context.create_iir_filter(feedforward, feedback);
                iir.connect(&context.destination());

                let mut src = context.create_buffer_source();
                src.connect(&iir);
                src.set_buffer(noise.clone());
                src.start();

                context.start_rendering_sync()
            };

            assert_float_eq!(
                biquad_res.get_channel_data(0),
                iir_res.get_channel_data(0),
                abs_all <= 0.
            );
        }

        // these are the unormalized coefs computed by the biquad filter for:
        // - frequency = 2000.;
        // - q = 1.;
        // - gain = 3.;
        // see node::biquad_filter::tests::test_frequency_responses

        // lowpass
        let a0 = 1.1252702717383296;
        let a1 = -1.9193504546709936;
        let a2 = 0.8747297282616704;
        let b0 = 0.02016238633225159;
        let b1 = 0.04032477266450318;
        let b2 = 0.02016238633225159;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];
        compare_output(
            noise.clone(),
            BiquadFilterType::Lowpass,
            feedback,
            feedforward,
        );

        // highpass
        let a0 = 1.1252702717383296;
        let a1 = -1.9193504546709936;
        let a2 = 0.8747297282616704;
        let b0 = 0.9798376136677485;
        let b1 = -1.959675227335497;
        let b2 = 0.9798376136677485;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];
        compare_output(
            noise.clone(),
            BiquadFilterType::Highpass,
            feedback,
            feedforward,
        );

        // bandpass
        let a0 = 1.1405555566658274;
        let a1 = -1.9193504546709936;
        let a2 = 0.8594444433341726;
        let b0 = 0.14055555666582747;
        let b1 = 0.0;
        let b2 = -0.14055555666582747;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];
        compare_output(
            noise.clone(),
            BiquadFilterType::Bandpass,
            feedback,
            feedforward,
        );

        // notch
        let a0 = 1.1405555566658274;
        let a1 = -1.9193504546709936;
        let a2 = 0.8594444433341726;
        let b0 = 1.0;
        let b1 = -1.9193504546709936;
        let b2 = 1.0;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];
        compare_output(
            noise.clone(),
            BiquadFilterType::Notch,
            feedback,
            feedforward,
        );

        // allpass
        let a0 = 1.1405555566658274;
        let a1 = -1.9193504546709936;
        let a2 = 0.8594444433341726;
        let b0 = 0.8594444433341726;
        let b1 = -1.9193504546709936;
        let b2 = 1.1405555566658274;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];
        compare_output(
            noise.clone(),
            BiquadFilterType::Allpass,
            feedback,
            feedforward,
        );

        // peaking
        let a0 = 1.1182627625098631;
        let a1 = -1.9193504546709936;
        let a2 = 0.8817372374901369;
        let b0 = 1.167050592175986;
        let b1 = -1.9193504546709936;
        let b2 = 0.8329494078240139;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];
        compare_output(
            noise.clone(),
            BiquadFilterType::Peaking,
            feedback,
            feedforward,
        );

        // lowshelf
        let a0 = 2.8028072429836723;
        let a1 = -4.577507200153761;
        let a2 = 1.935999047828101;
        let b0 = 2.9011403634599007;
        let b1 = -4.544236234748791;
        let b2 = 1.8709368927568424;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];
        compare_output(
            noise.clone(),
            BiquadFilterType::Lowshelf,
            feedback,
            feedforward,
        );

        // highshelf
        let a0 = 2.4410054070459357;
        let a1 = -3.8234982904056865;
        let a2 = 1.5741972118903644;
        let b0 = 3.331142651362703;
        let b1 = -5.440377503491735;
        let b2 = 2.300939180659645;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];

        compare_output(
            noise.clone(),
            BiquadFilterType::Highshelf,
            feedback,
            feedforward,
        );
    }

    #[test]
    fn tests_get_frequency_response() {
        // This reference response has been generated with the python lib scipy
        // b, a = signal.iirfilter(2, 4000, rs=60, btype='high', analog=False, ftype='cheby2', fs=44100)
        // w, h = signal.freqz(b, a, 10, fs=44100)
        let ref_mag = [
            1e-3,
            4.152_807e-4,
            1.460_789_5e-3,
            5.051_316e-3,
            1.130_323_5e-2,
            2.230_340_2e-2,
            4.311_698e-2,
            8.843_45e-2,
            2.146_620_2e-1,
            6.802_952e-1,
        ];
        let context = OfflineAudioContext::new(2, LENGTH, 44_100.);
        let feedforward = vec![
            0.019_618_022_238_052_212,
            -0.036_007_928_102_449_24,
            0.019_618_022_238_052_21,
        ];
        let feedback = vec![1., 1.576_436_200_538_313_7, 0.651_680_173_116_867_3];

        let options = IIRFilterOptions {
            feedback,
            feedforward,
            audio_node_options: AudioNodeOptions::default(),
        };
        let iir = IIRFilterNode::new(&context, options);

        let frequency_hz = [
            0., 2205., 4410., 6615., 8820., 11025., 13230., 15435., 17640., 19845.,
        ];
        let mut mag_response = [0.; 10];
        let mut phase_response = [0.; 10];

        iir.get_frequency_response(&frequency_hz, &mut mag_response, &mut phase_response);

        assert_float_eq!(mag_response, ref_mag, abs_all <= 0.);
    }

    #[test]
    fn test_frequency_responses_against_biquad() {
        fn compare_frequency_response(
            filter_type: BiquadFilterType,
            feedback: Vec<f64>,
            feedforward: Vec<f64>,
        ) {
            let frequency = 2000.;
            let q = 1.;
            let gain = 3.;
            let freqs = [
                400., 800., 1200., 1600., 2000., 2400., 2800., 3200., 3600., 4000.,
            ];

            let context = OfflineAudioContext::new(1, 1, 44_100.);

            let biquad_response = {
                let mut mags = [0.; 10];
                let mut phases = [0.; 10];

                let mut biquad = context.create_biquad_filter();
                biquad.set_type(filter_type);
                biquad.frequency().set_value(frequency);
                biquad.q().set_value(q);
                biquad.gain().set_value(gain);

                biquad.get_frequency_response(&freqs, &mut mags, &mut phases);

                (mags, phases)
            };

            let iir_response = {
                let mut mags = [0.; 10];
                let mut phases = [0.; 10];

                let iir = context.create_iir_filter(feedforward, feedback);

                iir.get_frequency_response(&freqs, &mut mags, &mut phases);

                (mags, phases)
            };

            assert_float_eq!(biquad_response.0, iir_response.0, abs_all <= 1e-6);
            assert_float_eq!(biquad_response.1, iir_response.1, abs_all <= 1e-6);
        }

        // lowpass
        let a0 = 1.1252702717383296;
        let a1 = -1.9193504546709936;
        let a2 = 0.8747297282616704;
        let b0 = 0.02016238633225159;
        let b1 = 0.04032477266450318;
        let b2 = 0.02016238633225159;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];
        compare_frequency_response(BiquadFilterType::Lowpass, feedback, feedforward);

        // highpass
        let a0 = 1.1252702717383296;
        let a1 = -1.9193504546709936;
        let a2 = 0.8747297282616704;
        let b0 = 0.9798376136677485;
        let b1 = -1.959675227335497;
        let b2 = 0.9798376136677485;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];
        compare_frequency_response(BiquadFilterType::Highpass, feedback, feedforward);

        // bandpass
        let a0 = 1.1405555566658274;
        let a1 = -1.9193504546709936;
        let a2 = 0.8594444433341726;
        let b0 = 0.14055555666582747;
        let b1 = 0.0;
        let b2 = -0.14055555666582747;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];
        compare_frequency_response(BiquadFilterType::Bandpass, feedback, feedforward);

        // one of the value in phases differs by 0.28
        // @todo - handle that
        // notch
        // let a0 = 1.1405555566658274;
        // let a1 = -1.9193504546709936;
        // let a2 = 0.8594444433341726;
        // let b0 = 1.0;
        // let b1 = -1.9193504546709936;
        // let b2 = 1.0;

        // let feedback = vec![a0, a1, a2];
        // let feedforward = vec![b0, b1, b2];
        // compare_frequency_response(BiquadFilterType::Notch, feedback, feedforward);

        // allpass
        let a0 = 1.1405555566658274;
        let a1 = -1.9193504546709936;
        let a2 = 0.8594444433341726;
        let b0 = 0.8594444433341726;
        let b1 = -1.9193504546709936;
        let b2 = 1.1405555566658274;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];
        compare_frequency_response(BiquadFilterType::Allpass, feedback, feedforward);

        // peaking
        let a0 = 1.1182627625098631;
        let a1 = -1.9193504546709936;
        let a2 = 0.8817372374901369;
        let b0 = 1.167050592175986;
        let b1 = -1.9193504546709936;
        let b2 = 0.8329494078240139;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];
        compare_frequency_response(BiquadFilterType::Peaking, feedback, feedforward);

        // lowshelf
        let a0 = 2.8028072429836723;
        let a1 = -4.577507200153761;
        let a2 = 1.935999047828101;
        let b0 = 2.9011403634599007;
        let b1 = -4.544236234748791;
        let b2 = 1.8709368927568424;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];
        compare_frequency_response(BiquadFilterType::Lowshelf, feedback, feedforward);

        // highshelf
        let a0 = 2.4410054070459357;
        let a1 = -3.8234982904056865;
        let a2 = 1.5741972118903644;
        let b0 = 3.331142651362703;
        let b1 = -5.440377503491735;
        let b2 = 2.300939180659645;

        let feedback = vec![a0, a1, a2];
        let feedforward = vec![b0, b1, b2];
        compare_frequency_response(BiquadFilterType::Highshelf, feedback, feedforward);
    }

    #[test]
    fn test_frequency_response_invalid_frequencies() {
        let context = OfflineAudioContext::new(2, 555, 44_100.);
        let options = IIRFilterOptions {
            feedback: vec![1.; 10],
            feedforward: vec![1.; 10],
            audio_node_options: AudioNodeOptions::default(),
        };
        let iir = IIRFilterNode::new(&context, options);

        let frequency_hz = [-1., 22_051.];
        let mut mags = [0.; 2];
        let mut phases = [0.; 2];

        iir.get_frequency_response(&frequency_hz, &mut mags, &mut phases);
        mags.iter().for_each(|v| assert!(v.is_nan()));
        phases.iter().for_each(|v| assert!(v.is_nan()));
    }
}
