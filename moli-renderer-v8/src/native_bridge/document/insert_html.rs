use super::PageScriptHost;
use super::insert_text::{
    EditingInsertionTarget, editing_host_at_boundary, editing_insertion_target,
};
use crate::{
    context_bootstrap::{replace_editable_selection_fragment, replace_editable_selection_text},
    document_runtime::DomHandle,
    dom::native::{ContentEditableState, DomHost, NativeDom, Node, NodeData},
    native_bridge::{
        element::{
            ComputedStyleRead, TextEditInputType, construct_input_event, dispatch_public_event,
            rendered_text_fragment, replace_text_control_selection_for_insert_html,
        },
        node_owner_document_relevant_context,
    },
    parser::HtmlParser,
    style_engine::{
        ComputedDisplayKind, ComputedWhiteSpaceCollapseKind, FullStyleWorldSnapshot,
        MoliStyleEngine, OwnedStyleSourceDocumentContext, StyleViewport, StyloStylesheetSource,
    },
};

pub(super) fn insert_html_command_target(
    runtime: &PageScriptHost,
    document: DomHandle,
) -> Option<EditingInsertionTarget> {
    let target = editing_insertion_target(runtime, document)?;
    if let EditingInsertionTarget::TextControl(handle) = target {
        let element = runtime.dom_host().node(handle)?.as_element()?;
        if !element.is_html_textarea() && !element.input_type().supports_text_length_validation() {
            return None;
        }
    }
    Some(target)
}

pub(super) fn exec_command_insert_html(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    markup: &str,
) -> bool {
    let Some(target) = insert_html_command_target(unsafe { &*runtime_ptr }, document) else {
        return false;
    };
    let handle = match target {
        EditingInsertionTarget::TextControl(handle)
        | EditingInsertionTarget::EditingHost(handle) => handle,
    };
    let Some(context) = node_owner_document_relevant_context(scope, runtime_ptr, handle) else {
        return false;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    if let EditingInsertionTarget::TextControl(_) = target {
        let text = text_control_text_from_html(unsafe { &*runtime_ptr }, handle, markup);
        return replace_text_control_selection_for_insert_html(scope, runtime_ptr, handle, &text);
    }
    insert_html_into_editing_host(scope, runtime_ptr, document, handle, markup).is_some()
}

fn insert_html_into_editing_host(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    document: DomHandle,
    host: DomHandle,
    markup: &str,
) -> Option<()> {
    let runtime = unsafe { &mut *runtime_ptr };
    let dom = runtime.dom_host();
    let start = runtime
        .document_editing_selection(document)?
        .start
        .container;
    let context = if dom.node(start)?.is_element() {
        start
    } else {
        dom.parent_node(start)?
    };
    let is_editable = |runtime: &PageScriptHost, node| {
        runtime.document_is_fully_active(document)
            && editing_host_at_boundary(runtime, document, node) == Some(host)
    };
    if plaintext_insertion(runtime, document, host, context) {
        let (source, root) = parse_body_fragment(runtime, host, markup);
        let text = rendered_fragment_text(
            source,
            root,
            FragmentTextMode::Plaintext {
                scripting_enabled: runtime.document_scripting_enabled(document),
            },
        );
        let white_space = ComputedStyleRead::new(runtime, context)
            .rendered_style_facts()
            .map_or(ComputedWhiteSpaceCollapseKind::Collapse, |style| {
                style.white_space_collapse
            });
        let text = if preserves_spaces(white_space) {
            text
        } else {
            preserve_inserted_spaces(&text)
        };
        if text.contains('\n')
            && matches!(
                white_space,
                ComputedWhiteSpaceCollapseKind::Collapse | ComputedWhiteSpaceCollapseKind::Other
            )
        {
            let fragment = rendered_text_fragment(runtime, document, &text)?;
            replace_editable_selection_fragment(
                scope,
                runtime_ptr,
                document,
                fragment,
                None,
                is_editable,
            )?;
        } else {
            replace_editable_selection_text(scope, runtime_ptr, document, &text, is_editable)?;
        }
    } else {
        let fragment =
            runtime.build_editing_fragment(scope, runtime_ptr, document, context, markup)?;
        replace_editable_selection_fragment(
            scope,
            runtime_ptr,
            document,
            fragment,
            Some(host),
            is_editable,
        )?;
    }
    if let Some(event) = construct_input_event(scope, "input", TextEditInputType::None, None) {
        let _ = dispatch_public_event(scope, runtime_ptr, host, event);
    }
    Some(())
}

fn plaintext_insertion(
    runtime: &PageScriptHost,
    document: DomHandle,
    host: DomHandle,
    mut context: DomHandle,
) -> bool {
    let dom = runtime.dom_host();
    if runtime.document_design_mode_enabled(document)
        && dom
            .node(document)
            .and_then(Node::as_document)
            .is_some_and(|node| node.body_handle(dom.dom(), document) == Some(host))
    {
        return false;
    }
    while let Some(node) = dom.node(context) {
        match node
            .as_element()
            .map(|element| element.content_editable_state())
        {
            Some(ContentEditableState::PlaintextOnly) => return true,
            Some(ContentEditableState::True | ContentEditableState::False) => return false,
            _ => {}
        }
        let Some(parent) = node.parent_node_id().filter(|_| context != host) else {
            break;
        };
        context = parent;
    }
    false
}

fn preserves_spaces(white_space: ComputedWhiteSpaceCollapseKind) -> bool {
    matches!(
        white_space,
        ComputedWhiteSpaceCollapseKind::Preserve | ComputedWhiteSpaceCollapseKind::BreakSpaces
    )
}

fn preserve_inserted_spaces(text: &str) -> String {
    let mut value = String::with_capacity(text.len());
    let mut after_space = false;
    for (index, character) in text.char_indices() {
        let space_at_line_edge = index == 0
            || text.as_bytes().get(index - 1) == Some(&b'\n')
            || text
                .as_bytes()
                .get(index + 1)
                .is_none_or(|&byte| byte == b'\n');
        let character = if character == ' ' && (after_space || space_at_line_edge) {
            '\u{00a0}'
        } else {
            character
        };
        value.push(character);
        after_space = character == ' ';
    }
    value
}

fn parse_body_fragment(
    runtime: &PageScriptHost,
    target: DomHandle,
    markup: &str,
) -> (NativeDom, DomHandle) {
    let parsed =
        HtmlParser::with_scripting_enabled(runtime.node_document_scripting_enabled(target))
            .parse_fragment_without_declarative_shadow_roots(
                runtime.host_document().url().clone(),
                "http://www.w3.org/1999/xhtml",
                "body",
                markup.to_owned(),
            );
    let root = parsed
        .body_node_id()
        .or_else(|| parsed.document_element_node_id())
        .unwrap_or_else(|| parsed.document_node_id());
    (parsed, root)
}

fn text_control_text_from_html(
    runtime: &PageScriptHost,
    target: DomHandle,
    markup: &str,
) -> String {
    let (parsed, root) = parse_body_fragment(runtime, target, markup);
    let multiline = runtime
        .dom_host()
        .node(target)
        .and_then(Node::as_element)
        .is_some_and(|element| element.is_html_textarea());
    if multiline {
        rendered_fragment_text(parsed, root, FragmentTextMode::Textarea)
    } else {
        input_fragment_text(&parsed, root)
    }
}

fn input_fragment_text(dom: &NativeDom, root: DomHandle) -> String {
    // Blink's input path uses textContent with br-to-newline conversion. Hidden
    // elements and script/style source therefore remain part of the text.
    let mut text = String::new();
    let mut pending = dom.child_ids_reversed(root).collect::<Vec<_>>();
    while let Some(handle) = pending.pop() {
        let Some(node) = dom.node(handle) else {
            continue;
        };
        if node.is_html_element_named("br") {
            text.push('\n');
            continue;
        }
        match node.data() {
            NodeData::Text(value) => text.push_str(value.data()),
            NodeData::CDataSection(value) => text.push_str(value.data()),
            _ => pending.extend(dom.child_ids_reversed(handle)),
        }
    }
    text
}

#[derive(Clone, Copy)]
enum FragmentTextMode {
    Textarea,
    Plaintext { scripting_enabled: bool },
}

fn rendered_fragment_text(dom: NativeDom, root: DomHandle, mode: FragmentTextMode) -> String {
    // Resolve styles in an inert DOM: no resource loading, script execution,
    // custom-element reactions, or mutations visible in the caller's document.
    let host = DomHost::from_dom(dom);
    let document = host.document_handle();
    let context = OwnedStyleSourceDocumentContext::new(document);
    let url = host.dom().final_url().expect("parsed fragment URL");
    let engine = MoliStyleEngine::new();
    let inputs = FullStyleWorldSnapshot {
        document_stylesheet_sources: host
            .dom()
            .nodes()
            .iter()
            .filter(|node| node.is_connected() && node.is_html_element_named("style"))
            .map(|node| StyloStylesheetSource::new(node.text_content(host.dom()), url.clone()))
            .collect(),
        ..FullStyleWorldSnapshot::default()
    };
    let mut writer = RenderedFragmentText::default();
    let root_white_space = match mode {
        FragmentTextMode::Textarea => ComputedWhiteSpaceCollapseKind::Preserve,
        FragmentTextMode::Plaintext { .. } => ComputedWhiteSpaceCollapseKind::Collapse,
    };
    let mut pending = host
        .child_handles_reversed(root)
        .map(|handle| FragmentTextTask::Node(handle, root_white_space))
        .collect::<Vec<_>>();
    while let Some(task) = pending.pop() {
        let (handle, white_space) = match task {
            FragmentTextTask::Node(handle, white_space) => (handle, white_space),
            FragmentTextTask::Break(count) => {
                writer.require_breaks(count);
                continue;
            }
            FragmentTextTask::EndCell => {
                writer.pending_tab = true;
                continue;
            }
        };
        let Some(node) = host.node(handle) else {
            continue;
        };
        if let Some(text) = node.as_text() {
            writer.push_with_white_space(text.data(), white_space);
            continue;
        }
        let Some(element) = node.as_element() else {
            continue;
        };
        if matches!(
            mode,
            FragmentTextMode::Plaintext {
                scripting_enabled: true
            }
        ) && element.is_html_element("noscript")
        {
            continue;
        }
        let Some(style) = engine.computed_style_snapshot_after_style_update_with_document_context(
            &host,
            url,
            handle,
            &inputs,
            context.as_ref(),
            document,
            StyleViewport::default(),
        ) else {
            continue;
        };
        let style = style.rendered_style_facts();
        if style.display == ComputedDisplayKind::None || style.hides_contents {
            continue;
        }
        if element.is_html_element("br") {
            writer.push_br();
            continue;
        }
        // Retain the source text's characters and ignore visibility and
        // text-transform. Textareas additionally preserve source whitespace.
        let breaks = match style.display {
            ComputedDisplayKind::Block if element.is_html_element("p") => 2,
            ComputedDisplayKind::Block
            | ComputedDisplayKind::ListItem
            | ComputedDisplayKind::Table
            | ComputedDisplayKind::TableRow => 1,
            _ => 0,
        };
        if breaks != 0 {
            writer.require_breaks(breaks);
            pending.push(FragmentTextTask::Break(breaks));
        } else if style.display == ComputedDisplayKind::TableCell {
            pending.push(FragmentTextTask::EndCell);
        }
        pending.extend(host.child_handles_reversed(handle).map(|child| {
            FragmentTextTask::Node(
                child,
                match mode {
                    FragmentTextMode::Textarea => white_space,
                    FragmentTextMode::Plaintext { .. } => style.white_space_collapse,
                },
            )
        }));
    }
    writer.finish()
}

enum FragmentTextTask {
    Node(DomHandle, ComputedWhiteSpaceCollapseKind),
    Break(u8),
    EndCell,
}

#[derive(Default)]
struct RenderedFragmentText {
    text: String,
    pending_breaks: u8,
    pending_tab: bool,
    pending_space: bool,
    ends_with_br: bool,
}

impl RenderedFragmentText {
    fn require_breaks(&mut self, count: u8) {
        self.pending_breaks = self.pending_breaks.max(count);
        self.pending_tab = false;
        self.pending_space = false;
    }

    fn flush_boundaries(&mut self) {
        if !self.text.is_empty() {
            let trailing_breaks = self
                .text
                .chars()
                .rev()
                .take(self.pending_breaks as usize)
                .take_while(|ch| *ch == '\n')
                .count();
            for _ in trailing_breaks..self.pending_breaks as usize {
                self.text.push('\n');
            }
            if self.pending_tab {
                self.text.push('\t');
            }
            if self.pending_space && !self.text.ends_with(['\n', '\t', ' ']) {
                self.text.push(' ');
            }
        }
        self.pending_breaks = 0;
        self.pending_tab = false;
        self.pending_space = false;
    }

    fn push_text(&mut self, text: &str) {
        if !text.is_empty() {
            self.flush_boundaries();
            self.text.push_str(text);
            self.ends_with_br = false;
        }
    }

    fn push_br(&mut self) {
        self.pending_space = false;
        self.flush_boundaries();
        self.text.push('\n');
        self.ends_with_br = true;
    }

    fn push_with_white_space(&mut self, text: &str, white_space: ComputedWhiteSpaceCollapseKind) {
        if preserves_spaces(white_space) {
            self.push_text(text);
            return;
        }
        let mut start = 0;
        for (index, character) in text.char_indices() {
            if !matches!(character, ' ' | '\n' | '\r' | '\t' | '\x0c') {
                continue;
            }
            self.push_text(&text[start..index]);
            if character == '\n' && white_space == ComputedWhiteSpaceCollapseKind::PreserveBreaks {
                self.push_br();
                self.ends_with_br = false;
            } else {
                self.pending_space = true;
            }
            start = index + 1;
        }
        self.push_text(&text[start..]);
    }

    fn finish(mut self) -> String {
        // A final br in a rendered fragment is the last line's placeholder;
        // authored newlines in a text node remain part of the inserted value.
        if self.ends_with_br {
            self.text.pop();
        }
        self.text
    }
}
