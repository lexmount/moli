use std::sync::Arc;

use crate::RgbaImage;
#[cfg(all(feature = "native-video", target_os = "macos"))]
use crate::{NativeVideoFrame, VideoFrameSource};

/// Pixels pinned to one snapshot. A video can also name the frame source
/// that a viewport compositor may use after it publishes the scene.
#[derive(Clone, Debug, PartialEq)]
pub enum ImagePixels {
    Rgba(Arc<RgbaImage>),
    #[cfg(all(feature = "native-video", target_os = "macos"))]
    Video {
        frame: Arc<NativeVideoFrame>,
        source: Option<VideoFrameSource>,
    },
}

impl ImagePixels {
    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Rgba(image) => (image.width, image.height),
            #[cfg(all(feature = "native-video", target_os = "macos"))]
            Self::Video { frame, .. } => frame.dimensions(),
        }
    }

    /// Access the exact frame in this snapshot for software painting.
    pub fn rgba(&self) -> Option<&Arc<RgbaImage>> {
        match self {
            Self::Rgba(image) => Some(image),
            #[cfg(all(feature = "native-video", target_os = "macos"))]
            Self::Video { frame, .. } => frame.rgba(),
        }
    }

    pub fn same_pixels(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Rgba(a), Self::Rgba(b)) => Arc::ptr_eq(a, b),
            #[cfg(all(feature = "native-video", target_os = "macos"))]
            (Self::Video { frame: a, .. }, Self::Video { frame: b, .. }) => Arc::ptr_eq(a, b),
            #[cfg(all(feature = "native-video", target_os = "macos"))]
            _ => false,
        }
    }
}

impl From<Arc<RgbaImage>> for ImagePixels {
    fn from(image: Arc<RgbaImage>) -> Self {
        Self::Rgba(image)
    }
}
