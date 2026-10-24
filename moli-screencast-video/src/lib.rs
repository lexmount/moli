//! CPU-only H.264 recording. Native encoder code is distributed under the
//! x264 license in `native/COPYING.x264`; see `native/README.md`.

mod encoder;
mod recorder;
mod writer;

pub use recorder::{RecorderHandle, RecorderStatus, VideoRecorder};
pub use writer::{VideoWriter, VideoWriterStats};
