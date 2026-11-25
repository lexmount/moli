use std::sync::{Arc, OnceLock, Weak};

use core_video::pixel_buffer::CVPixelBuffer;
use parking_lot::Mutex;
use tokio::sync::watch;

use crate::RgbaImage;

type ConvertFrame = Box<dyn Fn() -> Option<Arc<RgbaImage>> + Send>;

pub struct NativeVideoFrame {
    buffer: CVPixelBuffer,
    visible: [u32; 4],
    conversion: [[f32; 4]; 3],
    chroma_offset: [f32; 2],
    rgba: OnceLock<Option<Arc<RgbaImage>>>,
    convert: Mutex<ConvertFrame>,
}

// The CVPixelBuffer is retained, immutable after decoding, and imported by
// Metal on the UI thread. Software readback uses its own retained AVFrame.
unsafe impl Send for NativeVideoFrame {}
unsafe impl Sync for NativeVideoFrame {}

impl NativeVideoFrame {
    /// # Safety
    ///
    /// The buffer must hold an immutable, IOSurface-backed I420 frame for the
    /// lifetime of every retained reference. The visible rect must fit its
    /// luma plane. `convert` must read this same frame.
    pub unsafe fn new(
        buffer: CVPixelBuffer,
        visible: [u32; 4],
        conversion: [[f32; 4]; 3],
        chroma_offset: [f32; 2],
        convert: impl Fn() -> Option<Arc<RgbaImage>> + Send + 'static,
    ) -> Self {
        Self {
            buffer,
            visible,
            conversion,
            chroma_offset,
            rgba: OnceLock::new(),
            convert: Mutex::new(Box::new(convert)),
        }
    }

    pub fn buffer(&self) -> CVPixelBuffer {
        self.buffer.clone()
    }

    pub fn visible_rect(&self) -> [u32; 4] {
        self.visible
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.visible[2], self.visible[3])
    }

    pub fn conversion(&self) -> [[f32; 4]; 3] {
        self.conversion
    }

    pub fn chroma_offset(&self) -> [f32; 2] {
        self.chroma_offset
    }

    pub fn rgba(&self) -> Option<&Arc<RgbaImage>> {
        self.rgba.get_or_init(|| self.convert.lock()()).as_ref()
    }
}

impl std::fmt::Debug for NativeVideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeVideoFrame")
            .field("visible", &self.visible)
            .finish_non_exhaustive()
    }
}

impl PartialEq for NativeVideoFrame {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

/// The latest frame accepted by the playback clock for one media load.
#[derive(Clone, Debug)]
pub struct VideoFrameSource(Arc<Mutex<VideoFrames>>);

#[derive(Debug)]
struct VideoFrames {
    latest: Arc<NativeVideoFrame>,
    listeners: Vec<Weak<VideoFrameListener>>,
}

/// Kept alive by a published viewport scene. An invisible layer still
/// consumes frames without scheduling redraws.
#[derive(Debug)]
pub struct VideoFrameListener(Option<watch::Sender<()>>);

impl VideoFrameSource {
    pub fn new(frame: Arc<NativeVideoFrame>) -> Self {
        Self(Arc::new(Mutex::new(VideoFrames {
            latest: frame,
            listeners: Vec::new(),
        })))
    }

    /// Replaces the displayed frame. Returns whether a current viewport scene
    /// consumes the resource independently of the page raster.
    pub fn publish(&self, frame: Arc<NativeVideoFrame>) -> bool {
        let mut state = self.0.lock();
        let previous = std::mem::replace(&mut state.latest, frame);
        let mut composited = false;
        state.listeners.retain(|listener| {
            let Some(listener) = listener.upgrade() else {
                return false;
            };
            composited = true;
            if let Some(changes) = &listener.0 {
                changes.send_replace(());
            }
            true
        });
        drop(state);
        drop(previous);
        composited
    }

    pub fn listen(&self, changes: Option<watch::Sender<()>>) -> Arc<VideoFrameListener> {
        let listener = Arc::new(VideoFrameListener(changes));
        let mut state = self.0.lock();
        state
            .listeners
            .retain(|listener| listener.strong_count() > 0);
        state.listeners.push(Arc::downgrade(&listener));
        listener
    }

    pub fn latest(&self) -> Arc<NativeVideoFrame> {
        Arc::clone(&self.0.lock().latest)
    }
}

impl PartialEq for VideoFrameSource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
