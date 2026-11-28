use std::{ops::Range, sync::Arc};

use moli_layout::PaintSnapshot;

use super::RendererDocumentLifecycleIdentity;

pub use style::values::specified::ui::CursorKind as RendererViewportCursorKind;

/// The resolved native cursor keyword for the current viewport pointer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RendererViewportCursor {
    pub document: RendererDocumentLifecycleIdentity,
    pub viewport_revision: u64,
    pub kind: RendererViewportCursorKind,
}

/// Owned input for presenting one version of a live viewport on another thread.
#[derive(Clone, Debug)]
pub struct RendererViewportFrame {
    pub document: RendererDocumentLifecycleIdentity,
    pub viewport_revision: u64,
    pub sequence: u64,
    pub editor: Option<RendererViewportEditor>,
    pub snapshot: Option<Arc<PaintSnapshot>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RendererViewportEditor {
    pub target: u32,
    pub revision: u64,
    pub text: String,
    pub password: bool,
    pub text_start: usize,
    pub selection: Range<usize>,
    pub reversed: bool,
    pub marked: Option<Range<usize>>,
    pub caret_rect: Option<[f32; 4]>,
    pub mark_rects: Vec<(usize, [f32; 4])>,
}

pub struct RendererViewportEdit {
    pub document: RendererDocumentLifecycleIdentity,
    pub target: u32,
    pub revision: u64,
    pub sequence: u64,
    pub previous: Option<u64>,
    pub action: RendererViewportEditAction,
}

pub enum RendererViewportEditAction {
    Compose {
        range: Option<Range<usize>>,
        text: String,
        selection: Option<Range<usize>>,
    },
    Commit {
        range: Option<Range<usize>>,
        text: String,
    },
    Paste {
        text: String,
    },
    Finish,
    Cancel,
}

pub struct RendererViewportEditReply {
    pub applied: bool,
    pub editor: Option<RendererViewportEditor>,
    /// Last frame published on this subscription before taking the editor snapshot.
    pub last_frame_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererViewportNavigation {
    pub url: url::Url,
    pub title: String,
    pub favicon_urls: Arc<[url::Url]>,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

/// Latest navigation metadata for the Page's committed Document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererPageNavigationState {
    pub document: RendererDocumentLifecycleIdentity,
    pub navigation: RendererViewportNavigation,
}

/// Pointer input in CSS pixels for the displayed document and viewport.
#[derive(Clone, Copy, Debug)]
pub struct RendererViewportPointerInput {
    pub document: RendererDocumentLifecycleIdentity,
    pub viewport_revision: u64,
    pub position: (f64, f64),
    pub event: &'static str,
    pub button: i32,
    pub buttons: i32,
    pub click_count: i32,
    pub modifiers: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererViewportWheelPhase {
    /// A mouse wheel whose platform does not provide transaction phases.
    Unspecified,
    Started,
    Moved,
    Ended,
    Cancelled,
}

/// A wheel event expressed in CSS pixels for a particular viewport revision.
#[derive(Clone, Copy, Debug)]
pub struct RendererViewportWheelInput {
    pub document: RendererDocumentLifecycleIdentity,
    pub viewport_revision: u64,
    pub position: (f64, f64),
    pub buttons: i32,
    pub delta: (f64, f64),
    pub modifiers: u8,
    pub phase: RendererViewportWheelPhase,
    pub precise: bool,
}
