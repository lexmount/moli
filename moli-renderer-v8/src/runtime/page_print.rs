//! Native PDF requests, replies and isolated print-layout lifecycle.

use super::PageVm;
use crate::native_bridge::LayoutPublicationRequest;
use crate::runtime::RendererScreenshotPurpose;
use moli_layout::{PaintCaptureRequest, PaintViewport};
use moli_page_types::LayoutPolicy;

pub use moli_paint::pdf::PdfOptions as RendererPdfOptions;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RendererPrintToPdfRequest {
    pub options: RendererPdfOptions,
    pub print_background: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RendererPrintToPdfReply {
    Printed(Vec<u8>),
    Failed(String),
    LayoutDisabled,
    NoDocument,
}

impl PageVm {
    /// Projects print media and encodes PDF without allocating a full-document
    /// bitmap or publishing interactive screen geometry. Both output clients
    /// use this lifecycle, including restoration before any encoding error.
    pub(super) fn print_to_pdf(
        &mut self,
        request: RendererPrintToPdfRequest,
    ) -> anyhow::Result<RendererPrintToPdfReply> {
        use RendererPrintToPdfReply as Reply;
        self.flush_page_action_window(moli_action_window::ActionBarrier::Explicit)?;
        if self.layout_configuration.policy == LayoutPolicy::Mock {
            return Ok(Reply::LayoutDisabled);
        }
        let environment_override =
            self.capture_style_environment_override(RendererScreenshotPurpose::Print {
                print_background: request.print_background,
            });
        let surface = self.document_settings.viewport_surface.unwrap_or_default();
        let viewport = PaintViewport::new(
            surface.inner_width,
            surface.inner_height,
            surface.device_pixel_ratio as f32,
        );
        let result = self.vm_mut().paint_layout_snapshot_with_capture(
            viewport,
            moli_layout::LayoutFlushReason::Print,
            PaintCaptureRequest {
                include_backgrounds: request.print_background,
                ..PaintCaptureRequest::print_document()
            },
            LayoutPublicationRequest::Transient,
            environment_override,
        );
        Ok(match result? {
            Some(snapshot) => match moli_paint::pdf::pdf_snapshot(&snapshot, &request.options) {
                Ok(bytes) => Reply::Printed(bytes),
                Err(error) => Reply::Failed(error.to_string()),
            },
            None => Reply::NoDocument,
        })
    }
}
