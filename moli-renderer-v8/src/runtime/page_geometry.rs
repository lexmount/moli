use anyhow::{Result, anyhow};
use moli_layout::{LayoutQuery, LayoutQueryAnswer, LayoutQueryBatch};

use super::{PageVm, RendererLayoutMetrics};

impl PageVm {
    pub(crate) fn layout_metrics(&mut self) -> Result<RendererLayoutMetrics> {
        let answers = self
            .vm_mut()
            .observable_geometry_batch_for_current_document(&LayoutQueryBatch::new(vec![
                LayoutQuery::DocumentMetrics,
            ]))?;
        let Some(LayoutQueryAnswer::DocumentMetrics(metrics)) = answers.answers.into_iter().next()
        else {
            return Err(anyhow!(
                "geometry provider returned a mismatched document metrics answer"
            ));
        };
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
