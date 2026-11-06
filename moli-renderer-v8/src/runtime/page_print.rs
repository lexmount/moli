//! Native PDF requests, replies and isolated print-layout lifecycle.

use super::PageVm;
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
        let environment_override = if self.document_settings.emulated_media.media.is_none() {
            let mut print = self.document_settings.emulated_media.clone();
            print.media = Some("print".to_owned());
            Some(crate::style_engine::StyloStyleEnvironment::from_emulated_media(&print))
        } else {
            None
        };
        let surface = self.document_settings.viewport_surface.unwrap_or_default();
        let viewport = PaintViewport::new(
            surface.inner_width,
            surface.inner_height,
            surface.device_pixel_ratio as f32,
        );
        let result = self
            .vm_mut()
            .paint_layout_snapshot_with_capture_in_environment(
                viewport,
                moli_layout::LayoutFlushReason::Print,
                PaintCaptureRequest {
                    include_backgrounds: request.print_background,
                    ..PaintCaptureRequest::print_document()
                },
                environment_override,
            );
        Ok(match result? {
            Some(snapshot) => match moli_paint::pdf::pdf_snapshot(&snapshot, &request.options) {
                Ok(bytes) => {
                    for diagnostic in moli_paint::pdf::pdf_snapshot_diagnostics(&snapshot) {
                        tracing::warn!(
                            target: "moli_paint",
                            code = %diagnostic.code,
                            "{}",
                            diagnostic.message
                        );
                    }
                    Reply::Printed(bytes)
                }
                Err(error) => Reply::Failed(error.to_string()),
            },
            None => Reply::NoDocument,
        })
    }
}
