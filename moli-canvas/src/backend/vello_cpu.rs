//! Reusable Vello CPU rendering backend for the Canvas surface.
//!
//! This owns a single [`VelloCpuScenePainter`] sized to the canvas and reuses
//! it across flushes. Rendering composites into the caller-provided buffer
//! using either:
//!
//! - [`CompositeMode::SrcOver`] to draw a batch of source-over operations onto
//!   the *existing* canonical surface, preserving prior content without
//!   clearing — the incremental path the proposal requires; or
//! - [`CompositeMode::Replace`] for a destructive step (e.g. a clear or a proven
//!   full overwrite), which clears the whole destination pixmap and renders the
//!   scene into it.
//!
//! The canonical surface format is premultiplied RGBA8 (see `surface.rs`); Vello
//! `SrcOver` composites premultiplied sources over premultiplied destinations,
//! which matches the authoritative store directly.

use anyrender::PaintScene;
use anyrender_vello_cpu::VelloCpuScenePainter;
use vello_cpu::{
    CompositeMode, PixelFormat, PixmapMut, RasterizerSettings, RenderContext as VelloRenderContext,
    RenderMode, Resources,
};

/// A reusable Vello CPU renderer bound to a canvas of `width` by `height`.
pub struct VelloCpuBackend {
    renderer: VelloCpuScenePainter,
    width: u32,
    height: u32,
}

impl VelloCpuBackend {
    /// Creates a backend for a canvas of the given size (in pixels).
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            renderer: VelloCpuScenePainter {
                render_ctx: VelloRenderContext::new(width as u16, height as u16),
                resources: Resources::new(),
            },
            width,
            height,
        }
    }

    /// Whether this backend is bound to the given canvas size.
    pub fn matches(&self, width: u32, height: u32) -> bool {
        self.width == width && self.height == height
    }

    /// Rebinds the backend to a different canvas size, discarding prior scene
    /// state. Callers must also replace the backing surface storage.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.renderer.render_ctx = VelloRenderContext::new(width as u16, height as u16);
        self.width = width;
        self.height = height;
    }

    /// Renders a scene built by `draw` into `target` (premultiplied RGBA8 of
    /// `width * height * 4` bytes) using the given composite mode. With
    /// `SrcOver`, content already in `target` is preserved underneath the batch.
    pub fn render<F>(&mut self, target: &mut [u8], composite: CompositeMode, draw: F)
    where
        F: FnOnce(&mut VelloCpuScenePainter),
    {
        debug_assert_eq!(
            target.len(),
            self.width as usize * self.height as usize * 4,
            "target must span the whole backend surface"
        );
        // Reset the mutable scene but keep the renderer/resources for reuse.
        self.renderer.reset();
        draw(&mut self.renderer);
        self.renderer.render_ctx.flush();
        let pixmap = PixmapMut::new(self.width as u16, self.height as u16, target)
            .expect("backend provides a correctly sized RGBA8 target");
        let settings = RasterizerSettings {
            render_mode: RenderMode::OptimizeSpeed,
            composite_mode: composite,
            pixel_format: PixelFormat::Rgba8,
            offset: (0, 0),
        };
        self.renderer
            .render_ctx
            .render_with(pixmap, &mut self.renderer.resources, settings);
    }
}
