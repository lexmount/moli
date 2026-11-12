use crate::context::{AudioContextRegistration, BaseAudioContext};
use crate::media_streams::MediaStream;
use crate::resampling::Resampler;
use crate::RENDER_QUANTUM_SIZE;

use super::{AudioNode, ChannelConfig, MediaStreamRenderer};

/// Options for constructing a [`MediaStreamAudioSourceNode`]
// dictionary MediaStreamAudioSourceOptions {
//   required MediaStream mediaStream;
// };
//
// @note - Does not extend AudioNodeOptions because AudioNodeOptions are
// useless for source nodes as they instruct how to upmix the inputs.
// This is a common source of confusion, see e.g. https://github.com/mdn/content/pull/18472
#[derive(Debug)]
pub struct MediaStreamAudioSourceOptions<'a> {
    pub media_stream: &'a MediaStream,
}

/// An audio source from a [`MediaStream`] (e.g. microphone input)
///
/// IMPORTANT: the media stream is polled on the render thread so you must ensure the media stream
/// iterator never blocks. Use a
/// [`MediaElementAudioSourceNode`](crate::node::MediaElementAudioSourceNode) for real time safe
/// media playback.
#[derive(Debug)]
pub struct MediaStreamAudioSourceNode {
    registration: AudioContextRegistration,
    channel_config: ChannelConfig,
}

impl AudioNode for MediaStreamAudioSourceNode {
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

impl MediaStreamAudioSourceNode {
    /// Create a new `MediaStreamAudioSourceNode`
    ///
    /// # Panics
    ///
    /// This method will panic when the provided `MediaStream` does not contain any audio tracks.
    pub fn new<C: BaseAudioContext>(
        context: &C,
        options: MediaStreamAudioSourceOptions<'_>,
    ) -> Self {
        context.base().register(move |registration| {
            let node = MediaStreamAudioSourceNode {
                registration,
                channel_config: ChannelConfig::default(),
            };

            let resampler = Resampler::new(
                context.sample_rate(),
                RENDER_QUANTUM_SIZE,
                options.media_stream.get_tracks()[0].iter(),
            );

            let render = MediaStreamRenderer::new(resampler);

            (node, Box::new(render))
        })
    }
}
