// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use accesskit::{Node, TextDecoration, TextDecorationStyle, TreeUpdate};
use core::default::Default;
use parley::editing::SplitString;
use parley::layout::{PositionedLayoutItem, Style};
use parley::{GenericFamily, StyleProperty};
use peniko::{
    Color, Fill,
    color::{AlphaColor, Srgb, palette},
    kurbo::{Affine, Line, Shape, Stroke},
};
use std::time::Duration;
use ui_events::pointer::PointerButton;
use ui_events::{
    keyboard::{Key, KeyboardEvent, NamedKey},
    pointer::{PointerButtonEvent, PointerEvent, PointerInfo, PointerState, PointerType},
};
use vello_cpu::{Glyph, RenderContext, Resources};
use winit::event::{Ime, WindowEvent};

use crate::access_ids::next_node_id;
use crate::accessibility::PlainEditorAccessibility;
pub use parley::editing::Generation;
use parley::{FontContext, LayoutContext, PlainEditor, PlainEditorDriver};

pub const BACKGROUND_COLOR: AlphaColor<Srgb> = palette::css::STEEL_BLUE;
pub const INSET: f32 = 32.0;

/// A newtype wrapper around [`peniko::Color`] used as the [`Brush`](parley::style::Brush) for text.
///
/// `vello_cpu`'s `PaintType` uses its own image/gradient types, so we can't pass a
/// `peniko::Brush` directly. This example only ever uses solid-color brushes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorBrush(pub Color);

impl Default for ColorBrush {
    fn default() -> Self {
        Self(Color::BLACK)
    }
}

impl From<Color> for ColorBrush {
    fn from(color: Color) -> Self {
        Self(color)
    }
}

pub struct Editor {
    font_cx: FontContext,
    layout_cx: LayoutContext<ColorBrush>,
    editor: PlainEditor<ColorBrush>,
    access: PlainEditorAccessibility,
    cursor_visible: bool,
    start_time: Option<Instant>,
    blink_period: Duration,
}

fn convert_rect(rect: &parley::BoundingBox) -> peniko::kurbo::Rect {
    peniko::kurbo::Rect::new(rect.x0, rect.y0, rect.x1, rect.y1)
}

/// Set the current paint from a [`ColorBrush`].
fn set_brush(renderer: &mut RenderContext, brush: &ColorBrush) {
    renderer.set_paint(brush.0);
}

fn to_accesskit_color(brush: &ColorBrush) -> accesskit::Color {
    let rgba = brush.0.to_rgba8();
    accesskit::Color {
        red: rgba.r,
        green: rgba.g,
        blue: rgba.b,
        alpha: rgba.a,
    }
}

fn set_accesskit_brush_properties(node: &mut Node, style: &Style<ColorBrush>) {
    node.set_foreground_color(to_accesskit_color(&style.brush));
    if let Some(deco) = &style.underline {
        node.set_underline(TextDecoration {
            style: TextDecorationStyle::Solid,
            color: to_accesskit_color(&deco.brush),
        });
    }
    if let Some(deco) = &style.strikethrough {
        node.set_strikethrough(TextDecoration {
            style: TextDecorationStyle::Solid,
            color: to_accesskit_color(&deco.brush),
        });
    }
}

impl Editor {
    pub fn new(text: &str) -> Self {
        let mut editor = PlainEditor::new(32.0);
        editor.set_text(text);
        editor.set_scale(1.0);
        let styles = editor.edit_styles();
        styles.insert(GenericFamily::SystemUi.into());
        styles.insert(StyleProperty::Brush(ColorBrush(palette::css::WHITE)));
        Self {
            font_cx: FontContext::default(),
            layout_cx: LayoutContext::default(),
            editor,
            access: PlainEditorAccessibility::default(),
            cursor_visible: false,
            start_time: None,
            // TODO: Why initialize to zero?
            blink_period: Duration::ZERO,
        }
    }

    fn driver(&mut self) -> PlainEditorDriver<'_, ColorBrush> {
        self.editor.driver(&mut self.font_cx, &mut self.layout_cx)
    }

    pub fn editor(&mut self) -> &mut PlainEditor<ColorBrush> {
        &mut self.editor
    }

    pub fn text(&self) -> SplitString<'_> {
        self.editor.text()
    }

    pub fn cursor_reset(&mut self) {
        self.start_time = Some(Instant::now());
        // TODO: for real world use, this should be reading from the system settings
        self.blink_period = Duration::from_millis(500);
        self.cursor_visible = true;
    }

    pub fn disable_blink(&mut self) {
        self.start_time = None;
    }

    pub fn next_blink_time(&self) -> Option<Instant> {
        self.start_time.map(|start_time| {
            let phase = Instant::now().duration_since(start_time);

            start_time
                + Duration::from_nanos(
                    ((phase.as_nanos() / self.blink_period.as_nanos() + 1)
                        * self.blink_period.as_nanos()) as u64,
                )
        })
    }

    pub fn cursor_blink(&mut self) {
        self.cursor_visible = self.start_time.is_some_and(|start_time| {
            let elapsed = Instant::now().duration_since(start_time);
            (elapsed.as_millis() / self.blink_period.as_millis()).is_multiple_of(2)
        });
    }

    pub fn handle_keyboard_event(&mut self, event: &KeyboardEvent) {
        match event {
            KeyboardEvent {
                state,
                key,
                modifiers,
                ..
            } if !self.editor.is_composing() && state.is_down() => {
                self.cursor_reset();
                let mut drv = self.editor.driver(&mut self.font_cx, &mut self.layout_cx);
                let action_mod = if cfg!(target_os = "macos") {
                    modifiers.meta()
                } else {
                    modifiers.ctrl()
                };
                let shift = modifiers.shift();

                match key {
                    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
                    Key::Character(c) if action_mod && matches!(c.as_str(), "c" | "x" | "v") => {
                        use clipboard_rs::{Clipboard, ClipboardContext};
                        match c.to_lowercase().as_str() {
                            "c" => {
                                if let Some(text) = drv.editor.selected_text() {
                                    let cb = ClipboardContext::new().unwrap();
                                    cb.set_text(text.to_owned()).ok();
                                }
                            }
                            "x" => {
                                if let Some(text) = drv.editor.selected_text() {
                                    let cb = ClipboardContext::new().unwrap();
                                    cb.set_text(text.to_owned()).ok();
                                    drv.delete_selection();
                                }
                            }
                            "v" => {
                                let cb = ClipboardContext::new().unwrap();
                                let text = cb.get_text().unwrap_or_default();
                                drv.insert_or_replace_selection(&text);
                            }
                            _ => (),
                        }
                    }
                    Key::Character(c) if action_mod && matches!(c.to_lowercase().as_str(), "a") => {
                        if shift {
                            drv.collapse_selection();
                        } else {
                            drv.select_all();
                        }
                    }
                    Key::Named(NamedKey::ArrowLeft) => {
                        if action_mod {
                            if shift {
                                drv.select_word_left();
                            } else {
                                drv.move_word_left();
                            }
                        } else if shift {
                            drv.select_left();
                        } else {
                            drv.move_left();
                        }
                    }
                    Key::Named(NamedKey::ArrowRight) => {
                        if action_mod {
                            if shift {
                                drv.select_word_right();
                            } else {
                                drv.move_word_right();
                            }
                        } else if shift {
                            drv.select_right();
                        } else {
                            drv.move_right();
                        }
                    }
                    Key::Named(NamedKey::ArrowUp) => {
                        if shift {
                            drv.select_up();
                        } else {
                            drv.move_up();
                        }
                    }
                    Key::Named(NamedKey::ArrowDown) => {
                        if shift {
                            drv.select_down();
                        } else {
                            drv.move_down();
                        }
                    }
                    Key::Named(NamedKey::Home) => {
                        if action_mod {
                            if shift {
                                drv.select_to_text_start();
                            } else {
                                drv.move_to_text_start();
                            }
                        } else if shift {
                            drv.select_to_line_start();
                        } else {
                            drv.move_to_line_start();
                        }
                    }
                    Key::Named(NamedKey::End) => {
                        let this = &mut *self;
                        let mut drv = this.driver();

                        if action_mod {
                            if shift {
                                drv.select_to_text_end();
                            } else {
                                drv.move_to_text_end();
                            }
                        } else if shift {
                            drv.select_to_line_end();
                        } else {
                            drv.move_to_line_end();
                        }
                    }
                    Key::Named(NamedKey::Delete) => {
                        if action_mod {
                            drv.delete_word();
                        } else {
                            drv.delete();
                        }
                    }
                    Key::Named(NamedKey::Backspace) => {
                        if action_mod {
                            drv.backdelete_word();
                        } else {
                            drv.backdelete();
                        }
                    }
                    Key::Named(NamedKey::Enter) => {
                        drv.insert_or_replace_selection("\n");
                    }
                    Key::Character(s) => {
                        drv.insert_or_replace_selection(s);
                    }
                    _ => (),
                }
            }
            _ => {}
        }
    }

    pub fn handle_pointer_event(&mut self, event: &PointerEvent) {
        let pressed = |i: &PointerInfo, s: &PointerState| {
            s.buttons.contains(PointerButton::Primary)
                || (s.pressure >= 0.5
                    && matches!(i.pointer_type, PointerType::Touch | PointerType::Pen))
        };
        match event {
            // TODO: Handle touch long press specially, for SelectWordAtPoint.
            PointerEvent::Down(PointerButtonEvent { pointer, state, .. })
                if pressed(pointer, state) && !self.editor.is_composing() =>
            {
                self.cursor_reset();
                let mut drv = self.editor.driver(&mut self.font_cx, &mut self.layout_cx);
                let cursor_pos = (
                    state.position.x as f32 - INSET,
                    state.position.y as f32 - INSET,
                );
                match state.count {
                    2 => drv.select_word_at_point(cursor_pos.0, cursor_pos.1),
                    3 => drv.select_hard_line_at_point(cursor_pos.0, cursor_pos.1),
                    _ => {
                        if state.modifiers.shift() {
                            drv.shift_click_extension(cursor_pos.0, cursor_pos.1);
                        } else {
                            drv.move_to_point(cursor_pos.0, cursor_pos.1);
                        }
                    }
                }
            }
            PointerEvent::Move(u)
                if pressed(&u.pointer, &u.current) && !self.editor.is_composing() =>
            {
                let cursor_pos = (
                    u.current.position.x as f32 - INSET,
                    u.current.position.y as f32 - INSET,
                );
                self.cursor_reset();
                self.driver()
                    .extend_selection_to_point(cursor_pos.0, cursor_pos.1);
            }
            _ => {}
        }
    }

    pub fn handle_event(&mut self, event: WindowEvent) {
        match event {
            WindowEvent::Resized(size) => {
                self.editor
                    .set_width(Some(size.width as f32 - 2_f32 * INSET));
            }
            WindowEvent::Ime(Ime::Disabled) => {
                self.driver().clear_compose();
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.driver().insert_or_replace_selection(&text);
            }
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                if text.is_empty() {
                    self.driver().clear_compose();
                } else {
                    self.driver().set_compose(&text, cursor);
                }
            }
            _ => {}
        }
    }

    pub fn handle_accesskit_action_request(&mut self, req: &accesskit::ActionRequest) {
        let mut drv = self.editor.driver(&mut self.font_cx, &mut self.layout_cx);
        self.access.handle_action_request(&mut drv, req);
    }

    /// Return the current `Generation` of the layout.
    pub fn generation(&self) -> Generation {
        self.editor.generation()
    }

    /// Draw into the `vello_cpu` render context.
    ///
    /// Returns drawn `Generation`.
    pub fn draw(&mut self, renderer: &mut RenderContext, resources: &mut Resources) -> Generation {
        let transform = Affine::translate((INSET as f64, INSET as f64));
        renderer.set_transform(transform);
        renderer.set_fill_rule(Fill::NonZero);
        renderer.set_paint(BACKGROUND_COLOR);
        self.editor.selection_geometry_with(|rect, _| {
            renderer.fill_rect(&convert_rect(&rect));
        });
        if self.cursor_visible
            && let Some(cursor) = self.editor.cursor_geometry(1.5)
        {
            renderer.set_paint(palette::css::WHITE);
            renderer.fill_rect(&convert_rect(&cursor));
        }
        let layout = self.editor.layout(&mut self.font_cx, &mut self.layout_cx);
        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let style = glyph_run.style();
                // We draw underlines under the text, then the strikethrough on top, following:
                // https://drafts.csswg.org/css-text-decor/#painting-order
                if let Some(underline) = &style.underline {
                    let run_metrics = glyph_run.run().font_metrics();
                    let offset = match underline.offset {
                        Some(offset) => offset,
                        None => run_metrics.underline_offset,
                    };
                    let width = match underline.size {
                        Some(size) => size,
                        None => run_metrics.underline_size,
                    };
                    // The `offset` is the distance from the baseline to the top of the underline
                    // so we move the line down by half the width
                    // Remember that we are using a y-down coordinate system
                    // If there's a custom width, because this is an underline, we want the custom
                    // width to go down from the default expectation
                    let y = glyph_run.baseline() - offset + width / 2.;

                    let line = Line::new(
                        (glyph_run.offset() as f64, y as f64),
                        ((glyph_run.offset() + glyph_run.advance()) as f64, y as f64),
                    );
                    renderer.set_stroke(Stroke::new(width.into()));
                    set_brush(renderer, &style.brush);
                    renderer.stroke_path(&line.to_path(0.0));
                }
                let run = glyph_run.run();
                let font = run.font();
                let font_size = run.font_size();
                let synthesis = run.synthesis();
                let glyph_xform = synthesis
                    .skew()
                    .map(|angle| Affine::skew(angle.to_radians().tan() as f64, 0.0));
                set_brush(renderer, &style.brush);
                let normalized_coords = bytemuck::cast_slice::<_, i16>(run.normalized_coords());
                let mut builder = renderer
                    .glyph_run(resources, &font.font)
                    .font_size(font_size)
                    .hint(true)
                    .normalized_coords(normalized_coords);
                if let Some(glyph_xform) = glyph_xform {
                    builder = builder.glyph_transform(glyph_xform);
                }
                builder.fill_glyphs(glyph_run.positioned_glyphs().map(|glyph| Glyph {
                    id: glyph.id,
                    x: glyph.x,
                    y: glyph.y,
                }));
                if let Some(strikethrough) = &style.strikethrough {
                    let run_metrics = glyph_run.run().font_metrics();
                    let offset = match strikethrough.offset {
                        Some(offset) => offset,
                        None => run_metrics.strikethrough_offset,
                    };
                    let width = match strikethrough.size {
                        Some(size) => size,
                        None => run_metrics.strikethrough_size,
                    };
                    // The `offset` is the distance from the baseline to the *top* of the strikethrough
                    // so we calculate the middle y-position of the strikethrough based on the font's
                    // standard strikethrough width.
                    // Remember that we are using a y-down coordinate system
                    let y = glyph_run.baseline() - offset + run_metrics.strikethrough_size / 2.;

                    let line = Line::new(
                        (glyph_run.offset() as f64, y as f64),
                        ((glyph_run.offset() + glyph_run.advance()) as f64, y as f64),
                    );
                    renderer.set_stroke(Stroke::new(width.into()));
                    set_brush(renderer, &style.brush);
                    renderer.stroke_path(&line.to_path(0.0));
                }
            }
        }
        self.editor.generation()
    }

    pub fn accessibility(&mut self, update: &mut TreeUpdate, node: &mut Node) {
        let mut drv = self.editor.driver(&mut self.font_cx, &mut self.layout_cx);
        self.access.build_nodes(
            &mut drv,
            update,
            node,
            next_node_id,
            INSET.into(),
            INSET.into(),
            set_accesskit_brush_properties,
        );
    }
}

pub const LOREM: &str = r" Lorem ipsum dolor sit amet, consectetur adipiscing elit. Morbi cursus mi sed euismod euismod. Orci varius natoque penatibus et magnis dis parturient montes, nascetur ridiculus mus. Nullam placerat efficitur tellus at semper. Morbi ac risus magna. Donec ut cursus ex. Etiam quis posuere tellus. Mauris posuere dui et turpis mollis, vitae luctus tellus consectetur. Lorem ipsum dolor sit amet, consectetur adipiscing elit. Curabitur eu facilisis nisl.

Phasellus in viverra dolor, vitae facilisis est. Maecenas malesuada massa vel ultricies feugiat. Vivamus venenatis et gהתעשייה בנושא האינטרנטa nibh nec pharetra. Phasellus vestibulum elit enim, nec scelerisque orci faucibus id. Vivamus consequat purus sit amet orci egestas, non iaculis massa porttitor. Vestibulum ut eros leo. In fermentum convallis magna in finibus. Donec justo leo, maximus ac laoreet id, volutpat ut elit. Mauris sed leo non neque laoreet faucibus. Aliquam orci arcu, faucibus in molestie eget, ornare non dui. Donec volutpat nulla in fringilla elementum. Aliquam vitae ante egestas ligula tempus vestibulum sit amet sed ante. ";
