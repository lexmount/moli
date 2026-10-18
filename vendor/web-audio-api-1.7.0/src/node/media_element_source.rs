use crate::context::{AudioContextRegistration, BaseAudioContext};
use crate::resampling::Resampler;
use crate::MediaElement;
use crate::RENDER_QUANTUM_SIZE;

use super::{AudioNode, ChannelConfig, MediaStreamRenderer};

/// Options for constructing a [`MediaElementAudioSourceNode`]
// dictionary MediaElementAudioSourceOptions {
//     required HTMLMediaElement mediaElement;
// };
//
// @note - Does not extend AudioNodeOptions because AudioNodeOptions are
// useless for source nodes as they instruct how to upmix the inputs.
// This is a common source of confusion, see e.g. https://github.com/mdn/content/pull/18472
#[derive(Debug)]
pub struct MediaElementAudioSourceOptions<'a> {
    pub media_element: &'a mut MediaElement,
}

/// An audio source from an `<audio>` element
///
/// - MDN documentation:
///   <https://developer.mozilla.org/en-US/docs/Web/API/MediaElementAudioSourceNode>
/// - specification: <https://webaudio.github.io/web-audio-api/#MediaElementAudioSourceNode>
/// - see also:
///   [`AudioContext::create_media_element_source`](crate::context::AudioContext::create_media_element_source)
///
/// # Usage
///
/// ```no_run
/// use web_audio_api::context::{AudioContext, BaseAudioContext};
/// use web_audio_api::MediaElement;
/// use web_audio_api::node::AudioNode;
///
/// let context = AudioContext::default();
/// let mut media = MediaElement::new("samples/major-scale.ogg").unwrap();
///
/// let mut src = context.create_media_element_source(&mut media);
/// src.connect(&context.destination());
///
/// media.set_loop(true); // continuously loop
/// media.set_current_time(1.0); // seek to offset
/// media.play(); // start playing
///
/// loop {}
/// ```
///
/// # Examples
///
/// - `cargo run --release --example media_element`
#[derive(Debug)]
pub struct MediaElementAudioSourceNode {
    registration: AudioContextRegistration,
    channel_config: ChannelConfig,
}

impl AudioNode for MediaElementAudioSourceNode {
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

impl MediaElementAudioSourceNode {
    /// Create a new `MediaElementAudioSourceNode`
    ///
    /// # Panics
    ///
    /// This method will panic when there already exists a source node for the given
    /// `MediaElement`. You can only set up a single source node per element!
    pub fn new<C: BaseAudioContext>(
        context: &C,
        options: MediaElementAudioSourceOptions<'_>,
    ) -> Self {
        context.base().register(move |registration| {
            let node = MediaElementAudioSourceNode {
                registration,
                channel_config: ChannelConfig::default(),
            };

            let stream = options
                .media_element
                .take_stream()
                .expect("InvalidStateError - stream already taken");

            let resampler = Resampler::new(context.sample_rate(), RENDER_QUANTUM_SIZE, stream);

            let render = MediaStreamRenderer::new(resampler);

            (node, Box::new(render))
        })
    }
}
