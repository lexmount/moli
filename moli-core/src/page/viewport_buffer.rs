use std::{ops::Range, sync::Arc};

use anyhow::{Context as _, Result};
use moli_page_types::ViewportSurface;
use moli_paint::{
    RasterBufferLayout, RasterPixelFormat, raster_buffer_layout, raster_damage_rows,
    raster_snapshot_rows_into,
};
use moli_renderer_v8::{
    RendererDocumentLifecycleIdentity, RendererRasterStream, RendererViewportCursor,
    RendererViewportEditor, RendererViewportFrame,
};
use parking_lot::Mutex;
use tokio::{sync::watch, task::JoinHandle};

use super::{Page, ViewportFrames};

/// Device-pixel height of an independently updated viewport band.
pub const VIEWPORT_BAND_HEIGHT: u32 = 256;

struct ViewportSlot {
    bytes: Arc<Vec<u8>>,
    versions: Option<Arc<[u64]>>,
}

#[derive(Clone)]
struct ViewportPixels {
    bytes: Arc<Vec<u8>>,
    band_versions: Arc<[u64]>,
}

/// Three caller-provided buffers for the visible frame, a reader, and a writer.
pub struct ViewportBuffers {
    layout: RasterBufferLayout,
    storage: [ViewportSlot; 3],
    last_frame: Option<RendererViewportFrame>,
    version: u64,
    band_versions: Arc<[u64]>,
}

impl ViewportBuffers {
    /// Returns the physical pixel layout required for this CSS viewport and DPR.
    pub fn layout_for(surface: ViewportSurface) -> Result<RasterBufferLayout> {
        Ok(raster_buffer_layout(
            surface.inner_width as f32,
            surface.inner_height as f32,
            surface.device_pixel_ratio as f32,
        )?)
    }

    /// Takes ownership of storage reserved by the caller. The writer initializes
    /// the bytes on first use, so vectors with reserved capacity are sufficient.
    pub fn new(layout: RasterBufferLayout, storage: [Vec<u8>; 3]) -> Result<Self> {
        anyhow::ensure!(
            storage
                .iter()
                .all(|bytes| bytes.capacity() >= layout.byte_len),
            "each viewport buffer must reserve at least {} bytes",
            layout.byte_len,
        );
        Ok(Self {
            layout,
            storage: storage.map(|bytes| ViewportSlot {
                bytes: Arc::new(bytes),
                versions: None,
            }),
            last_frame: None,
            version: 0,
            band_versions: vec![0; layout.height.div_ceil(VIEWPORT_BAND_HEIGHT) as usize].into(),
        })
    }

    fn paint(&mut self, frame: &RendererViewportFrame) -> Result<Option<ViewportPixels>> {
        let Some(snapshot) = &frame.snapshot else {
            self.last_frame = None;
            return Ok(None);
        };
        let previous = self
            .last_frame
            .as_ref()
            .filter(|last| {
                last.document == frame.document && last.viewport_revision == frame.viewport_revision
            })
            .and_then(|last| last.snapshot.as_deref());
        let damage = previous.map_or(0..self.layout.height, |previous| {
            raster_damage_rows(previous, snapshot, self.layout)
        });
        let mut versions = Arc::clone(&self.band_versions);
        if !damage.is_empty() {
            self.version = self
                .version
                .checked_add(1)
                .context("viewport band version exhausted")?;
            let first = damage.start / VIEWPORT_BAND_HEIGHT;
            let limit = damage.end.div_ceil(VIEWPORT_BAND_HEIGHT);
            Arc::make_mut(&mut versions)[first as usize..limit as usize].fill(self.version);
        }
        for slot in &mut self.storage {
            if let Some(bytes) = Arc::get_mut(&mut slot.bytes) {
                bytes.resize(self.layout.byte_len, 0);
                // A free slot may be older than the latest presented frame.
                // Track its actual pixels independently of the watch channel.
                let rows = rows_to_repaint(
                    slot.versions.take().as_deref(),
                    &versions,
                    self.layout.height,
                );
                let layout =
                    raster_snapshot_rows_into(snapshot, bytes, RasterPixelFormat::Bgra8, rows)?;
                anyhow::ensure!(
                    layout == self.layout,
                    "snapshot and viewport buffer sizes differ"
                );
                slot.versions = Some(versions.clone());
                self.last_frame = Some(frame.clone());
                self.band_versions = versions.clone();
                return Ok(Some(ViewportPixels {
                    bytes: slot.bytes.clone(),
                    band_versions: versions,
                }));
            }
        }
        anyhow::bail!("no writable viewport buffer")
    }
}

fn rows_to_repaint(previous: Option<&[u64]>, next: &[u64], height: u32) -> Range<u32> {
    let Some(previous) = previous else {
        return 0..height;
    };
    let mut changed = previous
        .iter()
        .zip(next)
        .enumerate()
        .filter_map(|(index, (a, b))| (a != b).then_some(index as u32));
    let Some(first) = changed.next() else {
        return 0..0;
    };
    let last = changed.next_back().unwrap_or(first);
    first * VIEWPORT_BAND_HEIGHT..((last + 1) * VIEWPORT_BAND_HEIGHT).min(height)
}

/// A readable view of the latest completed BGRA frame.
pub struct ViewportFrameRef<'a> {
    pub document: RendererDocumentLifecycleIdentity,
    pub viewport_revision: u64,
    pub sequence: u64,
    pub layout: RasterBufferLayout,
    pub pixels: Option<&'a [u8]>,
    /// Versions of successive full-width bands, valid for this viewport revision.
    pub band_versions: &'a [u64],
}

#[derive(Clone)]
struct ReadyFrame {
    document: RendererDocumentLifecycleIdentity,
    viewport_revision: u64,
    sequence: u64,
    layout: RasterBufferLayout,
    pixels: Option<ViewportPixels>,
}

impl ReadyFrame {
    fn new(
        frame: &RendererViewportFrame,
        layout: RasterBufferLayout,
        pixels: Option<ViewportPixels>,
    ) -> Self {
        Self {
            document: frame.document,
            viewport_revision: frame.viewport_revision,
            sequence: frame.sequence,
            layout,
            pixels,
        }
    }
}

#[derive(Default)]
struct OutputState {
    binding: u64,
    frame: Option<ReadyFrame>,
    cursor: Option<RendererViewportCursor>,
    display_tick_request: Option<u64>,
}

impl OutputState {
    fn advance_binding(&mut self) -> bool {
        self.binding += 1;
        let cleared =
            self.frame.is_some() || self.cursor.is_some() || self.display_tick_request.is_some();
        self.frame = None;
        self.cursor = None;
        self.display_tick_request = None;
        cleared
    }
}

/// Shared presentation endpoint for one tab. The watch channel carries only
/// invalidations. The pixels stay in the transferred storage until reused.
pub struct ViewportOutput {
    state: Mutex<OutputState>,
    reader: Mutex<()>,
    changes: watch::Sender<()>,
}

impl Default for ViewportOutput {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            reader: Mutex::default(),
            changes: watch::Sender::new(()),
        }
    }
}

impl ViewportOutput {
    pub fn subscribe(&self) -> watch::Receiver<()> {
        self.changes.subscribe()
    }

    pub fn cursor(&self) -> Option<RendererViewportCursor> {
        self.state.lock().cursor
    }

    pub fn display_tick_request(&self) -> Option<u64> {
        self.state.lock().display_tick_request
    }

    /// Reads a completed frame without waiting for rasterization. A callback
    /// pins one slot and does its work outside the output-state lock.
    pub fn read<R>(&self, read: impl FnOnce(ViewportFrameRef<'_>) -> R) -> Option<R> {
        let _reader = self.reader.try_lock()?;
        let frame = self.state.lock().frame.clone()?;
        Some(read(ViewportFrameRef {
            document: frame.document,
            viewport_revision: frame.viewport_revision,
            sequence: frame.sequence,
            layout: frame.layout,
            pixels: frame.pixels.as_ref().map(|pixels| pixels.bytes.as_slice()),
            band_versions: frame
                .pixels
                .as_ref()
                .map_or(&[], |pixels| pixels.band_versions.as_ref()),
        }))
    }

    fn bind(&self, display_tick_request: Option<u64>) -> u64 {
        let (binding, changed) = {
            let mut state = self.state.lock();
            let cleared = state.advance_binding();
            state.display_tick_request = display_tick_request;
            (state.binding, cleared || display_tick_request.is_some())
        };
        if changed {
            self.changes.send_replace(());
        }
        binding
    }

    fn unbind(&self, binding: u64) {
        let mut state = self.state.lock();
        if state.binding == binding {
            let cleared = state.advance_binding();
            drop(state);
            if cleared {
                self.changes.send_replace(());
            }
        }
    }

    fn publish_cursor(&self, binding: u64, cursor: Option<RendererViewportCursor>) {
        let mut state = self.state.lock();
        if state.binding == binding && state.cursor != cursor {
            state.cursor = cursor;
            drop(state);
            self.changes.send_replace(());
        }
    }

    fn publish_display_tick_request(&self, binding: u64, request: Option<u64>) {
        let mut state = self.state.lock();
        if state.binding == binding && state.display_tick_request != request {
            state.display_tick_request = request;
            drop(state);
            self.changes.send_replace(());
        }
    }

    fn publish(&self, binding: u64, frame: Option<ReadyFrame>) {
        let mut state = self.state.lock();
        if state.binding == binding && (state.frame.is_some() || frame.is_some()) {
            state.frame = frame;
            drop(state);
            self.changes.send_replace(());
        }
    }
}

/// Editor state associated with the subscribed viewport's painted document.
pub struct ViewportState {
    pub document: RendererDocumentLifecycleIdentity,
    pub sequence: u64,
    pub editor: Option<RendererViewportEditor>,
}

pub enum ViewportUpdate {
    StateChanged,
    ResourceChanged,
}

/// Keeps the buffer binding active and exposes viewport editing updates.
pub struct BufferedViewportFrames {
    frames: ViewportFrames,
    output: Arc<ViewportOutput>,
    binding: u64,
    rasterizer: JoinHandle<()>,
}

impl BufferedViewportFrames {
    pub async fn changed(&mut self) -> Result<ViewportUpdate, watch::error::RecvError> {
        loop {
            tokio::select! {
                update = self.frames.updates.changed() => return update.map(|()| ViewportUpdate::StateChanged),
                () = self.frames.resource_wake.notified() => return Ok(ViewportUpdate::ResourceChanged),
                cursor = self.frames.cursor.changed() => {
                    cursor?;
                    let cursor = *self.frames.cursor.borrow_and_update();
                    self.output.publish_cursor(self.binding, cursor);
                }
                request = self.frames.display_tick_requests.changed() => {
                    request?;
                    let request = *self.frames.display_tick_requests.borrow_and_update();
                    self.output.publish_display_tick_request(self.binding, request);
                }
            }
        }
    }

    pub fn latest_state(&mut self) -> Option<ViewportState> {
        self.frames
            .updates
            .borrow_and_update()
            .as_ref()
            .map(|frame| ViewportState {
                document: frame.document,
                sequence: frame.sequence,
                editor: frame.editor.clone(),
            })
    }
}

impl Drop for BufferedViewportFrames {
    fn drop(&mut self) {
        self.output.unbind(self.binding);
        self.rasterizer.abort();
    }
}

impl Page {
    /// Binds caller-provided pixel storage for the life of the returned subscription.
    pub async fn subscribe_viewport_buffer(
        &mut self,
        surface: ViewportSurface,
        viewport_revision: u64,
        buffers: ViewportBuffers,
        output: Arc<ViewportOutput>,
    ) -> Result<BufferedViewportFrames> {
        self.subscribe_viewport_buffer_with_pointer(
            surface,
            viewport_revision,
            None,
            buffers,
            output,
        )
        .await
    }

    /// Binds the buffers and the pointer's CSS coordinates to one viewport revision.
    pub async fn subscribe_viewport_buffer_with_pointer(
        &mut self,
        surface: ViewportSurface,
        viewport_revision: u64,
        initial_pointer_position: Option<(f64, f64)>,
        buffers: ViewportBuffers,
        output: Arc<ViewportOutput>,
    ) -> Result<BufferedViewportFrames> {
        anyhow::ensure!(
            buffers.layout == ViewportBuffers::layout_for(surface)?,
            "buffer layout does not match the viewport",
        );
        let mut frames = self
            .subscribe_viewport_frames_with_pointer(
                surface,
                viewport_revision,
                initial_pointer_position,
            )
            .await?;
        let binding = output.bind(*frames.display_tick_requests.borrow_and_update());
        let rasterizer = tokio::spawn(rasterize_frames(
            frames.updates.clone(),
            buffers,
            Arc::clone(&output),
            binding,
        ));
        Ok(BufferedViewportFrames {
            frames,
            output,
            binding,
            rasterizer,
        })
    }
}

async fn rasterize_frames(
    mut snapshots: watch::Receiver<Option<RendererViewportFrame>>,
    mut buffers: ViewportBuffers,
    output: Arc<ViewportOutput>,
    binding: u64,
) {
    let raster = RendererRasterStream::default();
    loop {
        let needs_raster = {
            let current = snapshots.borrow_and_update();
            let frame = current.as_ref();
            if frame.is_some_and(|frame| frame.snapshot.is_some()) {
                true
            } else {
                output.publish(
                    binding,
                    frame.map(|frame| ReadyFrame::new(frame, buffers.layout, None)),
                );
                false
            }
        };
        if needs_raster {
            let output = Arc::clone(&output);
            let job = async {
                raster
                    .submit(move || {
                        // The producer keeps replacing its watch value while
                        // another output has the worker. Sample at execution.
                        if output.state.lock().binding == binding {
                            let frame = snapshots.borrow_and_update().clone();
                            if let Some(frame) = frame {
                                match buffers.paint(&frame) {
                                    Ok(pixels) => {
                                        let current = snapshots.borrow();
                                        if current.as_ref().is_some_and(|current| {
                                            current.document == frame.document
                                                && current.viewport_revision
                                                    == frame.viewport_revision
                                        }) {
                                            output.publish(
                                                binding,
                                                Some(ReadyFrame::new(
                                                    &frame,
                                                    buffers.layout,
                                                    pixels,
                                                )),
                                            );
                                        }
                                    }
                                    Err(error) => {
                                        tracing::warn!(%error, "could not paint viewport buffer");
                                    }
                                }
                            } else {
                                output.publish(binding, None);
                            }
                        }
                        (snapshots, buffers)
                    })?
                    .await
                    .context("viewport raster job was cancelled")
            }
            .await;
            match job {
                Ok(latest) => (snapshots, buffers) = latest,
                Err(error) => {
                    tracing::warn!(%error);
                    break;
                }
            }
        }
        if snapshots.changed().await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_damage_checks_all_versions_of_a_stale_slot() {
        let height = 700;
        assert_eq!(rows_to_repaint(None, &[1, 2, 3], height), 0..700);
        assert_eq!(rows_to_repaint(Some(&[1, 2, 3]), &[1, 2, 3], height), 0..0);
        assert_eq!(
            rows_to_repaint(Some(&[1, 2, 3]), &[1, 5, 6], height),
            256..700
        );
        assert_eq!(
            rows_to_repaint(Some(&[1, 2, 3]), &[5, 2, 6], height),
            0..700
        );
    }
}
