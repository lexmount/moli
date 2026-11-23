use std::{ops::Range, sync::Arc};

use gpui::{
    App, Bounds, ClipboardItem, Context, ElementInputHandler, EntityInputHandler, ExternalPaths,
    FocusHandle, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ObjectFit, PaintQuad, Pixels, Point, RenderImage,
    ScrollWheelEvent, Subscription, UTF16Selection, Window, canvas, div, img, point, prelude::*,
    px, rgb, size,
};
use moli_core::{
    RendererDocumentLifecycleIdentity, RendererViewportEditAction, RendererViewportEditor,
    protocol_types::{DocumentActivity, ViewportSurface},
};
use tokio::sync::{mpsc, oneshot};

use crate::host::{DisplayFrame, KeyInput, PointerInput, TabCommand};
use crate::text_input::MoliTextInputBridge;

pub struct MoliSurface {
    pub focus: FocusHandle,
    commands: Option<mpsc::UnboundedSender<TabCommand>>,
    activity: Option<DocumentActivity>,
    bounds: Option<Bounds<Pixels>>,
    viewport: Option<ViewportSurface>,
    revision: u64,
    zoom: f64,
    buttons: i32,
    image: Option<Arc<RenderImage>>,
    document: Option<RendererDocumentLifecycleIdentity>,
    presented: Option<(RendererDocumentLifecycleIdentity, u64)>,
    text_input: MoliTextInputBridge,
    _subscriptions: Vec<Subscription>,
}

impl MoliSurface {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let activation = cx.observe_window_activation(window, |surface, window, _| {
            if !window.is_window_active() {
                surface.finish_composition();
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
            activity: None,
            bounds: None,
            viewport: None,
            revision: 0,
            zoom: 1.,
            buttons: 0,
            image: None,
            document: None,
            presented: None,
            text_input: MoliTextInputBridge::default(),
            _subscriptions: vec![activation, focus_in, focus_out],
        }
    }

    pub fn select_tab(
        &mut self,
        commands: mpsc::UnboundedSender<TabCommand>,
        zoom: f64,
        document: Option<RendererDocumentLifecycleIdentity>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.finish_composition();
        self.send(TabCommand::Activity(DocumentActivity::new(false, false)));
        self.commands = Some(commands);
        self.activity = None;
        self.viewport = None;
        self.document = document;
        self.buttons = 0;
        self.zoom = zoom;
        self.clear_frame(window, cx);
        self.sync_activity(window);
        cx.notify();
    }

    pub fn set_zoom(&mut self, zoom: f64, cx: &mut Context<Self>) {
        self.zoom = zoom;
        cx.notify();
    }

    pub fn clear_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.presented = None;
        if let Some(image) = self.image.take() {
            let _ = window.drop_image(image);
            cx.notify();
        }
    }

    pub fn show_frame(
        &mut self,
        frame: &DisplayFrame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.viewport.is_none()
            || self.revision != frame.viewport_revision
            || self.presented.is_some_and(|(document, sequence)| {
                document == frame.document && sequence >= frame.sequence
            })
        {
            return;
        }
        self.clear_frame(window, cx);
        self.image = frame.image.clone();
        self.presented = Some((frame.document, frame.sequence));
        cx.notify();
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

    fn measure(&mut self, bounds: Bounds<Pixels>, window: &Window, app: &App) {
        self.bounds = Some(bounds);
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
            self.viewport = Some(surface);
            self.revision += 1;
            self.presented = None;
            self.send(TabCommand::Viewport(surface, self.revision));
        }
    }

    fn css_point(&self, point: Point<Pixels>) -> Option<(f64, f64)> {
        let bounds = self.bounds?;
        let surface = self.viewport?;
        Some((
            f64::from((point.x - bounds.origin.x) / bounds.size.width)
                * f64::from(surface.inner_width),
            f64::from((point.y - bounds.origin.y) / bounds.size.height)
                * f64::from(surface.inner_height),
        ))
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_page(window, cx);
        self.sync_activity(window);
        let (button, mask) = mouse_button(event.button);
        self.buttons |= mask;
        if let Some(position) = self.css_point(event.position) {
            self.send(TabCommand::Pointer(PointerInput {
                event: "mousedown",
                position,
                button,
                buttons: self.buttons,
                click_count: event.click_count as i32,
                delta: (0., 0.),
                modifiers: modifiers(event.modifiers),
            }));
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        let (button, mask) = mouse_button(event.button);
        if self.buttons & mask == 0 {
            return;
        }
        self.buttons &= !mask;
        if let Some(position) = self.css_point(event.position) {
            self.send(TabCommand::Pointer(PointerInput {
                event: "mouseup",
                position,
                button,
                buttons: self.buttons,
                click_count: event.click_count as i32,
                delta: (0., 0.),
                modifiers: modifiers(event.modifiers),
            }));
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, _: &mut Context<Self>) {
        if let Some(position) = self.css_point(event.position) {
            self.send(TabCommand::Pointer(PointerInput {
                event: "mousemove",
                position,
                button: -1,
                buttons: self.buttons,
                click_count: 0,
                delta: (0., 0.),
                modifiers: modifiers(event.modifiers),
            }));
        }
    }

    fn wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(position) = self.css_point(event.position) {
            let delta = event.delta.pixel_delta(px(40.));
            self.send(TabCommand::Pointer(PointerInput {
                event: "wheel",
                position,
                button: -1,
                buttons: self.buttons,
                click_count: 0,
                delta: (
                    -f64::from(f32::from(delta.x)) / self.zoom,
                    -f64::from(f32::from(delta.y)) / self.zoom,
                ),
                modifiers: modifiers(event.modifiers),
            }));
            cx.stop_propagation();
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
            match (event.keystroke.key.as_str(), self.document) {
                ("c", Some(document)) => {
                    let (reply, text) = oneshot::channel();
                    self.send(TabCommand::Copy { document, reply });
                    cx.spawn_in(window, async move |_, cx| {
                        if let Ok(Some(text)) = text.await {
                            let _ = cx.update(|_, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(text))
                            });
                        }
                    })
                    .detach();
                }
                ("v", _) if self.text_input.editor().is_some() => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        self.apply_text_edit(RendererViewportEditAction::Paste { text });
                    }
                }
                _ => {
                    self.key("keydown", &event.keystroke, event.is_held);
                    return;
                }
            }
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
        if let Some(document) = self.document
            && let Some(position) = self.css_point(window.mouse_position())
        {
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
        let (key, code) = dom_key(keystroke);
        self.send(TabCommand::Key(KeyInput {
            event,
            key,
            code,
            repeat,
            modifiers: modifiers(keystroke.modifiers),
        }));
    }
}

impl Render for MoliSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let surface = cx.entity();
        let focus = self.focus.clone();
        let mut content = div()
            .id("moli-surface")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(rgb(0xffffff))
            .track_focus(&self.focus)
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
        if let Some(image) = self.image.clone() {
            content = content.child(img(image).size_full().object_fit(ObjectFit::Fill));
        }
        content.child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, cx| {
                    window.handle_input(
                        &focus,
                        ElementInputHandler::new(bounds, surface.clone()),
                        cx,
                    );
                    surface.update(cx, |surface, cx| {
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
