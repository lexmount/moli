//! Primitives of the MediaDevices API
//!
//! The MediaDevices interface provides access to connected media input devices like microphones.
//!
//! <https://developer.mozilla.org/en-US/docs/Web/API/MediaDevices>

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::context::{AudioContextLatencyCategory, AudioContextOptions};
use crate::io::BackendResult;
use crate::media_streams::MediaStream;

/// List the available media output devices, such as speakers, headsets, loopbacks, etc
///
/// The media device_id can be used to specify the [`sink_id` of the `AudioContext`](crate::context::AudioContextOptions::sink_id)
///
/// ```no_run
/// use web_audio_api::media_devices::{enumerate_devices_sync, MediaDeviceInfoKind};
///
/// let devices = enumerate_devices_sync();
/// assert_eq!(devices[0].device_id(), "1");
/// assert_eq!(devices[0].group_id(), None);
/// assert_eq!(devices[0].kind(), MediaDeviceInfoKind::AudioOutput);
/// assert_eq!(devices[0].label(), "Macbook Pro Builtin Speakers");
/// ```
pub fn enumerate_devices_sync() -> Vec<MediaDeviceInfo> {
    try_enumerate_devices_sync().unwrap_or_else(|e| {
        log::error!("Unable to enumerate media devices: {e}");
        vec![]
    })
}

pub(crate) fn try_enumerate_devices_sync() -> BackendResult<Vec<MediaDeviceInfo>> {
    crate::io::enumerate_devices_sync()
}

// Internal struct to derive a stable id for a given input / output device
// cf. https://github.com/orottier/web-audio-api-rs/issues/356
#[derive(Hash)]
pub(crate) struct DeviceId {
    kind: MediaDeviceInfoKind,
    host: String,
    device_name: String,
    num_channels: u16,
    index: u8,
}

impl DeviceId {
    pub(crate) fn as_string(
        kind: MediaDeviceInfoKind,
        host: String,
        device_name: String,
        num_channels: u16,
        index: u8,
    ) -> String {
        let device_info = Self {
            kind,
            host,
            device_name,
            num_channels,
            index,
        };

        let mut hasher = DefaultHasher::new();
        device_info.hash(&mut hasher);
        format!("{}", hasher.finish())
    }
}

/// Describes input/output type of a media device
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum MediaDeviceInfoKind {
    VideoInput,
    AudioInput,
    AudioOutput,
}

/// Describes a single media input or output device
///
/// Call [`enumerate_devices_sync`] to obtain a list of devices for your hardware.
#[derive(Debug)]
pub struct MediaDeviceInfo {
    device_id: String,
    group_id: Option<String>,
    kind: MediaDeviceInfoKind,
    label: String,
}

impl MediaDeviceInfo {
    pub(crate) fn new(
        device_id: String,
        group_id: Option<String>,
        kind: MediaDeviceInfoKind,
        label: String,
    ) -> Self {
        Self {
            device_id,
            group_id,
            kind,
            label,
        }
    }

    /// Identifier for the represented device
    ///
    /// The current implementation is not stable across sessions so you should not persist this
    /// value
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Two devices have the same group identifier if they belong to the same physical device
    pub fn group_id(&self) -> Option<&str> {
        self.group_id.as_deref()
    }

    /// Enumerated value that is either "videoinput", "audioinput" or "audiooutput".
    pub fn kind(&self) -> MediaDeviceInfoKind {
        self.kind
    }

    /// Friendly label describing this device
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// Dictionary used to instruct what sort of tracks to include in the [`MediaStream`] returned by
/// [`get_user_media_sync`]
#[derive(Clone, Debug)]
pub enum MediaStreamConstraints {
    Audio,
    AudioWithConstraints(MediaTrackConstraints),
}

/// Desired media stream track settings for [`MediaTrackConstraints`]
#[derive(Default, Debug, Clone)]
#[non_exhaustive]
pub struct MediaTrackConstraints {
    // ConstrainULong width;
    // ConstrainULong height;
    // ConstrainDouble aspectRatio;
    // ConstrainDouble frameRate;
    // ConstrainDOMString facingMode;
    // ConstrainDOMString resizeMode;
    pub sample_rate: Option<f32>,
    // ConstrainULong sampleSize;
    // ConstrainBoolean echoCancellation;
    // ConstrainBoolean autoGainControl;
    // ConstrainBoolean noiseSuppression;
    pub latency: Option<f64>,
    pub channel_count: Option<u32>, // TODO model as ConstrainULong;
    pub device_id: Option<String>,
    // ConstrainDOMString groupId;
}

impl From<MediaTrackConstraints> for AudioContextOptions {
    fn from(value: MediaTrackConstraints) -> Self {
        let latency_hint = match value.latency {
            Some(v) => AudioContextLatencyCategory::Custom(v),
            None => AudioContextLatencyCategory::Interactive,
        };
        let sink_id = value.device_id.unwrap_or(String::from(""));

        AudioContextOptions {
            latency_hint,
            sample_rate: value.sample_rate,
            sink_id,
            render_size_hint: Default::default(),
        }
    }
}

/// Check if the provided device_id is available for playback
///
/// It should be "" or a valid input `deviceId` returned from [`enumerate_devices_sync`]
fn is_valid_device_id(device_id: &str) -> bool {
    if device_id.is_empty() {
        true
    } else {
        enumerate_devices_sync()
            .into_iter()
            .filter(|d| d.kind == MediaDeviceInfoKind::AudioInput)
            .any(|d| d.device_id() == device_id)
    }
}

/// Prompt for permission to use a media input (audio only)
///
/// This produces a [`MediaStream`] with tracks containing the requested types of media, which can
/// be used inside a [`MediaStreamAudioSourceNode`](crate::node::MediaStreamAudioSourceNode).
///
/// It is okay for the `MediaStream` struct to go out of scope, any corresponding stream will still be
/// kept alive and emit audio buffers. Call the `close()` method if you want to stop the media
/// input and release all system resources.
///
/// This function operates synchronously, which may be undesirable on the control thread. An async
/// version is currently not implemented.
///
/// # Panics
///
/// This function will panic when the selected audio backend cannot create or start the input
/// stream. A public `try_get_user_media_sync` could be added in the future to handle these errors
/// without panicking, similar to [`AudioContext::try_new`](crate::context::AudioContext::new).
///
/// Input streams with more than [`MAX_CHANNELS`](crate::MAX_CHANNELS) are not
/// supported. A panic will occur when the hardware is unable to serve a stream
/// with lower channel count.
///
/// # Example
///
/// ```no_run
/// use web_audio_api::context::{BaseAudioContext, AudioContext};
/// use web_audio_api::context::{AudioContextLatencyCategory, AudioContextOptions};
/// use web_audio_api::media_devices;
/// use web_audio_api::media_devices::MediaStreamConstraints;
/// use web_audio_api::node::AudioNode;
///
/// let context = AudioContext::default();
/// let mic = media_devices::get_user_media_sync(MediaStreamConstraints::Audio);
///
/// // register as media element in the audio context
/// let background = context.create_media_stream_source(&mic);
///
/// // connect the node directly to the destination node (speakers)
/// background.connect(&context.destination());
///
/// // enjoy listening
/// std::thread::sleep(std::time::Duration::from_secs(4));
/// ```
pub fn get_user_media_sync(constraints: MediaStreamConstraints) -> MediaStream {
    try_get_user_media_sync(constraints).unwrap_or_else(|e| panic!("InvalidStateError - {e}"))
}

pub(crate) fn try_get_user_media_sync(
    constraints: MediaStreamConstraints,
) -> BackendResult<MediaStream> {
    let (channel_count, mut options) = match constraints {
        MediaStreamConstraints::Audio => (None, AudioContextOptions::default()),
        MediaStreamConstraints::AudioWithConstraints(cs) => (cs.channel_count, cs.into()),
    };

    if !is_valid_device_id(&options.sink_id) {
        log::error!("NotFoundError: invalid deviceId {:?}", options.sink_id);
        options.sink_id = String::from("");
    }

    crate::io::build_input(options, channel_count)
}
