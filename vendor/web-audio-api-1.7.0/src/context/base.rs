//! The `BaseAudioContext` interface

use crate::buffer::{AudioBuffer, AudioBufferOptions};
use crate::context::{
    AudioContextRegistration, AudioContextState, AudioParamId, ConcreteBaseAudioContext,
    DESTINATION_NODE_ID,
};
use crate::decoding::decode_media_data;
use crate::events::{Event, EventHandler, EventType};
use crate::node::{AudioNode, AudioNodeOptions};
use crate::param::AudioParamDescriptor;
use crate::periodic_wave::{PeriodicWave, PeriodicWaveOptions};
use crate::{node, AudioListener, RENDER_QUANTUM_SIZE};

use std::future::Future;

/// The interface representing an audio-processing graph built from audio modules linked together,
/// each represented by an `AudioNode`.
///
/// An audio context controls both the creation of the nodes it contains and the execution of the
/// audio processing, or decoding.
#[allow(clippy::module_name_repetitions)]
pub trait BaseAudioContext {
    /// Returns the [`BaseAudioContext`] concrete type associated with this `AudioContext`
    #[doc(hidden)] // we'd rather not expose the ConcreteBaseAudioContext
    fn base(&self) -> &ConcreteBaseAudioContext;

    /// Decode an [`AudioBuffer`] from a given input stream.
    ///
    /// The current implementation supports Symphonia's audio formats and codecs,
    /// including AIFF, CAF, ISO/MP4, MKV/WebM, Ogg, WAV, AAC, ADPCM, ALAC,
    /// FLAC, MP1/MP2/MP3, PCM, and Vorbis.
    ///
    /// In addition to the official spec, the input parameter can be any byte stream (not just an
    /// array). This means you can decode audio data from a file, network stream, or in memory
    /// buffer, and any other [`std::io::Read`] implementer. The data is buffered internally so you
    /// should not wrap the source in a `BufReader`.
    ///
    /// This function operates synchronously, which may be undesirable on the control thread. The
    /// example shows how to avoid this. See also the async method [`Self::decode_audio_data`].
    ///
    /// # Errors
    ///
    /// This method returns an Error in various cases (IO, mime sniffing, decoding).
    ///
    /// # Usage
    ///
    /// ```no_run
    /// use std::io::Cursor;
    /// use web_audio_api::context::{BaseAudioContext, OfflineAudioContext};
    ///
    /// let input = Cursor::new(vec![0; 32]); // or a File, TcpStream, ...
    ///
    /// let context = OfflineAudioContext::new(2, 44_100, 44_100.);
    /// let handle = std::thread::spawn(move || context.decode_audio_data_sync(input));
    ///
    /// // do other things
    ///
    /// // await result from the decoder thread
    /// let decode_buffer_result = handle.join();
    /// ```
    ///
    /// # Examples
    ///
    /// The following example shows how to use a thread pool for audio buffer decoding:
    ///
    /// `cargo run --release --example decode_multithreaded`
    fn decode_audio_data_sync<R: std::io::Read + Send + Sync + 'static>(
        &self,
        input: R,
    ) -> Result<AudioBuffer, Box<dyn std::error::Error + Send + Sync>> {
        decode_media_data(input, self.sample_rate())
    }

    /// Decode an [`AudioBuffer`] from a given input stream.
    ///
    /// The current implementation supports Symphonia's audio formats and codecs,
    /// including AIFF, CAF, ISO/MP4, MKV/WebM, Ogg, WAV, AAC, ADPCM, ALAC,
    /// FLAC, MP1/MP2/MP3, PCM, and Vorbis.
    ///
    /// In addition to the official spec, the input parameter can be any byte stream (not just an
    /// array). This means you can decode audio data from a file, network stream, or in memory
    /// buffer, and any other [`std::io::Read`] implementer. The data is buffered internally so you
    /// should not wrap the source in a `BufReader`.
    ///
    /// Warning, the current implementation still uses blocking IO so it's best to use Tokio's
    /// `spawn_blocking` to run the decoding on a thread dedicated to blocking operations. See also
    /// the async method [`Self::decode_audio_data_sync`].
    ///
    /// # Errors
    ///
    /// This method returns an Error in various cases (IO, mime sniffing, decoding).
    // Use of `async fn` in public traits is discouraged as auto trait bounds cannot be specified,
    // hence we use `-> impl Future + ..` instead.
    fn decode_audio_data<R: std::io::Read + Send + Sync + 'static>(
        &self,
        input: R,
    ) -> impl Future<Output = Result<AudioBuffer, Box<dyn std::error::Error + Send + Sync>>>
           + Send
           + 'static {
        let sample_rate = self.sample_rate();
        async move { decode_media_data(input, sample_rate) }
    }

    /// Create an new "in-memory" `AudioBuffer` with the given number of channels,
    /// length (i.e. number of samples per channel) and sample rate.
    ///
    /// Note: In most cases you will want the sample rate to match the current
    /// audio context sample rate.
    #[must_use]
    fn create_buffer(
        &self,
        number_of_channels: usize,
        length: usize,
        sample_rate: f32,
    ) -> AudioBuffer {
        let options = AudioBufferOptions {
            number_of_channels,
            length,
            sample_rate,
        };

        AudioBuffer::new(options)
    }

    /// Creates a `AnalyserNode`
    #[must_use]
    fn create_analyser(&self) -> node::AnalyserNode {
        node::AnalyserNode::new(self.base(), node::AnalyserOptions::default())
    }

    /// Creates an `BiquadFilterNode` which implements a second order filter
    #[must_use]
    fn create_biquad_filter(&self) -> node::BiquadFilterNode {
        node::BiquadFilterNode::new(self.base(), node::BiquadFilterOptions::default())
    }

    /// Creates an `AudioBufferSourceNode`
    #[must_use]
    fn create_buffer_source(&self) -> node::AudioBufferSourceNode {
        node::AudioBufferSourceNode::new(self.base(), node::AudioBufferSourceOptions::default())
    }

    /// Creates an `ConstantSourceNode`, a source representing a constant value
    #[must_use]
    fn create_constant_source(&self) -> node::ConstantSourceNode {
        node::ConstantSourceNode::new(self.base(), node::ConstantSourceOptions::default())
    }

    /// Creates an `ConvolverNode`, a processing node which applies linear convolution
    #[must_use]
    fn create_convolver(&self) -> node::ConvolverNode {
        node::ConvolverNode::new(self.base(), node::ConvolverOptions::default())
    }

    /// Creates a `ChannelMergerNode`
    #[must_use]
    fn create_channel_merger(&self, number_of_inputs: usize) -> node::ChannelMergerNode {
        let opts = node::ChannelMergerOptions {
            number_of_inputs,
            ..node::ChannelMergerOptions::default()
        };
        node::ChannelMergerNode::new(self.base(), opts)
    }

    /// Creates a `ChannelSplitterNode`
    #[must_use]
    fn create_channel_splitter(&self, number_of_outputs: usize) -> node::ChannelSplitterNode {
        let opts = node::ChannelSplitterOptions {
            number_of_outputs,
            ..node::ChannelSplitterOptions::default()
        };
        node::ChannelSplitterNode::new(self.base(), opts)
    }

    /// Creates a `DelayNode`, delaying the audio signal
    #[must_use]
    fn create_delay(&self, max_delay_time: f64) -> node::DelayNode {
        let opts = node::DelayOptions {
            max_delay_time,
            ..node::DelayOptions::default()
        };
        node::DelayNode::new(self.base(), opts)
    }

    /// Creates a `DynamicsCompressorNode`, compressing the audio signal
    #[must_use]
    fn create_dynamics_compressor(&self) -> node::DynamicsCompressorNode {
        node::DynamicsCompressorNode::new(self.base(), node::DynamicsCompressorOptions::default())
    }

    /// Creates an `GainNode`, to control audio volume
    #[must_use]
    fn create_gain(&self) -> node::GainNode {
        node::GainNode::new(self.base(), node::GainOptions::default())
    }

    /// Creates an `IirFilterNode`
    ///
    /// # Arguments
    ///
    /// * `feedforward` - An array of the feedforward (numerator) coefficients for the transfer function of the IIR filter.
    ///   The maximum length of this array is 20
    /// * `feedback` - An array of the feedback (denominator) coefficients for the transfer function of the IIR filter.
    ///   The maximum length of this array is 20
    #[must_use]
    fn create_iir_filter(&self, feedforward: Vec<f64>, feedback: Vec<f64>) -> node::IIRFilterNode {
        let options = node::IIRFilterOptions {
            audio_node_options: AudioNodeOptions::default(),
            feedforward,
            feedback,
        };
        node::IIRFilterNode::new(self.base(), options)
    }

    /// Creates an `OscillatorNode`, a source representing a periodic waveform.
    #[must_use]
    fn create_oscillator(&self) -> node::OscillatorNode {
        node::OscillatorNode::new(self.base(), node::OscillatorOptions::default())
    }

    /// Creates a `PannerNode`
    #[must_use]
    fn create_panner(&self) -> node::PannerNode {
        node::PannerNode::new(self.base(), node::PannerOptions::default())
    }

    /// Creates a periodic wave
    ///
    /// Please note that this constructor deviates slightly from the spec by requiring a single
    /// argument with the periodic wave options.
    #[must_use]
    fn create_periodic_wave(&self, options: PeriodicWaveOptions) -> PeriodicWave {
        PeriodicWave::new(self.base(), options)
    }

    /// Creates an `ScriptProcessorNode` for custom audio processing (deprecated);
    ///
    /// # Panics
    ///
    /// This function panics if:
    /// - `buffer_size` is not 256, 512, 1024, 2048, 4096, 8192, or 16384
    /// - the number of input and output channels are both zero
    /// - either of the channel counts exceed [`crate::MAX_CHANNELS`]
    #[must_use]
    fn create_script_processor(
        &self,
        buffer_size: usize,
        number_of_input_channels: usize,
        number_of_output_channels: usize,
    ) -> node::ScriptProcessorNode {
        let options = node::ScriptProcessorOptions {
            buffer_size,
            number_of_input_channels,
            number_of_output_channels,
        };

        node::ScriptProcessorNode::new(self.base(), options)
    }

    /// Creates an `StereoPannerNode` to pan a stereo output
    #[must_use]
    fn create_stereo_panner(&self) -> node::StereoPannerNode {
        node::StereoPannerNode::new(self.base(), node::StereoPannerOptions::default())
    }

    /// Creates a `WaveShaperNode`
    #[must_use]
    fn create_wave_shaper(&self) -> node::WaveShaperNode {
        node::WaveShaperNode::new(self.base(), node::WaveShaperOptions::default())
    }

    /// Returns an `AudioDestinationNode` representing the final destination of all audio in the
    /// context. It can be thought of as the audio-rendering device.
    #[must_use]
    fn destination(&self) -> node::AudioDestinationNode {
        let registration = AudioContextRegistration {
            id: DESTINATION_NODE_ID,
            context: self.base().clone(),
        };
        let channel_config = self.base().destination_channel_config();
        node::AudioDestinationNode::from_raw_parts(registration, channel_config)
    }

    /// Returns the `AudioListener` which is used for 3D spatialization
    #[must_use]
    fn listener(&self) -> AudioListener {
        self.base().listener()
    }

    /// The sample rate (in sample-frames per second) at which the `AudioContext` handles audio.
    #[must_use]
    fn sample_rate(&self) -> f32 {
        self.base().sample_rate()
    }

    /// Returns state of current context
    #[must_use]
    fn state(&self) -> AudioContextState {
        self.base().state()
    }

    /// This is the time in seconds of the sample frame immediately following the last sample-frame
    /// in the block of audio most recently processed by the context’s rendering graph.
    #[must_use]
    fn current_time(&self) -> f64 {
        self.base().current_time()
    }

    // Number of frames computed on each render block
    #[must_use]
    fn render_quantum_size(&self) -> usize {
        RENDER_QUANTUM_SIZE
    }

    /// Create an `AudioParam`.
    ///
    /// Call this inside the `register` closure when setting up your `AudioNode`
    #[must_use]
    fn create_audio_param(
        &self,
        opts: AudioParamDescriptor,
        dest: &AudioContextRegistration,
    ) -> (crate::param::AudioParam, AudioParamId) {
        let param = self.base().register(move |registration| {
            let (node, proc) = crate::param::audio_param_pair(opts, registration);

            (node, Box::new(proc))
        });

        // Connect the param to the node, once the node is registered inside the audio graph.
        self.base().queue_audio_param_connect(&param, dest.id());

        let proc_id = AudioParamId(param.registration().id().0);
        (param, proc_id)
    }

    /// Register callback to run when the state of the AudioContext has changed
    ///
    /// Only a single event handler is active at any time. Calling this method multiple times will
    /// override the previous event handler.
    fn set_onstatechange<F: FnMut(Event) + Send + 'static>(&self, mut callback: F) {
        let callback = move |_| {
            callback(Event {
                type_: "statechange",
            })
        };

        self.base().set_event_handler(
            EventType::StateChange,
            EventHandler::Multiple(Box::new(callback)),
        );
    }

    /// Unset the callback to run when the state of the AudioContext has changed
    fn clear_onstatechange(&self) {
        self.base().clear_event_handler(EventType::StateChange);
    }

    #[cfg(test)]
    fn mock_registration(&self) -> AudioContextRegistration {
        AudioContextRegistration {
            id: crate::context::AudioNodeId(0),
            context: self.base().clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::OfflineAudioContext;

    use float_eq::assert_float_eq;

    fn require_send_sync_static<T: Send + Sync + 'static>(_: T) {}

    #[test]
    fn test_decode_audio_data_sync() {
        let context = OfflineAudioContext::new(1, 1, 44100.);
        let file = std::fs::File::open("samples/sample.wav").unwrap();
        let audio_buffer = context.decode_audio_data_sync(file).unwrap();

        assert_eq!(audio_buffer.sample_rate(), 44100.);
        assert_eq!(audio_buffer.length(), 142_187);
        assert_eq!(audio_buffer.number_of_channels(), 2);
        assert_float_eq!(audio_buffer.duration(), 3.224, abs_all <= 0.001);

        let left_start = &audio_buffer.get_channel_data(0)[0..100];
        let right_start = &audio_buffer.get_channel_data(1)[0..100];
        // assert distinct two channel data
        assert!(left_start != right_start);
    }

    #[test]
    fn test_decode_audio_data_future_send_static() {
        let context = OfflineAudioContext::new(1, 1, 44100.);
        let file = std::fs::File::open("samples/sample.wav").unwrap();
        let future = context.decode_audio_data(file);
        require_send_sync_static(future);
    }

    #[test]
    fn test_decode_audio_data_async() {
        use futures::executor;
        let context = OfflineAudioContext::new(1, 1, 44100.);
        let file = std::fs::File::open("samples/sample.wav").unwrap();
        let future = context.decode_audio_data(file);
        let audio_buffer = executor::block_on(future).unwrap();

        assert_eq!(audio_buffer.sample_rate(), 44100.);
        assert_eq!(audio_buffer.length(), 142_187);
        assert_eq!(audio_buffer.number_of_channels(), 2);
        assert_float_eq!(audio_buffer.duration(), 3.224, abs_all <= 0.001);

        let left_start = &audio_buffer.get_channel_data(0)[0..100];
        let right_start = &audio_buffer.get_channel_data(1)[0..100];
        // assert distinct two channel data
        assert!(left_start != right_start);
    }

    // #[test]
    // disabled: symphonia cannot handle empty WAV-files
    #[allow(dead_code)]
    fn test_decode_audio_data_empty() {
        let context = OfflineAudioContext::new(1, 1, 44100.);
        let file = std::fs::File::open("samples/empty_2c.wav").unwrap();
        let audio_buffer = context.decode_audio_data_sync(file).unwrap();
        assert_eq!(audio_buffer.length(), 0);
    }

    #[test]
    fn test_decode_audio_data_decoding_error() {
        let context = OfflineAudioContext::new(1, 1, 44100.);
        let file = std::fs::File::open("samples/corrupt.wav").unwrap();
        assert!(context.decode_audio_data_sync(file).is_err());
    }

    #[test]
    fn test_create_buffer() {
        let number_of_channels = 3;
        let length = 2000;
        let sample_rate = 96_000.;

        let context = OfflineAudioContext::new(1, 1, 44100.);
        let buffer = context.create_buffer(number_of_channels, length, sample_rate);

        assert_eq!(buffer.number_of_channels(), 3);
        assert_eq!(buffer.length(), 2000);
        assert_float_eq!(buffer.sample_rate(), 96000., abs_all <= 0.);
    }
}
