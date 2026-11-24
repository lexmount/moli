use super::{PageVm, RendererLayoutMetrics};

impl PageVm {
    pub(crate) fn layout_metrics(&self) -> anyhow::Result<RendererLayoutMetrics> {
        let metrics = self.vm().document_metrics_for_current_document()?;
        let surface = self.vm().viewport_surface().unwrap_or_default();
        let mobile_viewport = surface.mobile_viewport();
        // CDP layout metrics expose the frame's scroll viewport.
        let (viewport_width, viewport_height) = if mobile_viewport.is_some() {
            (surface.inner_width, surface.inner_height)
        } else {
            (metrics.viewport.css_width, metrics.viewport.css_height)
        };
        Ok(RendererLayoutMetrics {
            viewport_width,
            viewport_height,
            page_x: f64::from(metrics.viewport_scroll.x),
            page_y: f64::from(metrics.viewport_scroll.y),
            content_width: f64::from(metrics.content_size.width),
            content_height: f64::from(metrics.content_size.height),
            device_pixel_ratio: f64::from(metrics.viewport.device_pixel_ratio),
            mobile_viewport,
        })
    }
}
