use std::sync::Arc;

use moli_input::{InputModifiers, Point, ScrollAction};
use moli_layout::{LayoutFlushReason, PaintCaptureRequest, PaintSnapshot};
use moli_page_types::ViewportSurface;
use tokio::sync::{Notify, watch};

use super::{
    PageVm, RendererDocumentLifecycleIdentity, RendererInputDispatchOutcome,
    RendererScreencastPaintSession, RendererVisualStateToken,
};
use crate::layout_renderer::layout_viewport_for_surface;
use crate::native_bridge::LayoutPublicationRequest;

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
    pub selection: std::ops::Range<usize>,
    pub reversed: bool,
    pub marked: Option<std::ops::Range<usize>>,
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
        range: Option<std::ops::Range<usize>>,
        text: String,
        selection: Option<std::ops::Range<usize>>,
    },
    Commit {
        range: Option<std::ops::Range<usize>>,
        text: String,
    },
    Paste {
        text: String,
    },
    Finish,
    Cancel,
}

pub enum RendererViewportPaste {
    Text(Option<String>),
    CanceledByKeydown,
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

pub struct RendererViewportSubscription {
    pub surface: ViewportSurface,
    pub viewport_revision: u64,
    pub initial_pointer_position: Option<(f64, f64)>,
    pub updates: watch::Sender<Option<RendererViewportFrame>>,
    pub cursor: watch::Sender<Option<RendererViewportCursor>>,
    pub display_tick_requests: watch::Sender<Option<u64>>,
    pub resource_wake: Arc<Notify>,
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

impl RendererViewportWheelPhase {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Ended | Self::Cancelled)
    }
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

impl RendererViewportWheelInput {
    pub(crate) fn coalesce(&mut self, next: &Self) -> bool {
        if !matches!(
            self.phase,
            RendererViewportWheelPhase::Moved | RendererViewportWheelPhase::Unspecified
        ) || next.phase != self.phase
            || self.document != next.document
            || self.viewport_revision != next.viewport_revision
            || self.buttons != next.buttons
            || self.precise != next.precise
        {
            return false;
        }

        let mut scroll = self.scroll_action();
        if !scroll.coalesce(&next.scroll_action()) {
            return false;
        }
        self.delta = (scroll.delta_x, scroll.delta_y);
        true
    }

    fn scroll_action(&self) -> ScrollAction {
        ScrollAction {
            modifiers: InputModifiers::from_bits(self.modifiers),
            ..ScrollAction::pixels(
                Point::new(self.position.0, self.position.1),
                self.delta.0,
                self.delta.1,
            )
        }
    }
}

pub(super) struct ViewportFrameState {
    subscription: RendererViewportSubscription,
    painted: Option<RendererVisualStateToken>,
    paint_session: RendererScreencastPaintSession,
    sequence: u64,
    editing: crate::context_bootstrap::viewport_edit::ViewportEditing,
    pointer: Option<(f64, f64)>,
    sampled_pointer: Option<(f64, f64)>,
    pressed_pointer: Option<RendererViewportPointerInput>,
}

impl ViewportFrameState {
    fn pressed_pointer_for(
        &self,
        document: RendererDocumentLifecycleIdentity,
    ) -> Option<RendererViewportPointerInput> {
        self.pressed_pointer.filter(|pointer| {
            pointer.document == document
                && pointer.viewport_revision == self.subscription.viewport_revision
        })
    }

    pub(super) fn discard_document(&mut self) {
        self.painted = None;
        self.editing = Default::default();
        self.pressed_pointer = None;
        self.subscription.updates.send_replace(None);
        self.clear_cursor();
    }

    fn clear_cursor(&mut self) {
        self.sampled_pointer = None;
        self.subscription
            .cursor
            .send_if_modified(|cursor| cursor.take().is_some());
    }

    fn publish_cursor(
        &self,
        document: RendererDocumentLifecycleIdentity,
        kind: RendererViewportCursorKind,
    ) {
        let next = Some(RendererViewportCursor {
            document,
            viewport_revision: self.subscription.viewport_revision,
            kind,
        });
        self.subscription.cursor.send_if_modified(|cursor| {
            let changed = *cursor != next;
            *cursor = next;
            changed
        });
    }
}

impl PageVm {
    /// Synchronizes pointer position within an existing viewport revision.
    /// These updates do not dispatch a DOM mouse event.
    pub(super) fn set_viewport_pointer_position(
        &mut self,
        viewport_revision: u64,
        position: Option<(f64, f64)>,
    ) {
        if self
            .viewport_frames
            .as_ref()
            .is_some_and(|state| state.subscription.viewport_revision == viewport_revision)
        {
            self.update_viewport_pointer_position(position);
        }
    }

    pub(super) fn update_viewport_pointer_position(&mut self, position: Option<(f64, f64)>) {
        let Some(state) = self.viewport_frames.as_mut() else {
            return;
        };
        let position = visible_pointer_position(
            state.subscription.surface,
            self.document_settings.document_activity.visible,
            position,
        );
        if state.pointer != position {
            state.pointer = position;
            if position.is_some() {
                self.wake_viewport_frames();
            } else {
                state.clear_cursor();
                self.vm().clear_viewport_hover();
            }
        }
    }

    fn viewport_pressed_buttons(&self) -> i32 {
        let document = self.document_lifecycle.identity();
        self.viewport_frames
            .as_ref()
            .and_then(|state| state.pressed_pointer_for(document))
            .map_or(0, |pointer| pointer.buttons)
    }

    pub(super) fn dispatch_viewport_pointer(
        &mut self,
        mut input: RendererViewportPointerInput,
    ) -> anyhow::Result<Option<RendererInputDispatchOutcome>> {
        if self.viewport_pressed_buttons() == 0 {
            if input.event == "mouseup" {
                return Ok(None);
            }
            if input.event == "mousemove" {
                input.buttons = 0;
            }
        }
        let outcome = self.dispatch_viewport_mouse(input, (0., 0.))?;
        if outcome.is_some() {
            self.viewport_frames
                .as_mut()
                .expect("input requires a viewport subscription")
                .pressed_pointer = (input.buttons != 0).then_some(input);
        } else if input.event == "mouseup" {
            self.cancel_viewport_pointer(input.document, input.viewport_revision)?;
        }
        Ok(outcome)
    }

    pub(super) fn cancel_viewport_pointer(
        &mut self,
        document: RendererDocumentLifecycleIdentity,
        viewport_revision: u64,
    ) -> anyhow::Result<()> {
        if document == self.document_lifecycle.identity()
            && self
                .viewport_frames
                .as_ref()
                .is_some_and(|state| state.subscription.viewport_revision == viewport_revision)
        {
            self.cancel_active_viewport_pointer()?;
        }
        Ok(())
    }

    fn cancel_active_viewport_pointer(&mut self) -> anyhow::Result<()> {
        let document = self.document_lifecycle.identity();
        let pressed = self.viewport_frames.as_mut().and_then(|state| {
            let pressed = state.pressed_pointer_for(document);
            state.pressed_pointer = None;
            pressed
        });
        if let Some(input) = pressed {
            self.vm_mut()
                .cancel_viewport_mouse(input.position, input.modifiers)?;
            self.wake_viewport_frames();
        }
        Ok(())
    }

    fn accepts_viewport_input(
        &self,
        document: RendererDocumentLifecycleIdentity,
        revision: u64,
    ) -> bool {
        document == self.document_lifecycle.identity()
            && self.document_settings.document_activity.visible
            && self.viewport_frames.as_ref().is_some_and(|state| {
                state.subscription.viewport_revision == revision
                    && state
                        .subscription
                        .updates
                        .borrow()
                        .as_ref()
                        .is_some_and(|frame| frame.document == document && frame.snapshot.is_some())
            })
    }

    fn dispatch_viewport_mouse(
        &mut self,
        input: RendererViewportPointerInput,
        delta: (f64, f64),
    ) -> anyhow::Result<Option<RendererInputDispatchOutcome>> {
        if !self.accepts_viewport_input(input.document, input.viewport_revision) {
            return Ok(None);
        }
        match self.dispatch_mouse_event_at_point_with_pointer(
            input.position.0,
            input.position.1,
            input.event,
            input.button,
            Some(input.buttons),
            input.click_count,
            delta.0,
            delta.1,
            super::RendererPointerEventProperties::default(),
            input.modifiers,
            false,
        ) {
            Ok(outcome) => Ok(Some(outcome)),
            // A document can lose its geometry after the last frame was painted.
            Err(error)
                if error.downcast_ref::<moli_layout::LayoutError>()
                    == Some(&moli_layout::LayoutError::NoLayoutSnapshot) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn dispatch_viewport_wheel(
        &mut self,
        input: RendererViewportWheelInput,
    ) -> anyhow::Result<()> {
        use crate::script_vm::WheelDispatchOptions;
        use crate::wheel_input::{WheelInput, WheelPhase, WheelSource};
        if input.document != self.document_lifecycle.identity() {
            return Ok(());
        }
        let phase = match input.phase {
            RendererViewportWheelPhase::Ended => WheelPhase::Ended,
            RendererViewportWheelPhase::Cancelled => WheelPhase::Cancelled,
            RendererViewportWheelPhase::Unspecified => WheelPhase::Unspecified,
            RendererViewportWheelPhase::Started => WheelPhase::Began,
            RendererViewportWheelPhase::Moved => WheelPhase::Changed,
        };
        // Some platforms include the last movement with End. Forward that
        // delta only with current geometry, and always finish the sequence.
        let has_update = match phase {
            WheelPhase::Ended => {
                input.delta != (0., 0.)
                    && self.wheel_position_in_bounds(input.position.0, input.position.1)
            }
            WheelPhase::Cancelled => false,
            _ => true,
        };
        let result =
            if has_update && self.accepts_viewport_input(input.document, input.viewport_revision) {
                let buttons = self.viewport_pressed_buttons();
                let result = self.vm_mut().dispatch_wheel_input(
                    WheelInput {
                        position: input.position,
                        delta: input.delta,
                        modifiers: input.modifiers,
                        timestamp: std::time::Instant::now(),
                        phase: if phase == WheelPhase::Ended {
                            WheelPhase::Changed
                        } else {
                            phase
                        },
                        source: WheelSource::Platform,
                    },
                    WheelDispatchOptions {
                        buttons: Some(buttons),
                        ..Default::default()
                    },
                );
                if result.is_ok() {
                    self.update_viewport_pointer_position(Some(input.position));
                }
                result
            } else {
                Ok(RendererInputDispatchOutcome::default())
            };
        if input.phase.is_terminal() {
            self.vm().wheel_input.borrow_mut().end_platform_sequence();
        }
        if let Err(error) = &result {
            if error.downcast_ref::<moli_layout::LayoutError>()
                == Some(&moli_layout::LayoutError::NoLayoutSnapshot)
            {
                return Ok(());
            }
            tracing::warn!(%error, "viewport wheel dispatch failed");
        }
        result.map(drop)
    }

    pub(super) fn read_viewport_selection(
        &mut self,
        document: RendererDocumentLifecycleIdentity,
    ) -> anyhow::Result<Option<String>> {
        if document != self.document_lifecycle.identity()
            || !self.document_settings.document_activity.focused
        {
            return Ok(None);
        }
        self.vm_mut().viewport_selected_text()
    }

    pub(super) fn copy_or_cut_viewport_selection(
        &mut self,
        document: RendererDocumentLifecycleIdentity,
        cut: bool,
    ) -> anyhow::Result<Option<String>> {
        if document != self.document_lifecycle.identity()
            || !self.document_settings.document_activity.focused
        {
            return Ok(None);
        }
        let text = self.vm_mut().copy_or_cut_viewport_selection(cut)?;
        self.wake_viewport_frames();
        Ok(text)
    }

    pub(super) fn paste_viewport_text(
        &mut self,
        document: RendererDocumentLifecycleIdentity,
        paste: &RendererViewportPaste,
        sequence: Option<u64>,
    ) -> anyhow::Result<bool> {
        if document != self.document_lifecycle.identity()
            || !self.document_settings.document_activity.focused
        {
            return Ok(false);
        }
        let (Some(state), Some(vm)) = (self.viewport_frames.as_mut(), self.vm.as_mut()) else {
            return Ok(false);
        };
        let applied = vm.paste_viewport_text(&mut state.editing, paste, sequence)?;
        self.wake_viewport_frames();
        Ok(applied)
    }

    pub(super) fn drop_viewport_files(
        &mut self,
        document: RendererDocumentLifecycleIdentity,
        x: f64,
        y: f64,
        files: Vec<super::RendererDraggedFile>,
        modifiers: u8,
    ) -> anyhow::Result<bool> {
        if document != self.document_lifecycle.identity()
            || !self.document_settings.document_activity.visible
            || files.is_empty()
        {
            return Ok(false);
        }
        let data = super::RendererDragData {
            items: Vec::new(),
            files,
            directories: Vec::new(),
            drag_operations_mask: 1,
        };
        let entered = self.vm_mut().dispatch_drag_event_at_point(
            x,
            y,
            "dragenter",
            data.clone(),
            modifiers,
        )?;
        if !entered.handled || document != self.document_lifecycle.identity() {
            return Ok(false);
        }
        let outcome = self
            .vm_mut()
            .dispatch_drag_event_at_point(x, y, "drop", data, modifiers)?;
        Ok(outcome.handled)
    }

    pub(super) fn read_viewport_editor(&mut self) -> Option<RendererViewportEditor> {
        if !self.document_settings.document_activity.focused {
            return None;
        }
        let editing = &self.viewport_frames.as_ref()?.editing;
        self.vm.as_mut()?.viewport_editor_snapshot(editing)
    }

    pub(super) fn apply_viewport_edit(
        &mut self,
        edit: RendererViewportEdit,
    ) -> anyhow::Result<RendererViewportEditReply> {
        let last_frame_sequence = self
            .viewport_frames
            .as_ref()
            .map_or(0, |state| state.sequence);
        if edit.document != self.document_lifecycle.identity()
            || !self.document_settings.document_activity.focused
        {
            return Ok(RendererViewportEditReply {
                applied: false,
                editor: None,
                last_frame_sequence,
            });
        }
        let (Some(state), Some(vm)) = (self.viewport_frames.as_mut(), self.vm.as_mut()) else {
            return Ok(RendererViewportEditReply {
                applied: false,
                editor: None,
                last_frame_sequence,
            });
        };
        let applied = vm.apply_viewport_editor_input(&mut state.editing, edit)?;
        self.wake_viewport_frames();
        Ok(RendererViewportEditReply {
            applied,
            editor: self.read_viewport_editor(),
            last_frame_sequence,
        })
    }

    pub(super) fn subscribe_viewport_frames(
        &mut self,
        mut subscription: RendererViewportSubscription,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.layout_configuration.policy.is_realtime(),
            "Viewport frames require the Realtime layout policy"
        );
        if let Err(error) = self.cancel_active_viewport_pointer() {
            tracing::warn!(%error, "could not cancel the previous viewport's pointer input");
        }
        self.set_viewport_surface(Some(subscription.surface))?;
        let pointer = visible_pointer_position(
            subscription.surface,
            self.document_settings.document_activity.visible,
            subscription.initial_pointer_position.take(),
        );
        if pointer.is_none() {
            self.vm().clear_viewport_hover();
        }
        let editing = self
            .viewport_frames
            .as_mut()
            .map(|state| std::mem::take(&mut state.editing))
            .unwrap_or_default();
        self.restore_viewport_frames(Box::new(ViewportFrameState {
            subscription,
            painted: None,
            paint_session: RendererScreencastPaintSession::default(),
            sequence: 0,
            editing,
            pointer,
            sampled_pointer: None,
            pressed_pointer: None,
        }));
        Ok(())
    }

    pub(super) fn restore_viewport_frames(&mut self, state: Box<ViewportFrameState>) {
        self.vm()
            .set_viewport_resource_wake(&state.subscription.resource_wake);
        self.vm().set_viewport_display_clock(Some((
            state.subscription.viewport_revision,
            state.subscription.display_tick_requests.clone(),
        )));
        self.viewport_frames = Some(state);
        self.wake_viewport_frames();
    }

    pub(super) fn take_viewport_frames(&mut self) -> Option<Box<ViewportFrameState>> {
        let frames = self.viewport_frames.take()?;
        if let Some(vm) = &self.vm {
            vm.set_viewport_display_clock(None);
        }
        Some(frames)
    }

    pub(super) fn accept_viewport_display_tick(
        &self,
        revision: u64,
        time: std::time::Instant,
    ) -> bool {
        if let Some(vm) = &self.vm
            && self.document_settings.document_activity.visible
            && self.viewport_frames.as_ref().is_some_and(|state| {
                state.subscription.viewport_revision == revision
                    && !state.subscription.updates.is_closed()
            })
        {
            vm.accept_viewport_display_tick(time)
        } else {
            false
        }
    }

    pub(super) fn wake_viewport_frames(&mut self) {
        if self
            .viewport_frames
            .as_ref()
            .is_some_and(|state| !state.subscription.updates.is_closed())
        {
            self.vm().request_viewport_frame();
        }
    }

    pub(super) fn update_viewport_frame(&mut self) {
        if !self.document_settings.document_activity.visible {
            return;
        }
        let Some(state) = self.viewport_frames.as_ref() else {
            return;
        };
        if state.subscription.updates.is_closed() {
            drop(self.take_viewport_frames());
            self.vm().clear_viewport_hover();
            return;
        }
        let surface = state.subscription.surface;
        let viewport = layout_viewport_for_surface(surface);
        let paint_session = state.paint_session.clone();
        let document = self.document_lifecycle.identity();
        let before = self.vm().visual_state_token(document, viewport, [255; 4]);
        let pixels_current = state.painted.as_ref() == Some(&before);
        let pointer_to_sample = state
            .pointer
            .filter(|_| !pixels_current || state.pointer != state.sampled_pointer);
        let hover_changed = if let Some(position) = pointer_to_sample {
            let (kind, hover_changed) = self
                .vm()
                .refresh_viewport_pointer(position)
                .unwrap_or_else(|error| {
                    tracing::debug!(%error, "could not resolve viewport cursor");
                    (RendererViewportCursorKind::Default, false)
                });
            let state = self
                .viewport_frames
                .as_mut()
                .expect("viewport subscription");
            // Recheck on the next frame if the hover change moves the hit geometry.
            state.sampled_pointer = (!hover_changed).then_some(position);
            state.publish_cursor(document, kind);
            hover_changed
        } else {
            false
        };
        if pixels_current && !hover_changed {
            return;
        }

        let vm = self.vm_mut();
        let reason = LayoutFlushReason::RenderingOpportunity;
        let capture = PaintCaptureRequest::viewport();
        let snapshot = match vm
            .paint_current_screen_layout(Some(&paint_session), viewport, capture, reason)
            .and_then(|snapshot| match snapshot {
                Some(_) => Ok(snapshot),
                None => vm.paint_layout_snapshot_with_capture(
                    viewport,
                    reason,
                    capture,
                    LayoutPublicationRequest::Screen,
                    None,
                ),
            }) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(%error, "could not produce viewport frame");
                return;
            }
        };
        if self.document_lifecycle.identity() != document {
            self.wake_viewport_frames();
            return;
        }
        let after = self.vm().visual_state_token(document, viewport, [255; 4]);
        let editor = self.read_viewport_editor();
        let state = self
            .viewport_frames
            .as_mut()
            .expect("viewport subscription");
        state.painted = Some(super::page_screenshot::visual_state_for_rendered_frame(
            before, after,
        ));
        state.sequence += 1;
        state
            .subscription
            .updates
            .send_replace(Some(RendererViewportFrame {
                document,
                viewport_revision: state.subscription.viewport_revision,
                sequence: state.sequence,
                editor,
                snapshot: snapshot.map(Arc::new),
            }));
    }
}

fn visible_pointer_position(
    surface: ViewportSurface,
    visible: bool,
    position: Option<(f64, f64)>,
) -> Option<(f64, f64)> {
    position.filter(|&(x, y)| {
        visible
            && (0.0..f64::from(surface.inner_width)).contains(&x)
            && (0.0..f64::from(surface.inner_height)).contains(&y)
    })
}
