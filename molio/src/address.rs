use std::{collections::VecDeque, ops::Range};

use gpui::{
    Bounds, CaretPosition, ClipboardItem, ContentMask, Context, ElementInputHandler,
    EntityInputHandler, EventEmitter, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, PaintQuad, Pixels, Point, ShapedLine, SharedString, TextAlign, TextRun,
    UTF16Selection, UnderlineStyle, Window, canvas, div, point, prelude::*, px, rgb_to_hsla, size,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::palette::Palette;

const LINE_HEIGHT: Pixels = px(19.);

pub enum AddressEvent {
    Submit(String),
    Cancel,
}

pub struct AddressSuggestion {
    pub url: String,
    pub title: String,
}

pub struct Address {
    pub focus: FocusHandle,
    placeholder: &'static str,
    value: String,
    anchor: usize,
    head: usize,
    marked: Option<Range<usize>>,
    history: VecDeque<AddressSuggestion>,
    suggest: bool,
    pub selected_suggestion: Option<usize>,
    geometry: Option<(Point<Pixels>, ShapedLine)>,
}

impl EventEmitter<AddressEvent> for Address {}

impl Address {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self::with_placeholder("Search or enter an address", cx)
    }

    pub fn with_placeholder(placeholder: &'static str, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            placeholder,
            value: String::new(),
            anchor: 0,
            head: 0,
            marked: None,
            history: VecDeque::new(),
            suggest: false,
            selected_suggestion: None,
            geometry: None,
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn show(&mut self, url: String, cx: &mut Context<Self>) {
        self.value = url;
        self.head = self.value.len();
        self.anchor = self.head;
        self.marked = None;
        self.suggest = false;
        self.selected_suggestion = None;
        cx.notify();
    }

    pub fn remember(&mut self, url: &url::Url, title: &str, cx: &mut Context<Self>) {
        if !matches!(url.scheme(), "http" | "https") {
            return;
        }
        self.history.retain(|entry| entry.url != url.as_str());
        self.history.push_front(AddressSuggestion {
            url: url.to_string(),
            title: title.to_owned(),
        });
        self.history.truncate(128);
        self.selected_suggestion = None;
        if self.suggest {
            cx.notify();
        }
    }

    pub fn suggestions(&self) -> Vec<&AddressSuggestion> {
        let query = self.value.trim().to_lowercase();
        if !self.suggest || self.marked.is_some() || query.is_empty() {
            return Vec::new();
        }
        self.history
            .iter()
            .filter(|entry| {
                entry.url != self.value
                    && (entry.url.to_lowercase().contains(&query)
                        || entry.title.to_lowercase().contains(&query))
            })
            .take(6)
            .collect()
    }

    pub fn select_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        self.anchor = 0;
        self.head = self.value.len();
        cx.notify();
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    fn replacement(&self, range: Option<Range<usize>>) -> Range<usize> {
        range
            .map(|range| utf8_range(&self.value, range))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection())
    }

    fn replace(&mut self, range: Range<usize>, text: &str) {
        self.head = range.start + text.len();
        self.anchor = self.head;
        self.value.replace_range(range, text);
        self.marked = None;
        self.suggest = true;
        self.selected_suggestion = None;
    }

    fn nearest_index(&self, position: Point<Pixels>) -> usize {
        self.geometry
            .as_ref()
            .filter(|(_, line)| line.text.as_str() == self.value)
            .map_or(self.head, |(origin, line)| {
                line.platform_layout
                    .caret_from_pixel_point(
                        point(position.x - origin.x, LINE_HEIGHT / 2.),
                        LINE_HEIGHT,
                    )
                    .unwrap_or_else(|caret| caret)
                    .index
            })
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        self.head = self.nearest_index(event.position);
        if !event.modifiers.shift {
            self.anchor = self.head;
        }
        if event.click_count > 1 {
            self.select_all(window, cx);
        }
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.pressed_button == Some(MouseButton::Left) {
            self.head = self.nearest_index(event.position);
            cx.notify();
        }
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        let selection = self.selection();
        let shortcut = if cfg!(target_os = "macos") {
            modifiers.platform
        } else {
            modifiers.control
        };
        if self.marked.is_some() && matches!(key, "enter" | "escape") {
            return;
        }
        match key {
            "enter" => {
                let target = self
                    .selected_suggestion
                    .and_then(|index| self.suggestions().get(index).map(|entry| entry.url.clone()))
                    .unwrap_or_else(|| self.value.clone());
                cx.emit(AddressEvent::Submit(target));
            }
            "escape" if !self.suggestions().is_empty() => {
                self.suggest = false;
                self.selected_suggestion = None;
            }
            "escape" => cx.emit(AddressEvent::Cancel),
            "up" | "down" if !self.suggestions().is_empty() => {
                let length = self.suggestions().len();
                let index = match (key, self.selected_suggestion) {
                    ("up", Some(index)) => (index + length - 1) % length,
                    ("up", None) => length - 1,
                    (_, Some(index)) => (index + 1) % length,
                    (_, None) => 0,
                };
                self.selected_suggestion = Some(index);
            }
            "a" if shortcut => self.select_all(window, cx),
            "c" | "x" if shortcut && !selection.is_empty() => {
                cx.write_to_clipboard(ClipboardItem::new_string(
                    self.value[selection.clone()].to_owned(),
                ));
                if key == "x" {
                    self.replace(selection, "");
                }
            }
            "v" if shortcut => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.replace(self.replacement(None), &single_line(&text));
                }
            }
            "left" | "right" | "home" | "end" => {
                let end = self.value.len();
                self.head = match key {
                    "home" => 0,
                    "end" => end,
                    "left" if !modifiers.shift && !selection.is_empty() => selection.start,
                    "right" if !modifiers.shift && !selection.is_empty() => selection.end,
                    "left" if modifiers.platform => 0,
                    "right" if modifiers.platform => end,
                    "left" => previous_boundary(&self.value, self.head),
                    _ => next_boundary(&self.value, self.head),
                };
                if !modifiers.shift {
                    self.anchor = self.head;
                }
            }
            "backspace" | "delete" => {
                let range = if !selection.is_empty() {
                    selection
                } else if key == "backspace" {
                    previous_boundary(&self.value, self.head)..self.head
                } else {
                    self.head..next_boundary(&self.value, self.head)
                };
                self.replace(range, "");
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for Address {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_window(window.appearance());
        let focused = self.focus.is_focused(window);
        let focus = self.focus.clone();
        let text: SharedString = if self.value.is_empty() {
            self.placeholder.into()
        } else {
            self.value.clone().into()
        };
        let placeholder = self.value.is_empty();
        let font = window.text_style().font();
        let mut runs = Vec::new();
        let run = TextRun {
            len: text.len(),
            font,
            color: rgb_to_hsla(if placeholder {
                palette.muted
            } else {
                palette.text
            }),
            background_color: None,
            underline: None,
            strikethrough: None,
            letter_spacing: None,
        };
        if let Some(marked) = self.marked.as_ref() {
            runs.extend(
                [
                    TextRun {
                        len: marked.start,
                        ..run.clone()
                    },
                    TextRun {
                        len: marked.len(),
                        underline: Some(UnderlineStyle {
                            thickness: px(1.),
                            color: Some(rgb_to_hsla(palette.accent)),
                            wavy: false,
                        }),
                        ..run.clone()
                    },
                    TextRun {
                        len: self.value.len() - marked.end,
                        ..run
                    },
                ]
                .into_iter()
                .filter(|run| run.len > 0),
            );
        } else {
            runs.push(run);
        }
        let caret = self.head;
        let selection = self.selection();
        let address = cx.entity();
        div()
            .id("address-entry")
            .flex_1()
            .h_full()
            .min_w_0()
            .cursor_text()
            .overflow_hidden()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .child(
                canvas(
                    move |bounds, window, _| {
                        let line = window.text_system().shape_line(text, px(14.), &runs);
                        let caret_x = line
                            .platform_layout
                            .caret_bounds(
                                CaretPosition::attached_to_next_cluster(caret),
                                LINE_HEIGHT,
                            )
                            .map_or(Pixels::ZERO, |rect| rect.origin.x);
                        let scroll = (caret_x - bounds.size.width + px(3.)).max(Pixels::ZERO);
                        let origin = point(
                            bounds.left() - scroll,
                            bounds.top() + (bounds.size.height - LINE_HEIGHT) / 2.,
                        );
                        (line, origin, caret_x)
                    },
                    move |bounds, (line, origin, caret_x), window, app| {
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, address.clone()),
                            app,
                        );
                        window.with_content_mask(
                            Some(ContentMask {
                                bounds,
                                ..Default::default()
                            }),
                            |window| {
                                if focused && !selection.is_empty() {
                                    for rect in line
                                        .platform_layout
                                        .selection_bounds(selection.clone(), LINE_HEIGHT)
                                    {
                                        window.paint_quad(
                                            PaintQuad::new(Bounds::new(
                                                origin + rect.origin,
                                                rect.size,
                                            ))
                                            .background(palette.selection),
                                        );
                                    }
                                }
                                let _ = line.paint(
                                    origin,
                                    LINE_HEIGHT,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    app,
                                );
                                if focused && selection.is_empty() {
                                    window.paint_quad(
                                        PaintQuad::new(Bounds::new(
                                            point(origin.x + caret_x, origin.y + px(1.)),
                                            size(px(1.), LINE_HEIGHT - px(2.)),
                                        ))
                                        .background(palette.accent),
                                    );
                                }
                            },
                        );
                        address.update(app, |view, _| view.geometry = Some((origin, line)));
                    },
                )
                .size_full(),
            )
    }
}

impl EntityInputHandler for Address {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = utf8_range(&self.value, range);
        *adjusted = Some(utf16_range(&self.value, range.clone()));
        Some(self.value[range].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: utf16_range(&self.value, self.selection()),
            reversed: self.head < self.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .clone()
            .map(|range| utf16_range(&self.value, range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace(self.replacement(range), &single_line(text));
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.replacement(range);
        let start = range.start;
        let text = single_line(text);
        self.replace(range, &text);
        self.marked = (!text.is_empty()).then_some(start..start + text.len());
        if let Some(selection) = selection {
            let selection = utf8_range(&text, selection);
            self.anchor = start + selection.start;
            self.head = start + selection.end;
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (origin, line) = self.geometry.as_ref()?;
        let range = utf8_range(&self.value, range);
        let caret = CaretPosition::attached_to_next_cluster(range.start.min(line.len()));
        let rect = if range.is_empty() || line.text.as_str() != self.value {
            None
        } else {
            line.platform_layout
                .selection_bounds(range, LINE_HEIGHT)
                .into_iter()
                .reduce(|bounds, next| bounds.union(&next))
        }
        .or_else(|| line.platform_layout.caret_bounds(caret, LINE_HEIGHT))?;
        Some(Bounds::new(
            *origin + rect.origin,
            size(rect.size.width.max(px(1.)), rect.size.height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.geometry.as_ref().map(|_| {
            self.value[..self.nearest_index(point)]
                .encode_utf16()
                .count()
        })
    }
}

fn single_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

pub fn utf8_range(text: &str, range: Range<usize>) -> Range<usize> {
    fn offset(text: &str, utf16: usize) -> usize {
        let mut units = 0;
        for (index, character) in text.char_indices() {
            if units >= utf16 {
                return index;
            }
            units += character.len_utf16();
        }
        text.len()
    }
    let start = offset(text, range.start);
    start..offset(text, range.end).max(start)
}

pub fn utf16_range(text: &str, range: Range<usize>) -> Range<usize> {
    text[..range.start].encode_utf16().count()..text[..range.end].encode_utf16().count()
}

fn previous_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .graphemes(true)
        .next()
        .map_or(cursor, |grapheme| cursor + grapheme.len())
}
