use std::{ops::Range, rc::Rc, sync::Arc};

use gpui::{
    App, BgraImage, Bounds, ClipboardItem, Context, CursorStyle, DispatchPhase,
    ElementInputHandler, EntityInputHandler, ExternalPaths, FocusHandle, KeyDownEvent, KeyUpEvent,
    Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    Pixels, Point, ScrollWheelEvent, Subscription, TouchPhase, UTF16Selection, Window, canvas, div,
    point, prelude::*, px, rgb, size,
};
use moli_core::{
    RendererDocumentLifecycleIdentity, RendererViewportCursorKind, RendererViewportEditAction,
    RendererViewportEditor, RendererViewportPointerInput, RendererViewportWheelInput,
    RendererViewportWheelPhase,
    page::{VIEWPORT_BAND_HEIGHT, ViewportFrameRef, ViewportOutput},
    protocol_types::{DocumentActivity, ViewportSurface},
};
use tokio::sync::{mpsc, oneshot, watch};

use crate::host::{ClipboardAction, KeyInput, TabCommand};
use crate::text_input::MoliTextInputBridge;

#[derive(Clone, Copy)]
struct WheelRoute {
    document: RendererDocumentLifecycleIdentity,
    viewport_revision: u64,
    phased: bool,
}

pub struct MoliSurface {
    pub focus: FocusHandle,
    commands: Option<mpsc::UnboundedSender<TabCommand>>,
    output: Option<Arc<ViewportOutput>>,
    display_ticks: Option<watch::Sender<(u64, std::time::Instant)>>,
    display_tick_pending: bool,
    activity: Option<DocumentActivity>,
    bounds: Option<Bounds<Pixels>>,
    viewport: Option<ViewportSurface>,
    revision: u64,
    zoom: f64,
    buttons: i32,
    pointer_over: bool,
    image: Option<ViewportImage>,
    document: Option<RendererDocumentLifecycleIdentity>,
    presented: Option<(RendererDocumentLifecycleIdentity, u64)>,
    wheel_route: Option<WheelRoute>,
    text_input: MoliTextInputBridge,
    _subscriptions: Vec<Subscription>,
}

struct ViewportBandImage {
    start: u32,
    end: u32,
    version: u64,
    texture: Rc<BgraImage>,
}

struct ViewportImage {
    width: u32,
    height: u32,
    banded: bool,
    bands: Vec<ViewportBandImage>,
}

impl ViewportImage {
    fn new(
        frame: &ViewportFrameRef<'_>,
        bounds: Bounds<Pixels>,
        window: &Window,
        previous: Option<&Self>,
    ) -> anyhow::Result<Self> {
        let scale = window.scale_factor();
        let width = frame.layout.width;
        let height = frame.layout.height;
        // Sample each atlas tile at device-pixel centers. Fractional resampling
        // at a band boundary requires the whole-frame image to avoid seams.
        let banded = frame.band_versions.len() == height.div_ceil(VIEWPORT_BAND_HEIGHT) as usize
            && frame.band_versions.len() > 1
            && Self::bands_fit(bounds, scale, width, height);
        let previous = previous.filter(|image| {
            image.width == width && image.height == height && image.banded == banded
        });
        let band_height = if banded { VIEWPORT_BAND_HEIGHT } else { height };
        let mut bands = Vec::new();
        for index in 0..height.div_ceil(band_height) {
            let start = index * band_height;
            let end = (start + band_height).min(height);
            let version = if banded {
                frame.band_versions[index as usize]
            } else {
                frame.sequence
            };
            let cached = previous
                .and_then(|image| image.bands.get(index as usize))
                .filter(|band| band.version == version);
            let texture = if let Some(cached) = cached {
                cached.texture.clone()
            } else {
                Rc::new(BgraImage::new(size(width.into(), (end - start).into()))?)
            };
            bands.push(ViewportBandImage {
                start,
                end,
                version,
                texture,
            });
        }
        Ok(Self {
            width,
            height,
            banded,
            bands,
        })
    }

    fn bands_fit(bounds: Bounds<Pixels>, scale: f32, width: u32, height: u32) -> bool {
        f32::from(bounds.size.width) * scale == width as f32
            && f32::from(bounds.size.height) * scale == height as f32
            && (f32::from(bounds.origin.x) * scale).fract() == 0.0
            && (f32::from(bounds.origin.y) * scale).fract() == 0.0
    }

    fn paint_bounds(&self, bounds: Bounds<Pixels>, window: &Window) -> Bounds<Pixels> {
        if !self.banded {
            return bounds;
        }
        // Only the atlas tiles may remain while a new viewport is rasterized.
        // Draw them at native size and let the surface clip the old image.
        let scale = window.scale_factor();
        Bounds::new(
            window.pixel_snap_point(bounds.origin),
            size(
                px(self.width as f32 / scale),
                px(self.height as f32 / scale),
            ),
        )
    }

    fn paint(
        &self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        pixels: Option<&[u8]>,
    ) -> anyhow::Result<bool> {
        let bounds = self.paint_bounds(bounds, window);
        let row_bytes = self.width as usize * 4;
        for band in &self.bands {
            let top = bounds.size.height * (band.start as f32 / self.height as f32);
            let bottom = bounds.size.height * (band.end as f32 / self.height as f32);
            let band_bounds = Bounds {
                origin: point(bounds.origin.x, bounds.origin.y + top),
                size: size(bounds.size.width, bottom - top),
            };
            let bytes = pixels.map(|pixels| {
                &pixels[band.start as usize * row_bytes..band.end as usize * row_bytes]
            });
            if !window.paint_bgra_image(band_bounds, &band.texture, bytes)? {
                return Ok(false);
            }
        }
        Ok(!self.bands.is_empty())
    }

    fn release_replaced(&self, kept: Option<&Self>, window: &mut Window) {
        for (index, band) in self.bands.iter().enumerate() {
            if kept
                .and_then(|image| image.bands.get(index))
                .is_none_or(|next| !Rc::ptr_eq(&band.texture, &next.texture))
            {
                window.drop_bgra_image(&band.texture);
            }
        }
    }
}

impl MoliSurface {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let activation = cx.observe_window_activation(window, |surface, window, _| {
            if !window.is_window_active() {
                surface.finish_composition();
                surface.cancel_wheel();
            }
            surface.sync_activity(window);
        });
        let focus_in = cx.on_focus_in(&focus, window, |surface, window, _| {
            surface.sync_activity(window);
        });
        let focus_out = cx.on_focus_out(&focus, window, |surface, _, window, _| {
            surface.finish_composition();
            surface.sync_activity(window);
        });
        Self {
            focus,
            commands: None,
            output: None,
            display_ticks: None,
            display_tick_pending: false,
            activity: None,
            bounds: None,
            viewport: None,
            revision: 0,
            zoom: 1.,
            buttons: 0,
            pointer_over: false,
            image: None,
            document: None,
            presented: None,
            wheel_route: None,
            text_input: MoliTextInputBridge::default(),
            _subscriptions: vec![activation, focus_in, focus_out],
        }
    }

    pub fn select_tab(
        &mut self,
        connection: Option<&crate::host::TabConnection>,
        zoom: f64,
        document: Option<RendererDocumentLifecycleIdentity>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_composition();
        self.clear_frame(window, cx);
        self.send(TabCommand::Activity(DocumentActivity::new(false, false)));
        self.commands = connection.map(|tab| tab.commands.clone());
        self.output = connection.map(|tab| tab.viewport.clone());
        self.display_ticks = connection.and_then(|tab| tab.display_ticks.clone());
        self.activity = None;
        self.viewport = None;
        self.document = document;
        self.zoom = zoom;
        self.sync_activity(window);
    }

    fn display_tick_sender(&self) -> Option<&watch::Sender<(u64, std::time::Instant)>> {
        self.display_ticks.as_ref().filter(|clock| {
            !clock.is_closed()
                && self.viewport.is_some()
                && self
                    .output
                    .as_ref()
                    .is_some_and(|output| output.display_tick_request() == Some(self.revision))
        })
    }

    pub(crate) fn request_display_tick(&mut self, window: &mut Window, cx: &Context<Self>) {
        if self.display_tick_pending || self.display_tick_sender().is_none() {
            return;
        }
        self.display_tick_pending = true;
        cx.on_next_frame(window, |surface, window, cx| {
            surface.display_tick_pending = false;
            if let Some(clock) = surface.display_tick_sender() {
                clock.send_replace((surface.revision, std::time::Instant::now()));
                surface.request_display_tick(window, cx);
            }
        });
    }

    pub fn set_zoom(&mut self, zoom: f64, cx: &mut Context<Self>) {
        self.zoom = zoom;
        cx.notify();
    }

    pub fn clear_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_pointer();
        self.cancel_wheel();
        self.presented = None;
        self.drop_image(window);
        cx.notify();
    }

    fn drop_image(&mut self, window: &mut Window) {
        let Some(image) = self.image.take() else {
            return;
        };
        image.release_replaced(None, window);
    }

    fn is_current_frame(&self, frame: &ViewportFrameRef<'_>) -> bool {
        self.viewport.is_some()
            && self.document == Some(frame.document)
            && self.revision == frame.viewport_revision
    }

    fn is_new_frame(&self, frame: &ViewportFrameRef<'_>) -> bool {
        self.presented.is_none_or(|(document, sequence)| {
            document != frame.document || sequence < frame.sequence
        })
    }

    fn paint_image(&mut self, bounds: Bounds<Pixels>, window: &mut Window) -> anyhow::Result<()> {
        if self
            .output
            .clone()
            .and_then(|output| output.read(|frame| self.paint_ready_image(frame, bounds, window)))
            .transpose()?
            .unwrap_or(false)
        {
            return Ok(());
        }
        if let Some(image) = self.image.as_ref() {
            image.paint(bounds, window, None)?;
        }
        Ok(())
    }

    fn paint_ready_image(
        &mut self,
        frame: ViewportFrameRef<'_>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
    ) -> anyhow::Result<bool> {
        if !self.is_current_frame(&frame) {
            return Ok(false);
        }
        if !self.is_new_frame(&frame) {
            let Some(image) = self.image.as_ref() else {
                return Ok(false);
            };
            if !image.banded
                || ViewportImage::bands_fit(
                    bounds,
                    window.scale_factor(),
                    image.width,
                    image.height,
                )
            {
                return image.paint(bounds, window, frame.pixels);
            }
        }
        let Some(pixels) = frame.pixels else {
            self.cancel_pointer();
            self.drop_image(window);
            self.presented = Some((frame.document, frame.sequence));
            return Ok(false);
        };
        let previous = self.image.as_ref().filter(|_| {
            self.presented
                .is_some_and(|(document, _)| document == frame.document)
        });
        let image = ViewportImage::new(&frame, bounds, window, previous)?;
        let result = image.paint(bounds, window, Some(pixels));
        if !matches!(result, Ok(true)) {
            image.release_replaced(previous, window);
            return result;
        }
        if let Some(old) = self.image.replace(image) {
            old.release_replaced(self.image.as_ref(), window);
        }
        self.presented = Some((frame.document, frame.sequence));
        Ok(true)
    }

    pub fn focus_page(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    pub fn update_editor(
        &mut self,
        document: Option<RendererDocumentLifecycleIdentity>,
        editor: Option<RendererViewportEditor>,
        ack: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.document = document;
        if self.activity.is_some_and(|activity| activity.focused)
            && self.text_input.refresh(document, editor, ack)
        {
            window.invalidate_character_coordinates();
            cx.notify();
        }
    }

    fn apply_text_edit(&mut self, action: RendererViewportEditAction) -> bool {
        if let Some(edit) = self.text_input.edit(action) {
            self.send(TabCommand::Edit(edit));
            true
        } else {
            false
        }
    }

    fn finish_composition(&mut self) {
        if self
            .text_input
            .editor()
            .is_some_and(|editor| editor.marked.is_some())
        {
            self.apply_text_edit(RendererViewportEditAction::Finish);
        }
        self.text_input.clear();
    }

    fn window_rect(&self, rect: [f32; 4]) -> Option<Bounds<Pixels>> {
        let bounds = self.bounds?;
        let viewport = self.viewport?;
        let sx = f32::from(bounds.size.width) / viewport.inner_width as f32;
        let sy = f32::from(bounds.size.height) / viewport.inner_height as f32;
        Some(Bounds::new(
            point(
                bounds.origin.x + px(rect[0] * sx),
                bounds.origin.y + px(rect[1] * sy),
            ),
            size(px(rect[2] * sx), px(rect[3] * sy)),
        ))
    }

    fn sync_activity(&mut self, window: &Window) {
        let activity = DocumentActivity::new(
            window.is_window_active() && self.focus.is_focused(window),
            true,
        );
        if self.activity != Some(activity) {
            self.activity = Some(activity);
            self.send(TabCommand::Activity(activity));
        }
    }

    fn send(&self, command: TabCommand) {
        if let Some(commands) = self.commands.as_ref() {
            let _ = commands.send(command);
        }
    }

    fn cancel_pointer(&mut self) {
        if std::mem::take(&mut self.buttons) != 0
            && let Some((document, _)) = self.presented
        {
            self.send(TabCommand::CancelPointer {
                document,
                viewport_revision: self.revision,
            });
        }
    }

    fn measure(&mut self, bounds: Bounds<Pixels>, window: &Window, app: &App) {
        let moved = self
            .bounds
            .replace(bounds)
            .is_some_and(|old| old.origin != bounds.origin);
        if self.commands.is_none() || bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
            return;
        }
        let screen = window.display(app).map(|display| display.bounds());
        let outer = window.bounds();
        let width = f64::from(f32::from(bounds.size.width));
        let height = f64::from(f32::from(bounds.size.height));
        let mut surface = ViewportSurface {
            inner_width: (width / self.zoom).round().max(1.) as u32,
            inner_height: (height / self.zoom).round().max(1.) as u32,
            outer_width: f32::from(outer.size.width).round().max(1.) as u32,
            outer_height: f32::from(outer.size.height).round().max(1.) as u32,
            device_pixel_ratio: f64::from(window.scale_factor()) * self.zoom,
            window_x: f32::from(outer.origin.x).round() as i32,
            window_y: f32::from(outer.origin.y).round() as i32,
            ..ViewportSurface::default()
        };
        if let Some(screen) = screen {
            surface.screen_width = f32::from(screen.size.width).round() as u32;
            surface.screen_height = f32::from(screen.size.height).round() as u32;
            surface.screen_avail_width = surface.screen_width;
            surface.screen_avail_height = surface.screen_height;
        }
        if self.viewport != Some(surface) {
            self.cancel_pointer();
            self.viewport = Some(surface);
            self.revision += 1;
            self.presented = None;
            self.send(TabCommand::Viewport {
                surface,
                revision: self.revision,
                pointer_position: self.pointer_position(window),
            });
        } else if moved && self.pointer_over {
            self.sync_pointer_position(window);
        }
    }

    fn viewport_input_position(
        &self,
        point: Point<Pixels>,
    ) -> Option<(RendererDocumentLifecycleIdentity, (f64, f64))> {
        let (document, _) = self.presented?;
        if self.document != Some(document) || self.image.is_none() {
            return None;
        }
        Some((document, self.css_point(point)?))
    }

    fn css_point(&self, point: Point<Pixels>) -> Option<(f64, f64)> {
        let bounds = self.bounds?;
        let surface = self.viewport?;
        if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
            return None;
        }
        Some((
            f64::from((point.x - bounds.origin.x) / bounds.size.width)
                * f64::from(surface.inner_width),
            f64::from((point.y - bounds.origin.y) / bounds.size.height)
                * f64::from(surface.inner_height),
        ))
    }

    fn set_pointer_over(&mut self, over: bool, window: &Window) {
        if self.pointer_over != over {
            self.pointer_over = over;
            self.sync_pointer_position(window);
        }
    }

    fn sync_pointer_position(&self, window: &Window) {
        self.send(TabCommand::PointerPosition {
            viewport_revision: self.revision,
            position: self.pointer_position(window),
        });
    }

    fn pointer_position(&self, window: &Window) -> Option<(f64, f64)> {
        if self.pointer_over {
            self.css_point(window.mouse_position())
        } else {
            None
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_page(window, cx);
        self.sync_activity(window);
        if let Some((document, position)) = self.viewport_input_position(event.position) {
            let (button, mask) = mouse_button(event.button);
            self.buttons |= mask;
            self.send(TabCommand::Pointer(RendererViewportPointerInput {
                document,
                viewport_revision: self.revision,
                event: "mousedown",
                position,
                button,
                buttons: self.buttons,
                click_count: event.click_count as i32,
                modifiers: modifiers(event.modifiers),
            }));
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        let (button, mask) = mouse_button(event.button);
        if self.buttons & mask == 0 {
            return;
        }
        if let Some((document, position)) = self.viewport_input_position(event.position) {
            self.buttons &= !mask;
            self.send(TabCommand::Pointer(RendererViewportPointerInput {
                document,
                viewport_revision: self.revision,
                event: "mouseup",
                position,
                button,
                buttons: self.buttons,
                click_count: event.click_count as i32,
                modifiers: modifiers(event.modifiers),
            }));
        } else {
            self.cancel_pointer();
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, _: &mut Context<Self>) {
        self.pointer_over = true;
        if let Some((document, position)) = self.viewport_input_position(event.position) {
            self.send(TabCommand::Pointer(RendererViewportPointerInput {
                document,
                viewport_revision: self.revision,
                event: "mousemove",
                position,
                button: -1,
                buttons: self.buttons,
                click_count: 0,
                modifiers: modifiers(event.modifiers),
            }));
        }
    }

    fn wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        use RendererViewportWheelPhase as Phase;
        let Some((document, position)) = self.viewport_input_position(event.position) else {
            return;
        };
        let phase = match event.touch_phase {
            TouchPhase::Started => Phase::Started,
            TouchPhase::Moved
                if self
                    .wheel_route
                    .is_some_and(|route| route.document == document && route.phased) =>
            {
                Phase::Moved
            }
            // GPUI also represents unphased mouse wheels as Moved.
            TouchPhase::Moved => Phase::Unspecified,
            TouchPhase::Ended | TouchPhase::Cancelled => return,
        };
        let route = WheelRoute {
            document,
            viewport_revision: self.revision,
            phased: matches!(phase, Phase::Started | Phase::Moved),
        };
        self.wheel_route = Some(route);
        self.send_wheel(route, Some(position), event, phase);
        cx.stop_propagation();
    }

    fn finish_wheel(&mut self, event: &ScrollWheelEvent) {
        use RendererViewportWheelPhase as Phase;
        let phase = match event.touch_phase {
            TouchPhase::Ended => Phase::Ended,
            TouchPhase::Cancelled => Phase::Cancelled,
            TouchPhase::Started | TouchPhase::Moved => return,
        };
        let Some(route) = self.wheel_route.take() else {
            return;
        };
        let position = self
            .viewport_input_position(event.position)
            .filter(|(document, _)| {
                *document == route.document
                    && self.revision == route.viewport_revision
                    && self
                        .bounds
                        .is_some_and(|bounds| bounds.contains(&event.position))
            })
            .map(|(_, position)| position);
        self.send_wheel(route, position, event, phase);
    }

    fn send_wheel(
        &self,
        route: WheelRoute,
        position: Option<(f64, f64)>,
        event: &ScrollWheelEvent,
        phase: RendererViewportWheelPhase,
    ) {
        let delta = if position.is_some() {
            let delta = event.delta.pixel_delta(px(40.));
            (
                -f64::from(f32::from(delta.x)) / self.zoom,
                -f64::from(f32::from(delta.y)) / self.zoom,
            )
        } else {
            (0., 0.)
        };
        self.send(TabCommand::Wheel(RendererViewportWheelInput {
            document: route.document,
            viewport_revision: route.viewport_revision,
            position: position.unwrap_or_default(),
            buttons: self.buttons,
            delta,
            modifiers: modifiers(event.modifiers),
            phase,
            precise: event.delta.precise(),
        }));
    }

    fn cancel_wheel(&mut self) {
        if let Some(route) = self.wheel_route.take() {
            self.send(TabCommand::Wheel(RendererViewportWheelInput {
                document: route.document,
                viewport_revision: route.viewport_revision,
                position: (0., 0.),
                buttons: self.buttons,
                delta: (0., 0.),
                modifiers: 0,
                phase: RendererViewportWheelPhase::Cancelled,
                precise: false,
            }));
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let composing = self
            .text_input
            .editor()
            .is_some_and(|editor| editor.marked.is_some());
        if composing {
            match event.keystroke.key.as_str() {
                "escape" => {
                    self.apply_text_edit(RendererViewportEditAction::Cancel);
                    cx.stop_propagation();
                    return;
                }
                "enter" => return,
                _ => {}
            }
        }
        let primary = if cfg!(target_os = "macos") {
            event.keystroke.modifiers.platform
        } else {
            event.keystroke.modifiers.control
        };
        if primary && !event.keystroke.modifiers.alt {
            let (Some(document), shortcut @ ("c" | "x" | "v")) =
                (self.document, event.keystroke.key.as_str())
            else {
                self.key("keydown", &event.keystroke, event.is_held);
                return;
            };
            let action = if shortcut == "v" {
                let text = cx.read_from_clipboard().and_then(|item| item.text());
                let sequence = text
                    .as_ref()
                    .filter(|text| !text.is_empty())
                    .and_then(|text| {
                        self.text_input
                            .edit(RendererViewportEditAction::Paste { text: text.clone() })
                            .map(|edit| edit.sequence)
                    });
                ClipboardAction::Paste { text, sequence }
            } else {
                let (reply, text) = oneshot::channel();
                cx.spawn_in(window, async move |_, cx| {
                    if let Ok(Some(text)) = text.await {
                        let _ = cx
                            .update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(text)));
                    }
                })
                .detach();
                if shortcut == "x" {
                    ClipboardAction::Cut {
                        reply,
                        sequence: self.text_input.cut(),
                    }
                } else {
                    ClipboardAction::Copy(reply)
                }
            };
            self.send(TabCommand::Clipboard {
                document,
                key: viewport_key_input("keydown", &event.keystroke, event.is_held),
                action,
            });
            cx.stop_propagation();
            window.prevent_default();
            return;
        }
        self.key("keydown", &event.keystroke, event.is_held);
        if event.keystroke.key == "enter" {
            window.prevent_default();
            cx.stop_propagation();
        }
    }

    fn drop_files(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((document, position)) = self.viewport_input_position(window.mouse_position()) {
            self.send(TabCommand::DropFiles {
                document,
                position,
                paths: paths.paths().to_vec(),
                modifiers: modifiers(window.modifiers()),
            });
            cx.stop_propagation();
        }
    }

    fn key_up(&mut self, event: &KeyUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.key("keyup", &event.keystroke, false);
    }

    fn key(&self, event: &'static str, keystroke: &Keystroke, repeat: bool) {
        self.send(TabCommand::Key(viewport_key_input(
            event, keystroke, repeat,
        )));
    }
}

impl Render for MoliSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let surface = cx.entity();
        let focus = self.focus.clone();
        let cursor = self
            .output
            .as_ref()
            .and_then(|output| output.cursor())
            .filter(|cursor| {
                self.viewport.is_some()
                    && Some(cursor.document) == self.document
                    && cursor.viewport_revision == self.revision
            })
            .map_or(CursorStyle::Arrow, native_cursor);
        let mut content = div()
            .id("moli-surface")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0xffffff))
            .track_focus(&self.focus)
            .cursor(cursor)
            .on_hover(cx.listener(|surface, &over, window, _| {
                surface.set_pointer_over(over, window);
            }))
            .on_mouse_exit(cx.listener(|surface, _, window, _| {
                surface.set_pointer_over(false, window);
            }))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_scroll_wheel(cx.listener(Self::wheel))
            .on_drop(cx.listener(Self::drop_files))
            .on_key_down(cx.listener(Self::key_down))
            .on_key_up(cx.listener(Self::key_up));
        for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            content = content
                .on_mouse_down(button, cx.listener(Self::mouse_down))
                .on_mouse_up(button, cx.listener(Self::mouse_up))
                .on_mouse_up_out(button, cx.listener(Self::mouse_up));
        }
        content.child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, cx| {
                    let wheel_surface = surface.downgrade();
                    // End and Cancel still belong to this viewport when the
                    // pointer or the last painted frame leaves its hitbox.
                    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture
                            && matches!(
                                event.touch_phase,
                                TouchPhase::Ended | TouchPhase::Cancelled
                            )
                            && let Some(surface) = wheel_surface.upgrade()
                        {
                            surface.update(cx, |surface, _| surface.finish_wheel(event));
                        }
                    });
                    window.handle_input(
                        &focus,
                        ElementInputHandler::new(bounds, surface.clone()),
                        cx,
                    );
                    surface.update(cx, |surface, cx| {
                        if let Err(error) = surface.paint_image(bounds, window) {
                            eprintln!("Could not paint browser viewport: {error:#}");
                        }
                        surface.measure(bounds, window, cx);
                        if let Some(editor) = surface.text_input.editor() {
                            for (_, rect) in &editor.mark_rects {
                                if let Some(rect) = surface.window_rect(*rect) {
                                    window.paint_quad(
                                        PaintQuad::new(Bounds::new(
                                            point(rect.left(), rect.bottom() - px(1.)),
                                            size(rect.size.width, px(1.)),
                                        ))
                                        .background(rgb(0x267859)),
                                    );
                                }
                            }
                        }
                    });
                },
            )
            .absolute()
            .inset_0()
            .size_full(),
        )
    }
}

fn native_cursor(cursor: moli_core::RendererViewportCursor) -> CursorStyle {
    use RendererViewportCursorKind as Kind;
    match cursor.kind {
        Kind::Pointer => CursorStyle::PointingHand,
        Kind::Text => CursorStyle::IBeam,
        Kind::VerticalText => CursorStyle::IBeamCursorForVerticalLayout,
        Kind::Crosshair | Kind::Cell => CursorStyle::Crosshair,
        Kind::Grab => CursorStyle::OpenHand,
        Kind::Grabbing => CursorStyle::ClosedHand,
        Kind::Alias => CursorStyle::DragLink,
        Kind::Copy => CursorStyle::DragCopy,
        Kind::ContextMenu => CursorStyle::ContextualMenu,
        Kind::NotAllowed | Kind::NoDrop => CursorStyle::OperationNotAllowed,
        Kind::EResize => CursorStyle::ResizeRight,
        Kind::NResize => CursorStyle::ResizeUp,
        Kind::NeResize => CursorStyle::ResizeUpRight,
        Kind::NwResize => CursorStyle::ResizeUpLeft,
        Kind::SResize => CursorStyle::ResizeDown,
        Kind::SeResize => CursorStyle::ResizeDownRight,
        Kind::SwResize => CursorStyle::ResizeDownLeft,
        Kind::WResize => CursorStyle::ResizeLeft,
        Kind::EwResize => CursorStyle::ResizeLeftRight,
        Kind::NsResize => CursorStyle::ResizeUpDown,
        Kind::NeswResize => CursorStyle::ResizeUpRightDownLeft,
        Kind::NwseResize => CursorStyle::ResizeUpLeftDownRight,
        Kind::ColResize => CursorStyle::ResizeColumn,
        Kind::RowResize => CursorStyle::ResizeRow,
        // The platform API only exposes a subset of CSS cursor keywords.
        Kind::Auto
        | Kind::Default
        | Kind::None
        | Kind::Help
        | Kind::Progress
        | Kind::Wait
        | Kind::Move
        | Kind::AllScroll
        | Kind::ZoomIn
        | Kind::ZoomOut => CursorStyle::Arrow,
    }
}

impl EntityInputHandler for MoliSurface {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        self.text_input.text(range, adjusted)
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let editor = self.text_input.editor()?;
        Some(UTF16Selection {
            range: editor.selection.clone(),
            reversed: editor.reversed,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.text_input.editor()?.marked.clone()
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.apply_text_edit(RendererViewportEditAction::Finish);
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        if !self.apply_text_edit(RendererViewportEditAction::Commit {
            range,
            text: text.to_owned(),
        }) {
            self.send(TabCommand::InsertText(text.to_owned()));
        }
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        let action = if text.is_empty() {
            RendererViewportEditAction::Cancel
        } else {
            RendererViewportEditAction::Compose {
                range,
                text: text.to_owned(),
                selection,
            }
        };
        self.apply_text_edit(action);
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let editor = self.text_input.editor()?;
        let rect = editor
            .mark_rects
            .iter()
            .find(|(index, _)| *index == range.start)
            .map(|(_, rect)| *rect)
            .or(editor.caret_rect)?;
        self.window_rect(rect)
    }
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let editor = self.text_input.editor()?;
        for (index, rect) in &editor.mark_rects {
            if self.window_rect(*rect)?.contains(&point) {
                return Some(*index);
            }
        }
        self.window_rect(editor.caret_rect?)?
            .contains(&point)
            .then_some(editor.selection.end)
    }
}

fn mouse_button(button: MouseButton) -> (i32, i32) {
    match button {
        MouseButton::Left => (0, 1),
        MouseButton::Middle => (1, 4),
        MouseButton::Right => (2, 2),
        _ => (-1, 0),
    }
}

fn viewport_key_input(event: &'static str, keystroke: &Keystroke, repeat: bool) -> KeyInput {
    let (key, code) = dom_key(keystroke);
    KeyInput {
        event,
        key,
        code,
        repeat,
        modifiers: modifiers(keystroke.modifiers),
    }
}

fn modifiers(modifiers: Modifiers) -> u8 {
    u8::from(modifiers.alt)
        | u8::from(modifiers.control) << 1
        | u8::from(modifiers.platform) << 2
        | u8::from(modifiers.shift) << 3
}

fn dom_key(keystroke: &Keystroke) -> (String, String) {
    let name = keystroke.key.as_str();
    let standard = match name {
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "enter" => "Enter",
        "escape" => "Escape",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "tab" => "Tab",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "space" => return (" ".to_owned(), "Space".to_owned()),
        _ => {
            let code = if name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic() {
                format!("Key{}", name.to_ascii_uppercase())
            } else if name.len() == 1 && name.as_bytes()[0].is_ascii_digit() {
                format!("Digit{name}")
            } else {
                "Unidentified".to_owned()
            };
            return (
                keystroke
                    .key_char
                    .clone()
                    .unwrap_or_else(|| name.to_owned()),
                code,
            );
        }
    };
    (standard.to_owned(), standard.to_owned())
}

#[cfg(test)]
mod tests {
    use gpui::{ScrollDelta, TestAppContext, VisualTestContext};
    use moli_core::{
        PageId,
        page::{
            RendererDocumentToken, RendererFrameToken, RendererLifecycleEpoch, ViewportBuffers,
        },
    };

    use super::*;

    fn test_document() -> RendererDocumentLifecycleIdentity {
        let page_id = PageId::new_for_testing(1);
        RendererDocumentLifecycleIdentity {
            frame: RendererFrameToken { page_id },
            document: RendererDocumentToken::new_for_testing(page_id, 1),
            epoch: RendererLifecycleEpoch(0),
        }
    }

    #[test]
    fn viewport_resize_repaints_the_same_frame_without_scaling_bands() -> anyhow::Result<()> {
        let mut app = TestAppContext::single();
        let original = size(px(64.), px(512.));
        let resized = size(px(96.), px(768.));
        let handle = app.open_window(resized, MoliSurface::new);
        let mut cx = VisualTestContext::from_window(handle.into(), &app);
        let document = test_document();
        let (surface, viewport) = handle.update(&mut cx, |surface, window, cx| {
            let viewport = ViewportSurface {
                inner_width: 64,
                inner_height: 512,
                device_pixel_ratio: f64::from(window.scale_factor()),
                ..Default::default()
            };
            surface.document = Some(document);
            surface.viewport = Some(viewport);
            (cx.entity(), viewport)
        })?;
        let layout = ViewportBuffers::layout_for(viewport)?;
        let pixels = Rc::new(vec![255; layout.byte_len]);
        let versions = Rc::new(vec![
            1;
            layout.height.div_ceil(VIEWPORT_BAND_HEIGHT) as usize
        ]);

        for (draw_size, ready, banded) in [
            (original, true, true),
            (resized, false, true),
            (resized, true, false),
        ] {
            let (surface, pixels, versions) = (surface.clone(), pixels.clone(), versions.clone());
            cx.draw(Point::default(), draw_size, |_, _| {
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, cx| {
                        surface.update(cx, |surface, _| {
                            if ready {
                                let frame = ViewportFrameRef {
                                    document,
                                    viewport_revision: surface.revision,
                                    sequence: 1,
                                    layout,
                                    pixels: Some(pixels.as_slice()),
                                    band_versions: versions.as_slice(),
                                };
                                assert!(surface.paint_ready_image(frame, bounds, window).unwrap());
                            } else {
                                surface.paint_image(bounds, window).unwrap();
                            }
                            let image = surface.image.as_ref().unwrap();
                            assert_eq!(image.banded, banded);
                            let expected_size = if banded { original } else { draw_size };
                            assert_eq!(image.paint_bounds(bounds, window).size, expected_size);
                            assert_eq!(surface.presented, Some((document, 1)));
                        });
                    },
                )
                .size_full()
            });
        }
        Ok(())
    }

    #[test]
    fn pixel_wheel_uses_platform_phases_only_after_started() -> anyhow::Result<()> {
        use RendererViewportWheelPhase as Phase;

        let mut app = TestAppContext::single();
        let handle = app.open_window(size(px(160.), px(120.)), MoliSurface::new);
        let mut cx = VisualTestContext::from_window(handle.into(), &app);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let (commands, mut received) = mpsc::unbounded_channel();
        let document = test_document();
        handle.update(&mut cx, |surface, _, _| -> anyhow::Result<()> {
            surface.commands = Some(commands);
            surface.document = Some(document);
            surface.presented = Some((document, 1));
            surface.viewport = Some(ViewportSurface {
                inner_width: 160,
                inner_height: 120,
                ..Default::default()
            });
            surface.image = Some(ViewportImage {
                width: 1,
                height: 1,
                banded: false,
                bands: vec![ViewportBandImage {
                    start: 0,
                    end: 1,
                    version: 1,
                    texture: Rc::new(BgraImage::new(size(1_u32.into(), 1_u32.into()))?),
                }],
            });
            Ok(())
        })??;

        for (touch_phase, x, phase) in [
            (TouchPhase::Moved, 16., Phase::Unspecified),
            (TouchPhase::Started, 16., Phase::Started),
            (TouchPhase::Moved, 16., Phase::Moved),
            (TouchPhase::Ended, 200., Phase::Ended),
            (TouchPhase::Moved, 16., Phase::Unspecified),
        ] {
            cx.simulate_event(ScrollWheelEvent {
                position: point(px(x), px(16.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(-12.))),
                modifiers: Modifiers::default(),
                touch_phase,
            });
            let input = std::iter::from_fn(|| received.try_recv().ok())
                .find_map(|command| match command {
                    TabCommand::Wheel(input) => Some(input),
                    _ => None,
                })
                .expect("the viewport must forward the wheel sample");
            assert_eq!(input.phase, phase);
            assert!(input.precise);
            assert_eq!(input.document, document);
            if phase == Phase::Ended {
                assert_eq!(input.delta, (0., 0.), "End reached outside the hitbox");
            } else {
                assert_eq!(input.delta, (0., 12.));
            }
        }
        Ok(())
    }
}
