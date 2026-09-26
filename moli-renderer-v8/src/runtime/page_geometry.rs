use super::{PageVm, RendererLayoutMetrics};

impl PageVm {
    pub(crate) fn layout_metrics(&self) -> anyhow::Result<RendererLayoutMetrics> {
        let metrics = self.vm().document_metrics_for_current_document()?;
        Ok(RendererLayoutMetrics {
            viewport_width: metrics.viewport.css_width,
            viewport_height: metrics.viewport.css_height,
            page_x: f64::from(metrics.viewport_scroll.x),
            page_y: f64::from(metrics.viewport_scroll.y),
            content_width: f64::from(metrics.content_size.width),
            content_height: f64::from(metrics.content_size.height),
            device_pixel_ratio: f64::from(metrics.viewport.device_pixel_ratio),
        })
    }
}
