use std::sync::Arc;

use moli_action_window::{InputModifiers, Point, ScrollAction};
use moli_layout::{LayoutFlushReason, PaintCaptureRequest, PaintSnapshot, PaintViewport};
use moli_page_types::ViewportSurface;
use tokio::sync::{Notify, watch};

use super::{PageVm, RendererDocumentLifecycleIdentity, RendererVisualStateToken};
use crate::native_bridge::LayoutPublicationRequest;

/// Owned input for presenting one version of a live viewport on another thread.
#[derive(Clone, Debug)]
pub struct RendererViewportFrame {
    pub document: RendererDocumentLifecycleIdentity,
    pub viewport_revision: u64,
    pub sequence: u64,
    pub navigation: RendererViewportNavigation,
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

pub struct RendererViewportEditReply {
    pub applied: bool,
    pub editor: Option<RendererViewportEditor>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererViewportNavigation {
    pub url: url::Url,
    pub title: String,
    pub favicon_urls: Arc<[url::Url]>,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

pub struct RendererViewportSubscription {
    pub surface: ViewportSurface,
    pub viewport_revision: u64,
    pub updates: watch::Sender<Option<RendererViewportFrame>>,
    pub resource_wake: Arc<Notify>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererViewportWheelPhase {
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

impl RendererViewportWheelInput {
    pub(crate) fn coalesce(&mut self, next: &Self) -> bool {
        if self.phase != RendererViewportWheelPhase::Moved
            || next.phase != RendererViewportWheelPhase::Moved
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
    navigation: Option<RendererViewportNavigation>,
    sequence: u64,
    editing: crate::context_bootstrap::viewport_edit::ViewportEditing,
}

impl ViewportFrameState {
    pub(super) fn discard_document(&mut self) {
        self.painted = None;
        self.navigation = None;
        self.editing = Default::default();
        self.subscription.updates.send_replace(None);
    }
}

impl PageVm {
    pub(super) fn dispatch_viewport_wheel(
        &mut self,
        input: RendererViewportWheelInput,
    ) -> anyhow::Result<()> {
        if input.document != self.document_lifecycle.identity()
            || !self.document_settings.document_activity.visible
            || self
                .viewport_frames
                .as_ref()
                .is_none_or(|state| state.subscription.viewport_revision != input.viewport_revision)
        {
            return Ok(());
        }
        let result = self.dispatch_mouse_event_at_point_with_pointer(
            input.position.0,
            input.position.1,
            "wheel",
            -1,
            Some(input.buttons),
            0,
            input.delta.0,
            input.delta.1,
            super::RendererPointerEventProperties::default(),
            input.modifiers,
            false,
        );
        if let Err(error) = &result {
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
        if edit.document != self.document_lifecycle.identity()
            || !self.document_settings.document_activity.focused
        {
            return Ok(RendererViewportEditReply {
                applied: false,
                editor: None,
            });
        }
        let (Some(state), Some(vm)) = (self.viewport_frames.as_mut(), self.vm.as_mut()) else {
            return Ok(RendererViewportEditReply {
                applied: false,
                editor: None,
            });
        };
        let applied = vm.apply_viewport_editor_input(&mut state.editing, edit)?;
        self.wake_viewport_frames();
        Ok(RendererViewportEditReply {
            applied,
            editor: self.read_viewport_editor(),
        })
    }

    pub(super) fn subscribe_viewport_frames(
        &mut self,
        subscription: RendererViewportSubscription,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.layout_configuration.policy.is_realtime(),
            "Viewport frames require the Realtime layout policy"
        );
        self.set_viewport_surface(Some(subscription.surface))?;
        let editing = self
            .viewport_frames
            .as_mut()
            .map(|state| std::mem::take(&mut state.editing))
            .unwrap_or_default();
        self.restore_viewport_frames(Box::new(ViewportFrameState {
            subscription,
            painted: None,
            navigation: None,
            sequence: 0,
            editing,
        }));
        Ok(())
    }

    pub(super) fn restore_viewport_frames(&mut self, state: Box<ViewportFrameState>) {
        self.vm()
            .set_viewport_resource_wake(&state.subscription.resource_wake);
        self.viewport_frames = Some(state);
        self.wake_viewport_frames();
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

    pub(super) fn publish_viewport_frame(&mut self) {
        if !self.document_settings.document_activity.visible {
            return;
        }
        let Some(state) = self.viewport_frames.as_ref() else {
            return;
        };
        if state.subscription.updates.is_closed() {
            self.viewport_frames = None;
            return;
        }
        let surface = state.subscription.surface;
        let viewport = PaintViewport::new(
            surface.inner_width,
            surface.inner_height,
            surface.device_pixel_ratio as f32,
        );
        let document = self.document_lifecycle.identity();
        let before = self.vm().visual_state_token(document, viewport, [255; 4]);
        let navigation = self.vm().viewport_navigation();
        if state.painted.as_ref() == Some(&before) && state.navigation.as_ref() == Some(&navigation)
        {
            return;
        }

        let snapshot = match self.vm_mut().paint_layout_snapshot_with_capture(
            viewport,
            LayoutFlushReason::RenderingOpportunity,
            PaintCaptureRequest::viewport(),
            LayoutPublicationRequest::Screen,
            None,
        ) {
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
        state.navigation = Some(navigation.clone());
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
                navigation,
                editor,
                snapshot: snapshot.map(Arc::new),
            }));
    }
}
