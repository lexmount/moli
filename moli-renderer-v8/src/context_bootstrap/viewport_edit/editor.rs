use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    ops::Range,
};

use moli_layout::{LayoutQuery, LayoutQueryAnswer, LayoutRect};

use crate::{
    context_bootstrap::{range::new_range_for_document, selection::*, selection_value_for_window},
    document_runtime::DomHandle,
    dom::native::{DomHost, Element},
    native_bridge::{
        PageScriptHost, callback_value_dom_handle,
        element::{
            TextEditInputType, contenteditable_editing_host, dispatch_beforeinput, is_text_control,
            is_text_control_user_editable, replace_contenteditable_selection_after_beforeinput,
            replace_text_control_selection_after_beforeinput,
            text_control_set_selection_range_internal, text_control_value,
        },
    },
    runtime::RendererViewportEditor,
    util::node_wrapper_from_handle,
};

pub(super) struct Editor {
    target: DomHandle,
    pub(super) text: String,
    pub(super) selection: Range<usize>,
    reversed: bool,
    pub(super) password: bool,
    // Contenteditable text uses a single UTF-16 coordinate space across its nodes.
    map: Option<EditableTextMap>,
}

impl Editor {
    pub(super) fn revision(&self, host: &PageScriptHost) -> u64 {
        let mut hash = DefaultHasher::new();
        self.target.hash(&mut hash);
        self.selection.hash(&mut hash);
        self.reversed.hash(&mut hash);
        if self.password {
            host.dom_host().dom_version().hash(&mut hash);
        } else {
            self.text.hash(&mut hash);
        }
        hash.finish()
    }

    pub(super) fn into_snapshot(
        self,
        host: &PageScriptHost,
        revision: u64,
        marked: Option<Range<usize>>,
    ) -> RendererViewportEditor {
        let extent = self.text.encode_utf16().count();
        let start = self.selection.start.saturating_sub(1024).min(extent);
        let end = (start + 4096).min(extent);
        let text = if self.password {
            String::new()
        } else {
            String::from_utf16_lossy(
                &self
                    .text
                    .encode_utf16()
                    .skip(start)
                    .take(end - start)
                    .collect::<Vec<_>>(),
            )
        };
        let caret = if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        };
        let caret_rect = editor_rect(host, &self, caret..caret);
        let mark_rects = marked.as_ref().map_or_else(Vec::new, |range| {
            let length = range.len().min(128);
            (range.start..range.start + length)
                .filter_map(|index| {
                    editor_rect(host, &self, index..index + 1).map(|rect| (index, rect))
                })
                .collect()
        });
        RendererViewportEditor {
            target: self.target.index_u32(),
            revision,
            text,
            password: self.password,
            text_start: start,
            selection: self.selection,
            reversed: self.reversed,
            marked,
            caret_rect,
            mark_rects,
        }
    }

    fn offset(&self, offset: usize) -> (DomHandle, usize) {
        self.map.as_ref().map_or((self.target, offset), |map| {
            map.boundary(self.target, offset)
        })
    }
}

struct EditableTextMap {
    text: String,
    segments: Vec<(DomHandle, Range<usize>)>,
    nodes: HashMap<DomHandle, Range<usize>>,
}

impl EditableTextMap {
    fn read(dom: &DomHost, root: DomHandle) -> Self {
        let mut map = Self {
            text: String::new(),
            segments: Vec::new(),
            nodes: HashMap::new(),
        };
        let mut position = 0;
        let mut stack = vec![(root, false)];
        while let Some((handle, exiting)) = stack.pop() {
            if exiting {
                if let Some(range) = map.nodes.get_mut(&handle) {
                    range.end = position;
                }
            } else if let Some(text) = dom.node(handle).and_then(|node| node.as_text()) {
                let start = position;
                position += text.data().encode_utf16().count();
                map.segments.push((handle, start..position));
                map.nodes.insert(handle, start..position);
                map.text.push_str(text.data());
            } else {
                map.nodes.insert(handle, position..position);
                stack.push((handle, true));
                if let Some(children) = dom.child_nodes(handle) {
                    stack.extend(children.into_iter().rev().map(|child| (child, false)));
                }
            }
        }
        map
    }

    fn absolute(&self, dom: &DomHost, node: DomHandle, offset: usize) -> Option<usize> {
        let range = self.nodes.get(&node)?;
        if dom.node(node)?.as_text().is_some() {
            return Some(range.start + offset.min(range.len()));
        }
        Some(
            dom.child_nodes(node)
                .and_then(|children| {
                    self.nodes
                        .get(children.get(offset)?)
                        .map(|range| range.start)
                })
                .unwrap_or(range.end),
        )
    }

    fn boundary(&self, root: DomHandle, offset: usize) -> (DomHandle, usize) {
        self.segments
            .iter()
            .find(|(_, range)| offset <= range.end)
            .map_or((root, 0), |(handle, range)| {
                (*handle, offset.saturating_sub(range.start))
            })
    }
}

pub(super) fn active_target(host: &PageScriptHost) -> Option<DomHandle> {
    let active = host.active_element_handle()?;
    if is_text_control(host, active) {
        return is_text_control_user_editable(host, active).then_some(active);
    }
    contenteditable_editing_host(host, active)
}

pub(super) fn is_password_control(element: &Element) -> bool {
    element.is_html_input() && element.input_type() == moli_dom::forms::InputType::Password
}

pub(super) fn read_editor(
    scope: &mut v8::PinScope<'_, '_>,
    host: &PageScriptHost,
    target: DomHandle,
) -> Option<Editor> {
    let dom = host.dom_host();
    let element = dom.node(target)?.as_element()?;
    if is_text_control_user_editable(host, target) {
        return Some(Editor {
            target,
            text: text_control_value(host, target),
            selection: element.selection_start() as usize..element.selection_end() as usize,
            reversed: element.selection_direction() == "backward",
            password: is_password_control(element),
            map: None,
        });
    }
    let window = scope.get_current_context().global(scope);
    let selection = selection_value_for_window(scope, window)?;
    let map = EditableTextMap::read(dom, target);
    let anchor = selection_anchor_node(scope, selection)
        .and_then(|node| callback_value_dom_handle(scope, node.into()))
        .and_then(|node| {
            map.absolute(
                dom,
                node,
                selection_anchor_offset(scope, selection) as usize,
            )
        })
        .unwrap_or_default();
    let focus = selection_focus_node(scope, selection)
        .and_then(|node| callback_value_dom_handle(scope, node.into()))
        .and_then(|node| map.absolute(dom, node, selection_focus_offset(scope, selection) as usize))
        .unwrap_or(anchor);
    Some(Editor {
        target,
        text: map.text.clone(),
        selection: anchor.min(focus)..anchor.max(focus),
        reversed: anchor > focus,
        password: false,
        map: Some(map),
    })
}

pub(super) fn replace_range(
    scope: &mut v8::PinScope<'_, '_>,
    host: *mut PageScriptHost,
    editor: &Editor,
    revision: u64,
    range: &Range<usize>,
    text: &str,
    input_type: TextEditInputType,
) -> bool {
    let target = editor.target;
    if range.start > range.end
        || range.end > editor.text.encode_utf16().count()
        || active_target(unsafe { &*host }) != Some(target)
        || !dispatch_beforeinput(scope, host, target, input_type, input_type.data(text))
    {
        return false;
    }
    let live_host = unsafe { &*host };
    let Some(live) = read_editor(scope, live_host, target) else {
        return false;
    };
    if live.revision(live_host) != revision
        || active_target(live_host) != Some(target)
        || !set_range(scope, host, &live, range)
    {
        return false;
    }
    if live.map.is_some() {
        replace_contenteditable_selection_after_beforeinput(scope, host, target, text, input_type)
    } else {
        replace_text_control_selection_after_beforeinput(
            scope, host, target, text, input_type, None,
        )
    }
}

pub(super) fn set_range(
    scope: &mut v8::PinScope<'_, '_>,
    host: *mut PageScriptHost,
    editor: &Editor,
    range: &Range<usize>,
) -> bool {
    if editor.map.is_none() {
        text_control_set_selection_range_internal(
            scope,
            host,
            editor.target,
            range.start as u32,
            range.end as u32,
        );
        return true;
    }
    let window = scope.get_current_context().global(scope);
    let (start_node, start) = editor.offset(range.start);
    let (end_node, end) = editor.offset(range.end);
    let Some(selection) = selection_value_for_window(scope, window) else {
        return false;
    };
    let Some(document) = selection_owner_document(scope, selection) else {
        return false;
    };
    let Some(range) = new_range_for_document(scope, document) else {
        return false;
    };
    let Some(start_node) = node_wrapper_from_handle(scope, start_node) else {
        return false;
    };
    let Some(end_node) = node_wrapper_from_handle(scope, end_node) else {
        return false;
    };
    selection_store(
        scope,
        selection,
        range,
        start_node,
        start as u32,
        end_node,
        end as u32,
        "forward",
    );
    selection_dispatch_change(scope);
    true
}

pub(super) fn text_at(text: &str, range: &Range<usize>) -> String {
    String::from_utf16_lossy(
        &text
            .encode_utf16()
            .skip(range.start)
            .take(range.len())
            .collect::<Vec<_>>(),
    )
}

fn editor_rect(host: &PageScriptHost, editor: &Editor, range: Range<usize>) -> Option<[f32; 4]> {
    let (source, start) = editor.offset(range.start);
    let (end_node, end) = editor.offset(range.end);
    let document = host.layout_document_for_source(source)?;
    let query = LayoutQuery::TextRangeRects {
        source,
        utf16_range: start..if end_node == source { end } else { start },
    };
    let rect = match host
        .answer_layout_query_for_document(
            document,
            &query,
            crate::native_bridge::LayoutQueryMode::Published,
        )
        .ok()?
    {
        LayoutQueryAnswer::TextRangeRects(rects) => rects.first().map(|quad| quad.bounding_rect()),
        _ => None,
    }
    .or_else(|| {
        let LayoutQueryAnswer::ClientRects(rects) = host
            .answer_layout_query_for_document(
                document,
                &LayoutQuery::ClientRects {
                    source: editor.target,
                },
                crate::native_bridge::LayoutQueryMode::Published,
            )
            .ok()?
        else {
            return None;
        };
        let rect = rects.first()?.bounding_rect();
        Some(LayoutRect::new(
            rect.x + 4.,
            rect.y + 3.,
            0.,
            rect.height.max(14.) - 6.,
        ))
    })?;
    let frame_to_root = host.input_frame_transform(document, None)?.inverse()?;
    let rect = frame_to_root.map_rect(rect).bounding_rect();
    Some([rect.x, rect.y, rect.width.max(1.), rect.height.max(1.)])
}
