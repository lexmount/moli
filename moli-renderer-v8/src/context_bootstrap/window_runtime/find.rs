use std::ops::Range;

use icu_segmenter::{WordSegmenter, options::WordBreakInvariantOptions};
use regex::RegexBuilder;

use crate::context_bootstrap::{
    selection_callbacks::set_selection_boundaries, selection_value_for_window,
    window_accessors::window_host_ptr,
};
use crate::document_runtime::DomHandle;
use crate::dom::native::NodeType;
use crate::dom_presentation::flat_children;
use crate::native_bridge::{
    DocumentSelectionSnapshot, OwnerDispatchScope, PageScriptHost, SelectionBoundarySnapshot,
    WindowOperationReceiver,
    element::{
        ElementRenderedStyle, StyleObservation, is_hidden_until_found,
        rendered_element_ignores_dom_children, reveal_target_ancestors,
    },
};
use crate::range_boundary::RangeBoundaryPoint;
use crate::style_engine::{ComputedDisplayKind, ComputedWhiteSpaceCollapseKind};
use crate::util::node_wrapper_from_handle;
use crate::webidl;

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Window.find")]
struct WindowFindArgs {
    #[webidl(default = "")]
    query: String,
    #[webidl(default = false)]
    case_sensitive: bool,
    #[webidl(default = false)]
    backwards: bool,
    #[webidl(default = false)]
    wrap: bool,
    #[webidl(default = false)]
    whole_word: bool,
}

pub(in crate::context_bootstrap) fn window_find_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = args.this();
    let receiver = window_host_ptr(scope, target)
        .zip(target.get_creation_context(scope))
        .map(|(host_ptr, context)| {
            let scope = &mut v8::ContextScope::new(scope, context);
            (
                host_ptr,
                WindowOperationReceiver::capture_after_access_check(scope, target, unsafe {
                    &*host_ptr
                }),
            )
        });
    let Some(parsed) = webidl::parse_args::<WindowFindArgs>(scope, &args) else {
        return;
    };
    let found = receiver
        .and_then(|(host_ptr, receiver)| {
            let binding = receiver.resolve_live_binding(unsafe { &*host_ptr })?;
            let window = moli_webapi_declare::web_api_object_target(scope, target)?;
            let window = v8::Global::new(scope, window);
            binding.with_current_scope(scope, host_ptr, |scope, owner| {
                let window = v8::Local::new(scope, &window);
                find_in_document(scope, host_ptr, owner, window, &parsed)
            })?
        })
        .is_some();
    rv.set(v8::Boolean::new(scope, found).into());
}

fn find_in_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut PageScriptHost,
    dispatch_scope: OwnerDispatchScope,
    window: v8::Local<'s, v8::Object>,
    args: &WindowFindArgs,
) -> Option<()> {
    if args.query.is_empty() {
        return None;
    }
    let runtime = unsafe { &*host_ptr };
    let owner = runtime.current_window_document_task_target_for_dispatch_scope(dispatch_scope)?;
    let document = runtime.input_document_for_owner(owner)?;
    let found = find_rendered_text(runtime, document, args)?;

    // Revealing content can run author callbacks before the Selection changes.
    reveal_target_ancestors(scope, host_ptr, found.start.container);
    let runtime = unsafe { &*host_ptr };
    let dom = runtime.dom_host();
    if runtime.input_document_for_owner(owner) != Some(document)
        || [found.start, found.end].into_iter().any(|point| {
            dom.owner_document_handle(point.container) != Some(document)
                || !dom.is_connected(point.container)
                || RangeBoundaryPoint::new(dom, point.container, point.offset).is_none()
        })
    {
        return None;
    }
    let document = node_wrapper_from_handle(scope, document)?;
    let start = node_wrapper_from_handle(scope, found.start.container)?;
    let end = node_wrapper_from_handle(scope, found.end.container)?;
    let selection = selection_value_for_window(scope, window)?;
    set_selection_boundaries(
        scope,
        selection,
        document,
        (start, found.start.offset),
        (end, found.end.offset),
    )
}

#[derive(Clone, Copy)]
struct TextPosition {
    node: DomHandle,
    offset: u32,
    order: usize,
}

impl TextPosition {
    fn boundary(self) -> SelectionBoundarySnapshot {
        SelectionBoundarySnapshot {
            container: self.node,
            offset: self.offset,
        }
    }

    fn order(self) -> (usize, u32) {
        (self.order, self.offset)
    }
}

struct TextSpan {
    bytes: Range<usize>,
    start: TextPosition,
    end: TextPosition,
}

#[derive(Default)]
struct FindBuffer {
    text: String,
    spans: Vec<TextSpan>,
    pending_space: Option<(TextPosition, TextPosition)>,
}

impl FindBuffer {
    fn push(&mut self, text: &str, start: TextPosition, end: TextPosition) {
        let from = self.text.len();
        self.text.push_str(text);
        self.spans.push(TextSpan {
            bytes: from..self.text.len(),
            start,
            end,
        });
    }

    fn append(
        &mut self,
        mut text: &str,
        node: DomHandle,
        order: usize,
        whitespace: ComputedWhiteSpaceCollapseKind,
    ) {
        let collapses = |ch| match whitespace {
            ComputedWhiteSpaceCollapseKind::Preserve
            | ComputedWhiteSpaceCollapseKind::BreakSpaces => false,
            ComputedWhiteSpaceCollapseKind::PreserveBreaks => matches!(ch, ' ' | '\t' | '\u{c}'),
            _ => matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{c}'),
        };
        let mut start = TextPosition {
            node,
            order,
            offset: 0,
        };
        while let Some(first) = text.chars().next() {
            let space = collapses(first);
            let limit = text
                .char_indices()
                .find_map(|(offset, ch)| (collapses(ch) != space).then_some(offset))
                .unwrap_or(text.len());
            let (run, rest) = text.split_at(limit);
            let end = TextPosition {
                offset: start.offset + run.encode_utf16().count() as u32,
                ..start
            };
            if space {
                if !self.text.is_empty() {
                    let beginning = self.pending_space.map_or(start, |(beginning, _)| beginning);
                    self.pending_space = Some((beginning, end));
                }
            } else {
                if let Some((from, to)) = self.pending_space.take() {
                    self.push(" ", from, to);
                }
                self.push(run, start, end);
            }
            text = rest;
            start = end;
        }
    }

    fn position(&self, byte: usize, end: bool) -> Option<TextPosition> {
        let index = self.spans.partition_point(|span| {
            if end {
                span.bytes.end < byte
            } else {
                span.bytes.end <= byte
            }
        });
        let span = self.spans.get(index)?;
        if byte == span.bytes.end {
            return Some(span.end);
        }
        Some(TextPosition {
            offset: span.start.offset
                + self.text[span.bytes.start..byte].encode_utf16().count() as u32,
            ..span.start
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WalkEdge {
    Enter,
    Leave,
}

fn selection_cursor(
    runtime: &PageScriptHost,
    document: DomHandle,
    backwards: bool,
) -> Option<(DomHandle, WalkEdge, u32)> {
    let selection = runtime
        .document_editing_selection(document)
        .or_else(|| runtime.document_selection_snapshot(document))?;
    let point = if backwards {
        selection.start
    } else {
        selection.end
    };
    let dom = runtime.dom_host();
    if dom.node(point.container)?.character_data_value().is_some() {
        return Some((point.container, WalkEdge::Enter, point.offset));
    }
    if let Some(child) = dom
        .child_handles(point.container)
        .nth(point.offset as usize)
    {
        return Some((child, WalkEdge::Enter, 0));
    }
    let container = dom
        .shadow_root_host(point.container)
        .unwrap_or(point.container);
    Some((container, WalkEdge::Leave, 0))
}

enum Walk {
    Enter(DomHandle, bool, ComputedWhiteSpaceCollapseKind),
    Leave(DomHandle, bool),
}

fn rendered_text(
    runtime: &PageScriptHost,
    document: DomHandle,
    marker: Option<(DomHandle, WalkEdge, u32)>,
) -> (Vec<FindBuffer>, Option<(usize, u32)>) {
    let dom = runtime.dom_host();
    let root = dom
        .active_modal_dialog_for_document(document)
        .unwrap_or(document);
    let mut style = StyleObservation::new(runtime);
    for parent in std::iter::successors(dom.flat_tree_parent(root), |parent| {
        dom.flat_tree_parent(*parent)
    }) {
        if dom.element(parent).is_some() && hides_text_subtree(&mut style, parent) {
            return (Vec::new(), None);
        }
    }

    let mut blocks = Vec::new();
    let mut buffer = FindBuffer::default();
    let mut cursor = None;
    let mut stack = vec![Walk::Enter(
        root,
        true,
        ComputedWhiteSpaceCollapseKind::Collapse,
    )];
    let mut order = 0;
    while let Some(visit) = stack.pop() {
        order += 1;
        let (node, edge) = match visit {
            Walk::Enter(node, ..) => (node, WalkEdge::Enter),
            Walk::Leave(node, _) => (node, WalkEdge::Leave),
        };
        if let Some((target, target_edge, offset)) = marker
            && (node, edge) == (target, target_edge)
        {
            cursor = Some((order, offset));
        }
        let Walk::Enter(_, mut visible, mut whitespace) = visit else {
            if matches!(visit, Walk::Leave(_, true)) && !buffer.text.is_empty() {
                blocks.push(std::mem::take(&mut buffer));
            }
            continue;
        };
        let Some(native) = dom.node(node) else {
            continue;
        };
        if matches!(native.node_type(), NodeType::Text | NodeType::CDataSection) {
            if visible && let Some(data) = native.character_data_value() {
                buffer.append(data.as_str_lossy(), node, order, whitespace);
            }
            continue;
        }

        let mut separate = false;
        if let Some(element) = native.as_element() {
            if dom.is_inert_in_document(node)
                || rendered_element_ignores_dom_children(element)
                || element.is_html_select()
                || element.is_html_element("script")
                || element.is_inline_style_element()
            {
                if !buffer.text.is_empty() {
                    blocks.push(std::mem::take(&mut buffer));
                }
                continue;
            }
            let computed = ElementRenderedStyle::read_in_scope(&mut style, node);
            if computed.display == ComputedDisplayKind::None
                || (computed.hides_contents && !is_hidden_until_found(runtime, node))
            {
                continue;
            }
            visible = computed.visibility_visible;
            whitespace = computed.white_space_collapse;
            separate = element.is_html_element("br")
                || !matches!(
                    computed.display,
                    ComputedDisplayKind::Inline | ComputedDisplayKind::Contents
                );
            if separate && !buffer.text.is_empty() {
                blocks.push(std::mem::take(&mut buffer));
            }
        }
        stack.push(Walk::Leave(node, separate));
        stack.extend(
            flat_children(dom, document, node)
                .into_iter()
                .rev()
                .map(|child| Walk::Enter(child, visible, whitespace)),
        );
    }
    if !buffer.text.is_empty() {
        blocks.push(buffer);
    }
    (blocks, cursor)
}

fn hides_text_subtree(style: &mut StyleObservation<'_>, node: DomHandle) -> bool {
    let computed = ElementRenderedStyle::read_in_scope(style, node);
    computed.display == ComputedDisplayKind::None
        || (computed.hides_contents && !is_hidden_until_found(style.runtime(), node))
}

fn find_rendered_text(
    runtime: &PageScriptHost,
    document: DomHandle,
    args: &WindowFindArgs,
) -> Option<DocumentSelectionSnapshot> {
    let pattern = RegexBuilder::new(&regex::escape(&args.query))
        .case_insensitive(!args.case_sensitive)
        .build()
        .ok()?;
    let (blocks, cursor) = rendered_text(
        runtime,
        document,
        selection_cursor(runtime, document, args.backwards),
    );
    let segmenter = args
        .whole_word
        .then(|| WordSegmenter::new_auto(WordBreakInvariantOptions::default()));
    let mut preferred = None;
    let mut wrapped = None;
    for block in blocks {
        let boundaries = segmenter
            .as_ref()
            .map(|segmenter| segmenter.segment_str(&block.text).collect::<Vec<_>>());
        let mut offset = 0;
        while let Some(found) = pattern.find_at(&block.text, offset) {
            // Advance from the beginning so backward search also sees overlaps.
            offset = found.start() + block.text[found.start()..].chars().next()?.len_utf8();
            if boundaries.as_ref().is_some_and(|positions| {
                positions.binary_search(&found.start()).is_err()
                    || positions.binary_search(&found.end()).is_err()
            }) {
                continue;
            }
            let start = block.position(found.start(), false)?;
            let end = block.position(found.end(), true)?;
            let selection = DocumentSelectionSnapshot {
                start: start.boundary(),
                end: end.boundary(),
            };
            if args.backwards {
                wrapped = Some(selection);
                if cursor.is_none_or(|cursor| end.order() <= cursor) {
                    preferred = Some(selection);
                }
            } else {
                wrapped.get_or_insert(selection);
                if cursor.is_none_or(|cursor| start.order() >= cursor) {
                    return Some(selection);
                }
            }
        }
    }
    preferred.or(wrapped.filter(|_| args.wrap))
}
