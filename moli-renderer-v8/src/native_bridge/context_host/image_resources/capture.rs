use std::{fmt, sync::Arc};

use super::{
    ImageResponseDescriptor,
    decode::{DecodedImageContent, ReadyDecodedImage},
    state::{ReadyImageForLayout, ReadyImageResource, intrinsic_dimensions},
};

/// Capture ownership includes the decoded-byte permit. A projected pixel/SVG
/// Arc alone does not account for the lifetime of those bytes.
#[derive(Clone)]
pub(crate) struct CaptureReadyImage {
    resource: Arc<ReadyImageResource>,
}

impl CaptureReadyImage {
    pub(super) fn new(descriptor: ImageResponseDescriptor, decoded: ReadyDecodedImage) -> Self {
        let (pixels, svg) = match decoded.content {
            DecodedImageContent::Raster(pixels) => (Some(pixels), None),
            DecodedImageContent::Svg(svg) => (None, Some(svg)),
        };
        Self::from_resource(Arc::new(ReadyImageResource {
            descriptor,
            density: 1.0,
            pixels,
            svg,
            _decoded_bytes_permit: Some(decoded.decoded_bytes_permit),
        }))
    }

    pub(super) fn from_resource(resource: Arc<ReadyImageResource>) -> Self {
        Self { resource }
    }

    pub(crate) fn for_layout(&self) -> ReadyImageForLayout {
        let (intrinsic_width, intrinsic_height) = intrinsic_dimensions(&self.resource);
        ReadyImageForLayout {
            intrinsic_width,
            intrinsic_height,
            pixels: self.resource.pixels.clone(),
            svg: self.resource.svg.clone(),
        }
    }
}

impl fmt::Debug for CaptureReadyImage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CaptureReadyImage")
            .field("descriptor", &self.resource.descriptor)
            .finish_non_exhaustive()
    }
}

impl super::super::JsContextHost {
    pub(crate) fn ready_css_image_for_capture(
        &self,
        document: crate::document_runtime::DomHandle,
        resolved_url: &str,
    ) -> Option<CaptureReadyImage> {
        self.image_resources
            .css
            .ready_for_capture(document, resolved_url)
    }
}
