//! Authoritative Canvas pixel surface: ownership, flush, snapshot, readback,
//! and reset.
//!
//! This is the single writable pixel store for a Canvas. Its format is
//! **premultiplied RGBA8** (matching the Vello backend's native output);
//! conversion to straight RGBA8 happens only at the boundaries that publish
//! pixels (region readback, `ImageData`, encoding, page snapshots).
//!
//! The surface lazily materializes its buffer and a reusable rendering backend
//! on first flush, reuses both across successive flushes, keeps one immutable
//! published snapshot, and never retains a history of operations.

use std::sync::Arc;

use anyrender_vello_cpu::VelloCpuScenePainter;
use moli_image::RgbaImage;
use vello_cpu::CompositeMode;

use crate::backend::VelloCpuBackend;
use crate::pixel::{premultiply_rgba8_in_place, unpremultiply_rgba8_in_place};
use crate::types::byte_len;

/// Largest edge dimension Vello CPU can address (its contexts use `u16`).
const MAX_VELLO_EDGE: u32 = u16::MAX as u32;

/// Failure while provisioning or exposing a canvas surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasSurfaceError {
    /// The dimensions exceed the rasterizer or byte-length budget.
    SurfaceTooLarge { width: u32, height: u32 },
    /// A snapshot or allocation could not be produced (resource limit).
    AllocationFailed,
}

impl std::fmt::Display for CanvasSurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CanvasSurfaceError::SurfaceTooLarge { width, height } => {
                write!(
                    f,
                    "canvas surface {width}x{height} exceeds the supported budget"
                )
            }
            CanvasSurfaceError::AllocationFailed => write!(f, "canvas surface allocation failed"),
        }
    }
}

impl std::error::Error for CanvasSurfaceError {}

/// An authoritative, premultiplied-RGBA8 Canvas pixel surface.
pub struct CanvasSurface {
    width: u32,
    height: u32,
    /// Premultiplied RGBA8, row major. Allocated lazily on first use.
    pixels: Vec<u8>,
    /// Reusable Vello CPU backend, created lazily on first flush.
    backend: Option<VelloCpuBackend>,
    /// Immutable straight-alpha snapshot, cached so repeated clean reads
    /// perform no further rasterization or conversion.
    published: Option<Arc<RgbaImage>>,
    dirty: bool,
    /// Number of backend submissions (flushes) performed.
    flush_count: u64,
    /// Number of full-surface straight-alpha conversions performed.
    snapshot_count: u64,
}

impl CanvasSurface {
    /// Creates a surface with the given dimensions. Storage and backend are
    /// materialized lazily when content is first rendered.
    pub fn new(width: u32, height: u32) -> Result<Self, CanvasSurfaceError> {
        byte_len(width, height).ok_or(CanvasSurfaceError::SurfaceTooLarge { width, height })?;
        if width > MAX_VELLO_EDGE || height > MAX_VELLO_EDGE {
            return Err(CanvasSurfaceError::SurfaceTooLarge { width, height });
        }
        let len = byte_len(width, height).expect("validated byte length");
        Ok(Self {
            width,
            height,
            pixels: vec![0; len],
            backend: None,
            published: None,
            dirty: false,
            flush_count: 0,
            snapshot_count: 0,
        })
    }

    /// The surface width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The surface height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Whether the surface has any materialized content (allocated pixels).
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The authoritative premultiplied-RGBA8 pixel buffer.
    pub fn premultiplied(&self) -> &[u8] {
        &self.pixels
    }

    /// Number of backend submissions (flushes) performed since reset.
    pub fn flush_count(&self) -> u64 {
        self.flush_count
    }

    /// Number of full-surface straight-alpha conversions (snapshot creations)
    /// performed since reset. Clean repeated reads via [`Self::snapshot`] reuse
    /// the cached image and do not advance this counter.
    pub fn snapshot_count(&self) -> u64 {
        self.snapshot_count
    }

    /// Whether a flush has occurred since the last observation.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Marks the surface as observed (no longer dirty) without touching pixels.
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Discards all pending and rendered content, restoring a transparent
    /// surface. Reuses storage/backend where possible.
    pub fn reset(&mut self) {
        self.pixels.fill(0);
        self.published = None;
        self.dirty = true;
        self.flush_count = 0;
        self.snapshot_count = 0;
    }

    /// Resizes the surface, resetting content and re-provisioning storage and
    /// the backend when dimensions change (same-size assignment still clears).
    /// Returns `Err` when the new dimensions exceed the budget; the previous
    /// surface is left untouched in that case.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), CanvasSurfaceError> {
        byte_len(width, height).ok_or(CanvasSurfaceError::SurfaceTooLarge { width, height })?;
        if width > MAX_VELLO_EDGE || height > MAX_VELLO_EDGE {
            return Err(CanvasSurfaceError::SurfaceTooLarge { width, height });
        }
        let len = byte_len(width, height).expect("validated byte length");
        let same = self.width == width && self.height == height;
        if !same {
            self.pixels = vec![0; len];
            if let Some(mut backend) = self.backend.take() {
                backend.resize(width, height);
                self.backend = Some(backend);
            }
        } else {
            // Same-size assignment still clears content per reset semantics.
            self.pixels.fill(0);
        }
        self.width = width;
        self.height = height;
        self.published = None;
        self.dirty = true;
        self.flush_count = 0;
        self.snapshot_count = 0;
        Ok(())
    }

    /// Flushes a source-over batch of operations against the existing surface,
    /// preserving prior content. The closure builds the scene against the Vello
    /// scene painter (AnyRender [`PaintScene`]); it is executed against the
    /// authoritative premultiplied buffer and reuses the backend across calls.
    ///
    /// On success the surface is marked dirty and any cached snapshot is
    /// invalidated. On failure the surface is unchanged and not marked dirty,
    /// so old pixels are never published as successful new content.
    pub fn render<F>(&mut self, draw: F) -> Result<(), CanvasSurfaceError>
    where
        F: FnOnce(&mut VelloCpuScenePainter),
    {
        self.render_composite(CompositeMode::SrcOver, draw)
    }

    /// Flushes operations with destructive full overwrite semantics: the whole
    /// destination is replaced (cleared) by the rendered scene, not composited
    /// over existing content. Use for a proven full-canvas overwrite or a clear
    /// that the source-over path cannot express.
    pub fn render_replace<F>(&mut self, draw: F) -> Result<(), CanvasSurfaceError>
    where
        F: FnOnce(&mut VelloCpuScenePainter),
    {
        self.render_composite(CompositeMode::Replace, draw)
    }

    fn render_composite<F>(
        &mut self,
        composite: CompositeMode,
        draw: F,
    ) -> Result<(), CanvasSurfaceError>
    where
        F: FnOnce(&mut VelloCpuScenePainter),
    {
        if self.is_empty() {
            return Ok(());
        }
        if self.backend.is_none() {
            self.backend = Some(VelloCpuBackend::new(self.width, self.height));
        }
        let backend = self.backend.as_mut().expect("backend provisioned");
        let buffer = &mut self.pixels;
        if composite == CompositeMode::Replace {
            buffer.fill(0);
        }
        backend.render(buffer, composite, draw);
        self.published = None;
        self.dirty = true;
        self.flush_count = self.flush_count.saturating_add(1);
        Ok(())
    }

    /// Destructively clears the whole surface to transparent without a raster
    /// step. Invalidates any cached snapshot and marks the surface dirty.
    pub fn clear(&mut self) {
        self.pixels.fill(0);
        self.published = None;
        self.dirty = true;
        self.flush_count = self.flush_count.saturating_add(1);
    }

    /// Transitional adapter for the existing straight-RGBA8 draw helpers.
    ///
    /// Runs `f` against the whole surface as straight (non-premultiplied) RGBA8,
    /// then converts back to the authoritative premultiplied store. This keeps
    /// the immediate-execution draw paths producing byte-identical results while
    /// the surface remains the single owner; M4's ordered recorder replaces this
    /// per-call full-plane conversion with batched Vello rendering. The cached
    /// snapshot is invalidated.
    pub fn with_straight_pixels_mut(&mut self, f: impl FnOnce(&mut [u8], u32, u32)) -> Option<()> {
        if self.is_empty() {
            return None;
        }
        unpremultiply_rgba8_in_place(&mut self.pixels)?;
        f(&mut self.pixels, self.width, self.height);
        premultiply_rgba8_in_place(&mut self.pixels)?;
        self.published = None;
        self.dirty = true;
        Some(())
    }

    /// An immutable straight-alpha snapshot of the whole surface, cached so
    /// repeated clean observations perform no further rasterization. The
    /// snapshot remains valid (unchanged) after subsequent drawing.
    pub fn snapshot(&mut self) -> Result<Arc<RgbaImage>, CanvasSurfaceError> {
        if let Some(published) = &self.published {
            return Ok(published.clone());
        }
        let mut straight = self.pixels.clone();
        unpremultiply_rgba8_in_place(&mut straight)
            .expect("surface byte length is a multiple of four");
        let image = RgbaImage::try_new(self.width, self.height, straight)
            .map_err(|_| CanvasSurfaceError::AllocationFailed)?;
        let published = Arc::new(image);
        self.published = Some(published.clone());
        self.snapshot_count = self.snapshot_count.saturating_add(1);
        Ok(published)
    }

    /// Copies a rectangular region out of the surface as straight RGBA8 into a
    /// freshly allocated buffer. Out-of-canvas regions are filled transparent
    /// (matching `getImageData` semantics). Only the visible intersection is
    /// read, so a small region does not require a full-frame intermediate copy.
    pub fn readback_region(&self, x: i32, y: i32, width: u32, height: u32) -> Vec<u8> {
        let mut out = vec![0; (width as usize) * (height as usize) * 4];
        if self.is_empty() {
            return out;
        }
        let src_row_stride = self.width as usize * 4;
        let dst_row_stride = width as usize * 4;
        for row in 0..height as usize {
            let src_y = y + row as i32;
            if src_y < 0 || src_y >= self.height as i32 {
                continue;
            }
            for col in 0..width as usize {
                let src_x = x + col as i32;
                if src_x < 0 || src_x >= self.width as i32 {
                    continue;
                }
                let src_index = (src_y as usize) * src_row_stride + (src_x as usize) * 4;
                let dst_index = row * dst_row_stride + col * 4;
                let px = &self.pixels[src_index..src_index + 4];
                let dst = &mut out[dst_index..dst_index + 4];
                let alpha = u32::from(px[3]);
                dst[3] = px[3];
                if alpha == 0 {
                    dst[0] = 0;
                    dst[1] = 0;
                    dst[2] = 0;
                } else if alpha == u32::from(u8::MAX) {
                    dst[0] = px[0];
                    dst[1] = px[1];
                    dst[2] = px[2];
                } else {
                    dst[0] = ((u32::from(px[0]) * 255 + alpha / 2) / alpha) as u8;
                    dst[1] = ((u32::from(px[1]) * 255 + alpha / 2) / alpha) as u8;
                    dst[2] = ((u32::from(px[2]) * 255 + alpha / 2) / alpha) as u8;
                }
            }
        }
        out
    }
}
