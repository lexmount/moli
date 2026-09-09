use crate::context::{AudioContextRegistration, BaseAudioContext};
use crate::media_streams::MediaStreamTrack;
use crate::resampling::Resampler;
use crate::RENDER_QUANTUM_SIZE;

use super::{AudioNode, ChannelConfig, MediaStreamRenderer};

/// Options for constructing a [`MediaStreamTrackAudioSourceNode`]
// dictionary MediaStreamTrackAudioSourceOptions {
//     required MediaStreamTrack mediaStreamTrack;
// };
//
// @note - Does not extend AudioNodeOptions because AudioNodeOptions are
// useless for source nodes as they instruct how to upmix the inputs.
// This is a common source of confusion, see e.g. https://github.com/mdn/content/pull/18472
#[derive(Debug)]
pub struct MediaStreamTrackAudioSourceOptions<'a> {
    pub media_stream_track: &'a MediaStreamTrack,
}

/// An audio source from a [`MediaStreamTrack`] (e.g. the audio track of the microphone input)
///
/// Below is an example showing how to create and play a stream directly in the audio context.
/// Take care:  The media stream will be polled on the render thread which will have catastrophic
/// effects if the iterator blocks or for another reason takes too much time to yield a new sample
/// frame.  Use a [`MediaElementAudioSourceNode`](crate::node::MediaElementAudioSourceNode) for
/// real time safe media playback.
///
/// # Example
///
/// ```no_run
/// use web_audio_api::context::{AudioContext, BaseAudioContext};
/// use web_audio_api::{AudioBuffer, AudioBufferOptions};
/// use web_audio_api::node::AudioNode;
/// use web_audio_api::media_streams::MediaStreamTrack;
///
/// // create a new buffer: 512 samples of silence
/// let options = AudioBufferOptions {
///     number_of_channels: 0,
///     length: 512,
///     sample_rate: 44_100.,
/// };
/// let silence = AudioBuffer::new(options);
///
/// // create a sequence of this buffer
/// let sequence = std::iter::repeat(silence).take(5);
///
/// // the sequence should actually yield `Result<AudioBuffer, _>`s
/// let sequence = sequence.map(|b| Ok(b));
///
/// // convert to a media track
/// let media = MediaStreamTrack::from_iter(sequence);
///
/// // use in the web audio context
/// let context = AudioContext::default();
/// let node = context.create_media_stream_track_source(&media);
/// node.connect(&context.destination());
///
/// loop {}
/// ```
#[derive(Debug)]
pub struct MediaStreamTrackAudioSourceNode {
    registration: AudioContextRegistration,
    channel_config: ChannelConfig,
}

impl AudioNode for MediaStreamTrackAudioSourceNode {
    fn registration(&self) -> &AudioContextRegistration {
        &self.registration
    }

    fn channel_config(&self) -> &ChannelConfig {
        &self.channel_config
    }

    fn number_of_inputs(&self) -> usize {
        0
    }

    fn number_of_outputs(&self) -> usize {
        1
    }
}

impl MediaStreamTrackAudioSourceNode {
    pub fn new<C: BaseAudioContext>(
        context: &C,
        options: MediaStreamTrackAudioSourceOptions<'_>,
    ) -> Self {
        context.base().register(move |registration| {
            let node = MediaStreamTrackAudioSourceNode {
                registration,
                channel_config: ChannelConfig::default(),
            };

            let resampler = Resampler::new(
                context.sample_rate(),
                RENDER_QUANTUM_SIZE,
                options.media_stream_track.iter(),
            );

            let render = MediaStreamRenderer::new(resampler);

            (node, Box::new(render))
        })
    }
}
