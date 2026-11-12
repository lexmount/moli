use super::PageScriptHost;
use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, NativeDom, Node, NodeData},
    parser::HtmlParser,
    style_engine::{
        ComputedDisplayKind, FullStyleWorldSnapshot, MoliStyleEngine,
        OwnedStyleSourceDocumentContext, StyleViewport, StyloStylesheetSource,
    },
};

pub(super) fn text_control_text_from_html(
    runtime: &PageScriptHost,
    target: DomHandle,
    markup: &str,
) -> String {
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
    let multiline = runtime
        .dom_host()
        .node(target)
        .and_then(Node::as_element)
        .is_some_and(|element| element.is_html_textarea());
    if multiline {
        textarea_fragment_text(parsed, root)
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

fn textarea_fragment_text(dom: NativeDom, root: DomHandle) -> String {
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
    let mut writer = TextareaFragmentText::default();
    let mut pending = host
        .child_handles_reversed(root)
        .map(FragmentTextTask::Node)
        .collect::<Vec<_>>();
    while let Some(task) = pending.pop() {
        let handle = match task {
            FragmentTextTask::Node(handle) => handle,
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
            writer.push_text(text.data());
            continue;
        }
        let Some(element) = node.as_element() else {
            continue;
        };
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
        // Like Blink's fragment TextIterator, retain original text, including
        // whitespace, and ignore visibility and text-transform.
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
        pending.extend(
            host.child_handles_reversed(handle)
                .map(FragmentTextTask::Node),
        );
    }
    writer.finish()
}

enum FragmentTextTask {
    Node(DomHandle),
    Break(u8),
    EndCell,
}

#[derive(Default)]
struct TextareaFragmentText {
    text: String,
    pending_breaks: u8,
    pending_tab: bool,
    ends_with_br: bool,
}

impl TextareaFragmentText {
    fn require_breaks(&mut self, count: u8) {
        self.pending_breaks = self.pending_breaks.max(count);
        self.pending_tab = false;
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
        }
        self.pending_breaks = 0;
        self.pending_tab = false;
    }

    fn push_text(&mut self, text: &str) {
        if !text.is_empty() {
            self.flush_boundaries();
            self.text.push_str(text);
            self.ends_with_br = false;
        }
    }

    fn push_br(&mut self) {
        self.flush_boundaries();
        self.text.push('\n');
        self.ends_with_br = true;
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
