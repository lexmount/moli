use std::collections::HashSet;

use crate::{
    custom_elements,
    document_runtime::DomHandle,
    dom::native::{Element, Node},
    style_engine::{ComputedDisplayKind, ComputedTextWrapModeKind, ComputedWhiteSpaceCollapseKind},
    util::v8_string,
};

use super::super::node::set_text_content_in_reaction_scope;
use super::super::{
    JsContextHost, document::set_detached_text_replacement_value,
    node::node_runtime_and_handle_from_object_or_detached, throw_dom_exception,
};
use super::{
    html_element_getter_receiver, html_element_setter_receiver, observable_sources_with_fragments,
    property_string_value,
    rendered_state::{
        ElementBoxState, ElementContentVisibility, ElementRenderedState, ElementRenderedStyle,
        rendered_child_participates_in_flat_tree,
    },
    styles::StyleObservation,
    trusted_types::{TrustedHtmlSink, trusted_html_sink_string},
};

fn get_html_options(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    value: v8::Local<'_, v8::Value>,
) -> (bool, Vec<DomHandle>) {
    if value.is_null_or_undefined() {
        return (false, Vec::new());
    }
    let Ok(options) = v8::Local::<v8::Object>::try_from(value) else {
        return (false, Vec::new());
    };
    let serializable_shadow_roots = options
        .get(
            scope,
            crate::util::v8str(scope, "serializableShadowRoots").into(),
        )
        .is_some_and(|value| value.boolean_value(scope));
    let Some(shadow_roots_value) =
        options.get(scope, crate::util::v8str(scope, "shadowRoots").into())
    else {
        return (serializable_shadow_roots, Vec::new());
    };
    let Ok(shadow_roots) = v8::Local::<v8::Object>::try_from(shadow_roots_value) else {
        return (serializable_shadow_roots, Vec::new());
    };
    let length = shadow_roots
        .get(scope, crate::util::v8str(scope, "length").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    let mut handles = Vec::new();
    for index in 0..length {
        let Some(value) = shadow_roots.get_index(scope, index) else {
            continue;
        };
        let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        let Ok((candidate_runtime_ptr, handle)) =
            node_runtime_and_handle_from_object_or_detached(scope, object)
        else {
            continue;
        };
        if candidate_runtime_ptr == runtime_ptr
            && unsafe { &*runtime_ptr }.dom_host().is_shadow_root(handle)
        {
            handles.push(handle);
        }
    }
    (serializable_shadow_roots, handles)
}

pub(in crate::native_bridge) fn node_direct_text_content(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    let dom = runtime.dom_host().dom();
    dom.node(handle).map(|node| node.direct_text_content(dom))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InnerTextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

impl InnerTextTransform {
    fn append_to(self, text: &str, white_space: InnerTextWhiteSpace, writer: &mut InnerTextWriter) {
        match self {
            Self::None => writer.push_text(text, white_space),
            Self::Uppercase => {
                for ch in text.chars() {
                    for transformed in ch.to_uppercase() {
                        writer.push_char(transformed, white_space);
                    }
                }
            }
            Self::Lowercase => {
                for ch in text.chars() {
                    for transformed in ch.to_lowercase() {
                        writer.push_char(transformed, white_space);
                    }
                }
            }
            Self::Capitalize => {
                let mut at_word_start = true;
                for ch in text.chars() {
                    if ch.is_whitespace() {
                        at_word_start = true;
                        writer.push_char(ch, white_space);
                    } else if at_word_start {
                        for transformed in ch.to_uppercase() {
                            writer.push_char(transformed, white_space);
                        }
                        at_word_start = false;
                    } else {
                        writer.push_char(ch, white_space);
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InnerTextWhiteSpace {
    Collapse,
    Preserve,
    PreserveBreaks,
}

#[derive(Default)]
struct InnerTextWriter {
    output: String,
    pending_space: bool,
    segment_has_text: bool,
    inline_context_has_content: bool,
    pending_required_line_breaks: u8,
    pending_table_cell_boundary: bool,
    pending_table_row_boundary: bool,
    suppress_lf_after_cr: bool,
}

impl InnerTextWriter {
    fn push_text(&mut self, text: &str, white_space: InnerTextWhiteSpace) {
        match white_space {
            InnerTextWhiteSpace::Collapse => self.push_collapsed_text(text),
            InnerTextWhiteSpace::Preserve => {
                for ch in text.chars() {
                    self.push_preserved_char(ch);
                }
            }
            InnerTextWhiteSpace::PreserveBreaks => {
                for ch in text.chars() {
                    self.push_preserve_breaks_char(ch);
                }
            }
        }
    }

    fn push_collapsed_text(&mut self, text: &str) {
        let mut chunk_start = 0;
        for (index, byte) in text.bytes().enumerate() {
            if !byte.is_ascii_whitespace() {
                continue;
            }
            self.push_non_whitespace(&text[chunk_start..index]);
            self.pending_space = true;
            chunk_start = index + 1;
        }
        self.push_non_whitespace(&text[chunk_start..]);
    }

    fn push_non_whitespace(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.pending_table_row_boundary = false;
        self.flush_required_line_breaks();
        if self.pending_space && self.segment_has_text {
            self.output.push(' ');
        }
        self.pending_space = false;
        self.output.push_str(text);
        self.segment_has_text = true;
        self.inline_context_has_content = true;
    }

    fn push_char(&mut self, ch: char, white_space: InnerTextWhiteSpace) {
        match white_space {
            InnerTextWhiteSpace::Collapse => self.push_collapsed_char(ch),
            InnerTextWhiteSpace::Preserve => self.push_preserved_char(ch),
            InnerTextWhiteSpace::PreserveBreaks => self.push_preserve_breaks_char(ch),
        }
    }

    fn push_collapsed_char(&mut self, ch: char) {
        if ch.is_ascii_whitespace() {
            self.pending_space = true;
            return;
        }
        self.pending_table_row_boundary = false;
        self.flush_required_line_breaks();
        if self.pending_space && self.segment_has_text {
            self.output.push(' ');
        }
        self.pending_space = false;
        self.output.push(ch);
        self.segment_has_text = true;
        self.inline_context_has_content = true;
    }

    fn push_preserved_char(&mut self, ch: char) {
        if self.suppress_lf_after_cr {
            self.suppress_lf_after_cr = false;
            if ch == '\n' {
                return;
            }
        }
        if ch == '\r' {
            self.suppress_lf_after_cr = true;
            self.push_preserved_line_break();
            return;
        }
        if ch == '\n' {
            self.push_preserved_line_break();
            return;
        }
        self.pending_table_row_boundary = false;
        self.flush_required_line_breaks();
        if self.pending_space && self.segment_has_text {
            self.output.push(' ');
        }
        self.pending_space = false;
        self.output.push(ch);
        self.segment_has_text = true;
        self.inline_context_has_content = true;
    }

    fn push_preserve_breaks_char(&mut self, ch: char) {
        if self.suppress_lf_after_cr {
            self.suppress_lf_after_cr = false;
            if ch == '\n' {
                return;
            }
        }
        if ch == '\r' {
            self.suppress_lf_after_cr = true;
            self.push_preserved_line_break();
        } else if ch == '\n' {
            self.push_preserved_line_break();
        } else {
            self.push_collapsed_char(ch);
        }
    }

    fn push_preserved_line_break(&mut self) {
        self.pending_table_row_boundary = false;
        self.flush_required_line_breaks();
        self.pending_space = false;
        self.output.push('\n');
        self.segment_has_text = false;
        self.inline_context_has_content = false;
    }

    fn push_break(&mut self) {
        self.pending_table_row_boundary = false;
        self.flush_required_line_breaks();
        self.pending_space = false;
        self.output.push('\n');
        self.segment_has_text = false;
        self.inline_context_has_content = false;
    }

    fn push_required_line_breaks(&mut self, count: u8) {
        debug_assert!((1..=2).contains(&count));
        self.pending_table_row_boundary = false;
        self.pending_space = false;
        if self.output.is_empty() {
            return;
        }
        self.pending_required_line_breaks = self.pending_required_line_breaks.max(count);
        self.inline_context_has_content = false;
    }

    fn flush_required_line_breaks(&mut self) {
        if self.pending_required_line_breaks == 0 {
            return;
        }
        self.pending_space = false;
        for _ in 0..self.pending_required_line_breaks {
            self.output.push('\n');
        }
        self.pending_required_line_breaks = 0;
        self.segment_has_text = false;
    }

    fn begin_table_row(&mut self) {
        self.pending_table_cell_boundary = false;
        if self.pending_table_row_boundary {
            self.flush_required_line_breaks();
            self.pending_space = false;
            self.output.push('\n');
            self.segment_has_text = false;
            self.inline_context_has_content = false;
            self.pending_table_row_boundary = false;
        }
    }

    fn begin_table_cell(&mut self) {
        if !self.pending_table_cell_boundary {
            return;
        }
        self.flush_required_line_breaks();
        self.pending_space = false;
        self.output.push('\t');
        self.segment_has_text = false;
        self.inline_context_has_content = false;
        self.pending_table_cell_boundary = false;
    }

    fn end_table_cell(&mut self) {
        self.pending_space = false;
        self.inline_context_has_content = false;
        self.pending_table_cell_boundary = true;
    }

    fn end_table_row(&mut self) {
        self.pending_table_cell_boundary = false;
        self.pending_space = false;
        self.inline_context_has_content = false;
        self.pending_table_row_boundary = true;
    }

    fn begin_table(&mut self) {
        self.pending_table_row_boundary = false;
        self.inline_context_has_content = false;
    }

    fn end_table(&mut self, required_line_breaks: u8) {
        self.pending_table_cell_boundary = false;
        self.pending_table_row_boundary = false;
        self.pending_space = false;
        self.inline_context_has_content = false;
        if required_line_breaks != 0 {
            self.push_required_line_breaks(required_line_breaks);
        }
    }

    fn begin_inline_atomic(&mut self) {
        self.pending_table_row_boundary = false;
        let has_intervening_text_box = self.pending_space && self.inline_context_has_content;
        if has_intervening_text_box {
            // A collapsible text box between two inline-level atomic boxes is
            // observable. If the previous atomic contains a block boundary,
            // that boundary precedes the intervening space in Chromium's
            // layout-derived text stream.
            self.flush_required_line_breaks();
            self.output.push(' ');
        }
        self.pending_space = false;
        self.segment_has_text = false;
        self.inline_context_has_content = false;
    }

    fn end_inline_atomic(&mut self) {
        self.pending_space = false;
        // Even an empty atomic inline box separates collapsible whitespace on
        // its two sides. No placeholder byte is emitted; this flag only keeps
        // the two whitespace runs independent.
        self.segment_has_text = true;
        self.inline_context_has_content = true;
    }

    fn finish(self) -> String {
        self.output
    }
}

fn computed_inner_text_white_space(
    collapse: ComputedWhiteSpaceCollapseKind,
    _wrap_mode: ComputedTextWrapModeKind,
) -> InnerTextWhiteSpace {
    // Soft wrapping needs line boxes and remains outside the lightweight
    // collector. Both wrap modes have the same observable output here; the
    // collapse axis still determines authored spaces and segment breaks.
    match collapse {
        ComputedWhiteSpaceCollapseKind::Preserve | ComputedWhiteSpaceCollapseKind::BreakSpaces => {
            InnerTextWhiteSpace::Preserve
        }
        ComputedWhiteSpaceCollapseKind::PreserveBreaks => InnerTextWhiteSpace::PreserveBreaks,
        ComputedWhiteSpaceCollapseKind::Collapse | ComputedWhiteSpaceCollapseKind::Other => {
            InnerTextWhiteSpace::Collapse
        }
    }
}

fn computed_inner_text_transform(
    value: crate::style_engine::ComputedTextTransformKind,
) -> Option<InnerTextTransform> {
    match value {
        crate::style_engine::ComputedTextTransformKind::None => Some(InnerTextTransform::None),
        crate::style_engine::ComputedTextTransformKind::Uppercase => {
            Some(InnerTextTransform::Uppercase)
        }
        crate::style_engine::ComputedTextTransformKind::Lowercase => {
            Some(InnerTextTransform::Lowercase)
        }
        crate::style_engine::ComputedTextTransformKind::Capitalize => {
            Some(InnerTextTransform::Capitalize)
        }
        crate::style_engine::ComputedTextTransformKind::Other => None,
    }
}

#[derive(Clone, Copy)]
struct PendingInnerTextNode {
    handle: DomHandle,
    include_root_text: bool,
    transform: InnerTextTransform,
    white_space: InnerTextWhiteSpace,
    visible: bool,
    text_participates: bool,
    parent_groups_table_cells: bool,
    prepared_style: Option<ElementRenderedStyle>,
}

#[derive(Clone, Copy)]
enum PendingInnerTextTask {
    Node(PendingInnerTextNode),
    RequiredLineBreaks(u8),
    EndTableCell,
    EndTableRow,
    EndTable(u8),
    EndInlineAtomic,
}

#[derive(Clone, Copy)]
enum SelectInnerTextTask {
    Visit {
        handle: DomHandle,
        inside_optgroup: bool,
    },
    EndOptGroup,
}

fn append_option_inner_text(
    runtime: &JsContextHost,
    handle: DomHandle,
    writer: &mut InnerTextWriter,
) {
    let Some(text) = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::as_element)
        .filter(|element| element.is_html_option())
        .map(|element| element.option_text(runtime.dom_host().dom(), handle))
    else {
        return;
    };
    writer.push_required_line_breaks(1);
    writer.push_text(&text, InnerTextWhiteSpace::Collapse);
    writer.push_required_line_breaks(1);
}

fn append_select_descendants(
    runtime: &JsContextHost,
    root: DomHandle,
    inside_optgroup: bool,
    writer: &mut InnerTextWriter,
) {
    let mut pending = Vec::new();
    pending.extend(
        runtime
            .dom_host()
            .child_handles_reversed(root)
            .map(|handle| SelectInnerTextTask::Visit {
                handle,
                inside_optgroup,
            }),
    );

    while let Some(task) = pending.pop() {
        let (handle, inside_optgroup) = match task {
            SelectInnerTextTask::EndOptGroup => {
                writer.push_required_line_breaks(1);
                continue;
            }
            SelectInnerTextTask::Visit {
                handle,
                inside_optgroup,
            } => (handle, inside_optgroup),
        };
        let Some(element) = runtime.dom_host().node(handle).and_then(Node::as_element) else {
            continue;
        };
        if element.is_html_option() {
            append_option_inner_text(runtime, handle, writer);
            continue;
        }
        if element.is_html_element("optgroup") {
            if inside_optgroup {
                continue;
            }
            writer.push_required_line_breaks(1);
            pending.push(SelectInnerTextTask::EndOptGroup);
            pending.extend(
                runtime
                    .dom_host()
                    .child_handles_reversed(handle)
                    .map(|handle| SelectInnerTextTask::Visit {
                        handle,
                        inside_optgroup: true,
                    }),
            );
            continue;
        }
        pending.extend(
            runtime
                .dom_host()
                .child_handles_reversed(handle)
                .map(|handle| SelectInnerTextTask::Visit {
                    handle,
                    inside_optgroup,
                }),
        );
    }
}

fn append_select_inner_text(
    runtime: &JsContextHost,
    select: DomHandle,
    writer: &mut InnerTextWriter,
) {
    append_select_descendants(runtime, select, false, writer);
}

fn element_has_intrinsic_atomic_inline_box(element: &Element) -> bool {
    element.is_html_element("button")
        || element.is_html_element("input")
        || element.is_html_element("textarea")
        || element.is_html_element("select")
        || element.is_html_element("iframe")
        || element.is_html_element("audio")
        || element.is_html_element("video")
        || element.is_html_element("canvas")
        || element.is_html_element("object")
        || element.is_html_element("img")
        || element.is_html_element("embed")
}

fn rendered_element_ignores_dom_children(element: &Element) -> bool {
    element.is_html_element("input")
        || element.is_html_element("textarea")
        || element.is_html_element("iframe")
        || element.is_html_element("audio")
        || element.is_html_element("video")
        || element.is_html_element("canvas")
        || element.is_html_element("object")
        || element.is_html_element("img")
        || element.is_html_element("embed")
        // SVG stop elements do not establish a rendered text subtree. This
        // mirrors their no-layout-object behavior without retaining layout
        // state in Moli's DOM or innerText traversal.
        || (element.namespace() == "http://www.w3.org/2000/svg"
            && element.local_name() == "stop")
}

fn append_inner_text(
    runtime: &JsContextHost,
    style_scope: &mut StyleObservation<'_>,
    root: DomHandle,
    root_style: Option<ElementRenderedStyle>,
    rendered_text_sources: &HashSet<DomHandle>,
    writer: &mut InnerTextWriter,
) {
    let mut pending = vec![PendingInnerTextTask::Node(PendingInnerTextNode {
        handle: root,
        include_root_text: true,
        transform: InnerTextTransform::None,
        white_space: InnerTextWhiteSpace::Collapse,
        visible: true,
        text_participates: true,
        parent_groups_table_cells: false,
        prepared_style: root_style,
    })];

    while let Some(pending_task) = pending.pop() {
        let task = match pending_task {
            PendingInnerTextTask::RequiredLineBreaks(count) => {
                writer.push_required_line_breaks(count);
                continue;
            }
            PendingInnerTextTask::EndTableCell => {
                writer.end_table_cell();
                continue;
            }
            PendingInnerTextTask::EndTableRow => {
                writer.end_table_row();
                continue;
            }
            PendingInnerTextTask::EndTable(required_line_breaks) => {
                writer.end_table(required_line_breaks);
                continue;
            }
            PendingInnerTextTask::EndInlineAtomic => {
                writer.end_inline_atomic();
                continue;
            }
            PendingInnerTextTask::Node(task) => task,
        };
        let Some(node) = runtime.dom_host().node(task.handle) else {
            continue;
        };

        if let Some(text) = node.as_text() {
            let has_rendered_text = rendered_text_sources.contains(&task.handle)
                || text.data().chars().all(char::is_whitespace);
            if task.visible && task.text_participates && has_rendered_text {
                task.transform
                    .append_to(text.data(), task.white_space, writer);
            }
            continue;
        }

        let Some(element) = node.as_element() else {
            continue;
        };
        let style = task
            .prepared_style
            .unwrap_or_else(|| ElementRenderedStyle::read_in_scope(style_scope, task.handle));
        if style.display == ComputedDisplayKind::None
            || (style.content_visibility == ElementContentVisibility::Hidden
                && style.content_visibility_applicable)
        {
            continue;
        }

        // These containers do not expose normal rendered children. A direct
        // read of a non-rendered root has already taken the textContent
        // fallback in node_inner_text(). Script and style are intentionally
        // absent: author CSS can make either one rendered.
        if !task.include_root_text
            && (element.is_html_element("head")
                || element.is_html_element("noscript")
                || element.is_html_element("template"))
        {
            continue;
        }
        let visible = style.visibility_visible;
        if !task.include_root_text && element.is_html_element("br") {
            if visible {
                writer.push_break();
            }
            continue;
        }

        let transform =
            computed_inner_text_transform(style.text_transform).unwrap_or(task.transform);
        let white_space =
            computed_inner_text_white_space(style.white_space_collapse, style.text_wrap_mode);
        let inline_atomic = visible
            && (matches!(
                style.display,
                ComputedDisplayKind::InlineAtomic | ComputedDisplayKind::InlineTable
            ) || (style.display == ComputedDisplayKind::Inline
                && element_has_intrinsic_atomic_inline_box(element)));
        if visible && element.is_html_select() {
            if inline_atomic {
                writer.begin_inline_atomic();
            }
            append_select_inner_text(runtime, task.handle, writer);
            if inline_atomic {
                writer.end_inline_atomic();
            }
            continue;
        }
        if visible && element.is_html_option() {
            append_option_inner_text(runtime, task.handle, writer);
            continue;
        }

        if inline_atomic {
            writer.begin_inline_atomic();
            pending.push(PendingInnerTextTask::EndInlineAtomic);
        }
        let boundary = if visible && style.display == ComputedDisplayKind::Table {
            Some(PendingInnerTextTask::EndTable(1))
        } else if visible && style.display == ComputedDisplayKind::InlineTable {
            Some(PendingInnerTextTask::EndTable(0))
        } else if style.display == ComputedDisplayKind::TableRow {
            writer.begin_table_row();
            visible.then_some(PendingInnerTextTask::EndTableRow)
        } else if task.parent_groups_table_cells && style.display == ComputedDisplayKind::TableCell
        {
            writer.begin_table_cell();
            visible.then_some(PendingInnerTextTask::EndTableCell)
        } else if visible
            && style.display != ComputedDisplayKind::Contents
            && element.is_html_element("p")
        {
            Some(PendingInnerTextTask::RequiredLineBreaks(2))
        } else if visible
            && matches!(
                style.display,
                ComputedDisplayKind::Block | ComputedDisplayKind::ListItem
            )
        {
            Some(PendingInnerTextTask::RequiredLineBreaks(1))
        } else {
            None
        };
        match boundary {
            Some(PendingInnerTextTask::EndTable(required_line_breaks)) => {
                writer.begin_table();
                if required_line_breaks != 0 {
                    writer.push_required_line_breaks(required_line_breaks);
                }
                pending.push(PendingInnerTextTask::EndTable(required_line_breaks));
            }
            Some(PendingInnerTextTask::EndTableRow) => {
                pending.push(PendingInnerTextTask::EndTableRow);
            }
            Some(PendingInnerTextTask::EndTableCell) => {
                pending.push(PendingInnerTextTask::EndTableCell);
            }
            Some(PendingInnerTextTask::RequiredLineBreaks(count)) => {
                writer.push_required_line_breaks(count);
                pending.push(PendingInnerTextTask::RequiredLineBreaks(count));
            }
            Some(PendingInnerTextTask::Node(_))
            | Some(PendingInnerTextTask::EndInlineAtomic)
            | None => {}
        }
        let parent_groups_table_cells = matches!(
            style.display,
            ComputedDisplayKind::Table
                | ComputedDisplayKind::InlineTable
                | ComputedDisplayKind::TableInternal
                | ComputedDisplayKind::TableRow
        );
        let text_participates = !matches!(
            style.display,
            ComputedDisplayKind::Table
                | ComputedDisplayKind::InlineTable
                | ComputedDisplayKind::TableInternal
                | ComputedDisplayKind::TableRow
        );
        if !rendered_element_ignores_dom_children(element) {
            pending.extend(
                runtime
                    .dom_host()
                    .child_handles_reversed(task.handle)
                    .filter(|child| {
                        rendered_child_participates_in_flat_tree(style_scope, task.handle, *child)
                    })
                    .map(|child| {
                        PendingInnerTextTask::Node(PendingInnerTextNode {
                            handle: child,
                            include_root_text: false,
                            transform,
                            white_space,
                            visible,
                            text_participates,
                            parent_groups_table_cells,
                            prepared_style: None,
                        })
                    }),
            );
        }
    }
}

fn inner_text_source_handles(runtime: &JsContextHost, root: DomHandle) -> Vec<DomHandle> {
    let mut sources = Vec::new();
    let mut pending = vec![root];
    while let Some(handle) = pending.pop() {
        if runtime.dom_host().node(handle).is_some_and(Node::is_text) {
            sources.push(handle);
        }
        pending.extend(runtime.dom_host().child_handles_reversed(handle));
    }
    sources
}

fn node_inner_text(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Result<String, moli_layout::LayoutError> {
    let Some(node) = runtime.dom_host().node(handle) else {
        return Ok(String::new());
    };
    if !runtime.dom_host().is_connected(handle) {
        return Ok(node.text_content(runtime.dom_host().dom()));
    }
    let sources = inner_text_source_handles(runtime, handle);
    let rendered_text_sources = if let Some(document) = runtime.layout_document_for_source(handle) {
        observable_sources_with_fragments(
            runtime,
            document,
            &sources,
            moli_layout::LayoutFlushReason::SynchronousGeometry,
        )?
    } else {
        HashSet::new()
    };
    let mut style_scope = StyleObservation::new(runtime);
    let rendered_state = ElementRenderedState::read_in_scope(&mut style_scope, handle);
    if rendered_state.has_content_visibility_lock() {
        return Ok(String::new());
    }
    if rendered_state.box_state() == ElementBoxState::NoBox {
        return Ok(node.text_content(runtime.dom_host().dom()));
    }

    let mut writer = InnerTextWriter::default();
    append_inner_text(
        runtime,
        &mut style_scope,
        handle,
        rendered_state.target_style().copied(),
        &rendered_text_sources,
        &mut writer,
    );
    Ok(writer.finish())
}

fn node_inner_html(runtime: &JsContextHost, handle: DomHandle) -> Option<String> {
    let scripting_enabled_for_node = |node| runtime.node_document_scripting_enabled(node);
    runtime
        .dom_host()
        .get_html(handle, &scripting_enabled_for_node, false, &[])
}

fn is_element_or_shadow_root_receiver(runtime: &JsContextHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .is_some_and(Node::is_element)
        || runtime.dom_host().is_shadow_root(handle)
}

pub(in crate::native_bridge) fn node_inner_html_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        crate::webidl::throw_type_error(scope, "innerHTML getter called on incompatible receiver.");
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !is_element_or_shadow_root_receiver(runtime, handle) {
        crate::webidl::throw_type_error(scope, "innerHTML getter called on incompatible receiver.");
        rv.set_undefined();
        return;
    }
    let Some(value) = node_inner_html(runtime, handle) else {
        rv.set_undefined();
        return;
    };
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn node_outer_html_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        crate::webidl::throw_type_error(scope, "outerHTML getter called on incompatible receiver.");
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !runtime
        .dom_host()
        .node(handle)
        .is_some_and(Node::is_element)
    {
        crate::webidl::throw_type_error(scope, "outerHTML getter called on incompatible receiver.");
        rv.set_undefined();
        return;
    }
    let scripting_enabled_for_node = |node| runtime.node_document_scripting_enabled(node);
    let value = runtime
        .dom_host()
        .outer_html_with_shadow_roots(
            handle,
            &scripting_enabled_for_node,
            crate::dom::native::ShadowRootInclusion::None,
            None,
        )
        .unwrap_or_default();
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn node_outer_html_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        crate::webidl::throw_type_error(scope, "outerHTML setter called on incompatible receiver.");
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !runtime
        .dom_host()
        .node(handle)
        .is_some_and(Node::is_element)
    {
        crate::webidl::throw_type_error(scope, "outerHTML setter called on incompatible receiver.");
        return;
    }
    let value = args.get(0);
    let Some(html) =
        trusted_html_sink_string(scope, runtime_ptr, value, TrustedHtmlSink::ElementOuterHtml)
    else {
        return;
    };
    if runtime
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)
        .and_then(|parent| runtime.dom_host().node(parent))
        .is_some_and(Node::is_document)
    {
        throw_dom_exception(
            scope,
            "NoModificationAllowedError",
            7,
            "Cannot set outerHTML when the parent is a document.",
        );
        return;
    }
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let runtime = unsafe { &mut *runtime_ptr };
        let _ = runtime.set_outer_html(scope, runtime_ptr, handle, &html);
    });
}

pub(in crate::native_bridge) fn node_inner_html_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        crate::webidl::throw_type_error(scope, "innerHTML setter called on incompatible receiver.");
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !is_element_or_shadow_root_receiver(runtime, handle) {
        crate::webidl::throw_type_error(scope, "innerHTML setter called on incompatible receiver.");
        return;
    }
    let value = args.get(0);
    let sink = if runtime.dom_host().is_shadow_root(handle) {
        TrustedHtmlSink::ShadowRootInnerHtml
    } else {
        TrustedHtmlSink::ElementInnerHtml
    };
    let Some(html) = trusted_html_sink_string(scope, runtime_ptr, value, sink) else {
        return;
    };
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let runtime = unsafe { &mut *runtime_ptr };
        let _ = runtime.set_inner_html(scope, runtime_ptr, handle, &html);
    });
}

pub(in crate::native_bridge) fn node_set_html_unsafe_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        crate::webidl::throw_type_error(
            scope,
            "Failed to execute 'setHTMLUnsafe' on 'Element': Illegal invocation.",
        );
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !is_element_or_shadow_root_receiver(runtime, handle) {
        crate::webidl::throw_type_error(
            scope,
            "Failed to execute 'setHTMLUnsafe' on 'Element': Illegal invocation.",
        );
        return;
    }
    let sink = if runtime.dom_host().is_shadow_root(handle) {
        TrustedHtmlSink::ShadowRootSetHtmlUnsafe
    } else {
        TrustedHtmlSink::ElementSetHtmlUnsafe
    };
    let Some(html) = trusted_html_sink_string(scope, runtime_ptr, args.get(0), sink) else {
        return;
    };
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let runtime = unsafe { &mut *runtime_ptr };
        let _ = runtime.set_html_unsafe(scope, runtime_ptr, handle, &html);
    });
}

pub(in crate::native_bridge) fn node_get_html_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        crate::webidl::throw_type_error(
            scope,
            "Failed to execute 'getHTML' on 'Element': Illegal invocation.",
        );
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !is_element_or_shadow_root_receiver(runtime, handle) {
        crate::webidl::throw_type_error(
            scope,
            "Failed to execute 'getHTML' on 'Element': Illegal invocation.",
        );
        rv.set_undefined();
        return;
    }
    let (serializable_shadow_roots, explicit_shadow_roots) =
        get_html_options(scope, runtime_ptr, args.get(0));
    let should_serialize_registry_attribute =
        |_: DomHandle, shadow_root: DomHandle, _: &crate::dom::native::ShadowRootInit| {
            runtime.should_serialize_shadow_root_registry_attribute(shadow_root)
        };
    let scripting_enabled_for_node = |node| runtime.node_document_scripting_enabled(node);
    let Some(html) = runtime
        .dom_host()
        .get_html_with_shadow_root_registry_attribute_policy(
            handle,
            &scripting_enabled_for_node,
            serializable_shadow_roots,
            &explicit_shadow_roots,
            Some(&should_serialize_registry_attribute),
        )
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) = v8_string(scope, &html) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn node_inner_text_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        rv.set_null();
        return;
    };
    let value = match node_inner_text(unsafe { &*runtime_ptr }, handle) {
        Ok(value) => value,
        Err(error) => {
            let Some(message) = v8_string(
                scope,
                &format!("Layout failed while reading innerText: {error}"),
            ) else {
                rv.set_undefined();
                return;
            };
            let exception = v8::Exception::error(scope, message);
            scope.throw_exception(exception);
            rv.set_undefined();
            return;
        }
    };
    let Some(value) = v8_string(scope, &value) else {
        rv.set_null();
        return;
    };
    rv.set(value.into());
}

pub(in crate::native_bridge) fn node_inner_text_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(value) = property_string_value(scope, args.get(0)) else {
        return;
    };
    let _ = set_text_content_in_reaction_scope(scope, runtime_ptr, handle, &value);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn title_text_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_getter_receiver(scope, args.this(), "HTMLTitleElement", "text", "title")
    else {
        rv.set_empty_string();
        return;
    };
    let value = node_direct_text_content(unsafe { &*runtime_ptr }, handle).unwrap_or_default();
    if let Some(value) = v8_string(scope, &value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

pub(in crate::native_bridge) fn title_text_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) =
        html_element_setter_receiver(scope, args.this(), "HTMLTitleElement", "text", "title")
    else {
        rv.set_undefined();
        return;
    };
    let Some(value) = property_string_value(scope, args.get(0)) else {
        return;
    };
    if set_detached_text_replacement_value(scope, args.this(), &value).is_some() {
        rv.set_undefined();
        return;
    }
    let _ = set_text_content_in_reaction_scope(scope, runtime_ptr, handle, &value);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn node_outer_text_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s, v8::Value>,
) {
    node_inner_text_getter_function(scope, args, rv);
}

pub(in crate::native_bridge) fn node_outer_text_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        return;
    };
    let Some(value) = property_string_value(scope, args.get(0)) else {
        return;
    };
    let Some(parent) = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)
    else {
        throw_dom_exception(
            scope,
            "NoModificationAllowedError",
            7,
            "Cannot set outerText on a detached element.",
        );
        return;
    };
    custom_elements::with_custom_element_reaction_scope(scope, runtime_ptr, |scope| {
        let runtime = unsafe { &mut *runtime_ptr };
        if value.is_empty() {
            let _ = runtime.remove_child_appending_to_current_reaction_queue(
                scope,
                runtime_ptr,
                parent,
                handle,
            );
            return;
        }
        let text = match runtime.dom_host().owner_document_handle(parent) {
            Some(document_handle) => runtime.create_text_node_for_document(document_handle, &value),
            None => runtime.create_text_node(&value),
        };
        let _ = runtime.replace_child_appending_to_current_reaction_queue(
            scope,
            runtime_ptr,
            parent,
            text,
            handle,
        );
    });
    rv.set_undefined();
}

#[cfg(test)]
mod tests {
    use super::{InnerTextTransform, InnerTextWhiteSpace, InnerTextWriter};

    #[test]
    fn inner_text_writer_collapses_segments_without_treating_private_use_text_as_a_break() {
        let mut writer = InnerTextWriter::default();
        InnerTextTransform::None.append_to(" \talpha ", InnerTextWhiteSpace::Collapse, &mut writer);
        InnerTextTransform::Uppercase.append_to(
            " betaß\n",
            InnerTextWhiteSpace::Collapse,
            &mut writer,
        );
        writer.push_break();
        InnerTextTransform::None.append_to("  \r\n", InnerTextWhiteSpace::Collapse, &mut writer);
        writer.push_break();
        InnerTextTransform::None.append_to(
            " \u{E000} end ",
            InnerTextWhiteSpace::Collapse,
            &mut writer,
        );

        assert_eq!(writer.finish(), "alpha BETASS\n\n\u{E000} end");
    }

    #[test]
    fn inner_text_writer_capitalizes_words_while_collapsing_ascii_whitespace() {
        let mut writer = InnerTextWriter::default();
        InnerTextTransform::Capitalize.append_to(
            " hello\tworld\nnext ",
            InnerTextWhiteSpace::Collapse,
            &mut writer,
        );

        assert_eq!(writer.finish(), "Hello World Next");
    }

    #[test]
    fn inner_text_writer_merges_required_breaks_and_discards_edge_breaks() {
        let mut writer = InnerTextWriter::default();
        writer.push_required_line_breaks(2);
        writer.push_text("a", InnerTextWhiteSpace::Collapse);
        writer.push_required_line_breaks(1);
        writer.push_required_line_breaks(2);
        writer.push_text("b", InnerTextWhiteSpace::Collapse);
        writer.push_required_line_breaks(2);

        assert_eq!(writer.finish(), "a\n\nb");
    }

    #[test]
    fn inner_text_writer_preserves_authored_spaces_and_segment_breaks_by_mode() {
        let mut preserve = InnerTextWriter::default();
        preserve.push_text("  a \t b\r\n c  ", InnerTextWhiteSpace::Preserve);
        assert_eq!(preserve.finish(), "  a \t b\n c  ");

        let mut preserve_breaks = InnerTextWriter::default();
        preserve_breaks.push_text("  a \t b\r\n c  ", InnerTextWhiteSpace::PreserveBreaks);
        assert_eq!(preserve_breaks.finish(), "a b\nc");
    }
}
