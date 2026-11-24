use std::sync::Arc;

use anyhow::{Context as _, Result};
use moli_page_types::ViewportSurface;
use moli_paint::{
    RasterBufferLayout, RasterPixelFormat, raster_buffer_layout, raster_snapshot_into,
};
use moli_renderer_v8::{
    RendererDocumentLifecycleIdentity, RendererRasterStream, RendererViewportCursor,
    RendererViewportEditor, RendererViewportFrame,
};
use parking_lot::Mutex;
use tokio::{sync::watch, task::JoinHandle};

use super::{Page, ViewportFrames};

/// Three caller-provided buffers for the visible frame, a reader, and a writer.
pub struct ViewportBuffers {
    layout: RasterBufferLayout,
    storage: [Arc<Vec<u8>>; 3],
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
            storage: storage.map(Arc::new),
        })
    }

    fn paint(&mut self, frame: &RendererViewportFrame) -> Result<Option<Arc<Vec<u8>>>> {
        let Some(snapshot) = &frame.snapshot else {
            return Ok(None);
        };
        for slot in &mut self.storage {
            if let Some(bytes) = Arc::get_mut(slot) {
                bytes.resize(self.layout.byte_len, 0);
                let layout = raster_snapshot_into(snapshot, bytes, RasterPixelFormat::Bgra8)?;
                anyhow::ensure!(
                    layout == self.layout,
                    "snapshot and viewport buffer sizes differ"
                );
                return Ok(Some(Arc::clone(slot)));
            }
        }
        anyhow::bail!("no writable viewport buffer")
    }
}

/// A readable view of the latest completed BGRA frame.
pub struct ViewportFrameRef<'a> {
    pub document: RendererDocumentLifecycleIdentity,
    pub viewport_revision: u64,
    pub sequence: u64,
    pub layout: RasterBufferLayout,
    pub pixels: Option<&'a [u8]>,
}

#[derive(Clone)]
struct ReadyFrame {
    document: RendererDocumentLifecycleIdentity,
    viewport_revision: u64,
    sequence: u64,
    layout: RasterBufferLayout,
    pixels: Option<Arc<Vec<u8>>>,
}

impl ReadyFrame {
    fn new(
        frame: &RendererViewportFrame,
        layout: RasterBufferLayout,
        pixels: Option<Arc<Vec<u8>>>,
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
}

impl OutputState {
    fn advance_binding(&mut self) -> bool {
        self.binding += 1;
        let cleared = self.frame.is_some() || self.cursor.is_some();
        self.frame = None;
        self.cursor = None;
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
            pixels: frame.pixels.as_ref().map(|bytes| bytes.as_slice()),
        }))
    }

    fn bind(&self) -> u64 {
        let (binding, cleared) = {
            let mut state = self.state.lock();
            let cleared = state.advance_binding();
            (state.binding, cleared)
        };
        if cleared {
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
        let frames = self
            .subscribe_viewport_frames_with_pointer(
                surface,
                viewport_revision,
                initial_pointer_position,
            )
            .await?;
        let binding = output.bind();
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
