#![cfg(all(
    feature = "media-ext",
    any(target_os = "linux", target_os = "macos"),
    target_pointer_width = "64"
))]

mod audio;
mod fragmented;
mod h264;
mod mp4;
mod worker;

pub use audio::AudioPlayback;
pub use fragmented::FragmentAppend;
pub use h264::VideoFrame;
pub use mp4::{Mp4Decoder, VideoMetadata};
pub use worker::{DecodeRequest, DecodeResult, VideoWorker};
