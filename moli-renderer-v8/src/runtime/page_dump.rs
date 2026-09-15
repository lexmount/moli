use crate::dom::native::{NativeDom, NativeNodeId, NodeData};
use crate::runtime::page_surface::{
    RendererPageDumpFormat, RendererPageDumpOptions, RendererPageDumpStripOptions,
};
use moli_page_types::MAX_DOM_OUTPUT_TREE_DEPTH;

use super::page_vm::PageVm;

impl PageVm {
    pub(crate) fn render_page_dump(&mut self, options: RendererPageDumpOptions) -> String {
        let mut dom = self.vm().document_runtime.dom_host().dom().clone();

        if options.with_base {
            let href = self
                .vm()
                .document_runtime
                .document_url()
                .as_str()
                .to_owned();
            inject_base_href(&mut dom, &href);
        }
        if options.with_frames {
            self.inline_child_frame_markups_into_dump_dom(&mut dom);
        }
        apply_strip_options(&mut dom, options.strip);

        match options.format {
            RendererPageDumpFormat::Html => dom.serialize_document(),
            RendererPageDumpFormat::Markdown => render_markdown_document(&dom),
        }
    }

    fn inline_child_frame_markups_into_dump_dom(&mut self, dom: &mut NativeDom) {
        let child_frames = self
            .vm()
            .live_child_document_handles_in_snapshot_order()
            .into_iter()
            .map(|(frame_id, owner_node_id, _)| (frame_id, owner_node_id))
            .collect::<Vec<_>>();

        for (frame_id, owner_node_id) in child_frames {
            let Some(snapshot) = self
                .vm_mut()
                .child_browsing_context_document_snapshot_by_frame_id(&frame_id)
            else {
                continue;
            };
            let _ = dom.set_attribute(owner_node_id, "srcdoc", &snapshot.markup);
            let _ = dom.set_attribute(owner_node_id, "data-moli-frame-url", &snapshot.url);
        }
    }
}

fn inject_base_href(dom: &mut NativeDom, href: &str) {
    let head = ensure_head_node(dom);
    let existing_base = first_direct_html_child(dom, head, "base");

    if let Some(base_id) = existing_base {
        let _ = dom.set_attribute(base_id, "href", href);
        return;
    }

    let base_id = dom.create_element("base");
    let _ = dom.set_attribute(base_id, "href", href);
    let first_child = dom.first_child(head);
    let _ = dom.insert_before(head, base_id, first_child);
}

fn first_direct_html_child(
    dom: &NativeDom,
    parent: NativeNodeId,
    local_name: &str,
) -> Option<NativeNodeId> {
    dom.find_child(parent, |child_id| {
        dom.node(child_id)
            .and_then(|node| node.as_element())
            .is_some_and(|element| element.is_html_element(local_name))
    })
}

fn ensure_head_node(dom: &mut NativeDom) -> NativeNodeId {
    if let Some(head) = dom.head_node_id() {
        return head;
    }

    let html = dom.document_element_node_id().unwrap_or_else(|| {
        let html = dom.create_element("html");
        let _ = dom.append_child(dom.document_node_id(), html);
        html
    });

    let head = dom.create_element("head");
    let body = dom.body_node_id();
    let _ = dom.insert_before(html, head, body);
    head
}

fn apply_strip_options(dom: &mut NativeDom, strip: RendererPageDumpStripOptions) {
    if !strip.js && !strip.ui && !strip.css {
        return;
    }

    let mut node_ids = Vec::new();
    collect_node_ids(dom, dom.document_node_id(), &mut node_ids);

    for node_id in &node_ids {
        if strip.css {
            let _ = dom.remove_attribute(*node_id, "style");
        }
        if strip.js {
            for attribute_name in dom.get_attribute_names(*node_id).unwrap_or_default() {
                if attribute_name.starts_with("on") {
                    let _ = dom.remove_attribute(*node_id, &attribute_name);
                }
            }
        }
    }

    let mut remove = Vec::new();
    for node_id in node_ids {
        let Some(node) = dom.node(node_id) else {
            continue;
        };
        let Some(element) = node.as_element() else {
            continue;
        };
        let tag = element.local_name();
        let should_remove = (strip.js && matches!(tag, "script" | "noscript"))
            || (strip.css
                && (tag == "style"
                    || (tag == "link"
                        && dom
                            .get_attribute(node_id, "rel")
                            .unwrap_or_default()
                            .split_ascii_whitespace()
                            .any(|token| token.eq_ignore_ascii_case("stylesheet")))))
            || (strip.ui
                && matches!(
                    tag,
                    "header"
                        | "footer"
                        | "nav"
                        | "aside"
                        | "form"
                        | "input"
                        | "button"
                        | "select"
                        | "textarea"
                        | "option"
                        | "dialog"
                        | "menu"
                        | "menuitem"
                        | "details"
                        | "summary"
                ));
        if should_remove {
            remove.push(node_id);
        }
    }

    remove.sort_by_key(|node_id| std::cmp::Reverse(node_id.index()));
    remove.dedup();
    for node_id in remove {
        if let Some(parent_id) = dom.parent_node(node_id) {
            let _ = dom.remove_child(parent_id, node_id);
        }
    }
}

fn collect_node_ids(dom: &NativeDom, node_id: NativeNodeId, out: &mut Vec<NativeNodeId>) {
    let mut stack = vec![node_id];
    while let Some(node_id) = stack.pop() {
        out.push(node_id);
        let child_ids = dom.child_ids(node_id).collect::<Vec<_>>();
        stack.extend(child_ids.into_iter().rev());
    }
}

fn render_markdown_document(dom: &NativeDom) -> String {
    let root = dom.body_node_id().unwrap_or(dom.document_node_id());
    let mut out = String::new();
    render_markdown_node(dom, root, &mut out, 0, MAX_DOM_OUTPUT_TREE_DEPTH);
    normalize_markdown(&out)
}

enum MarkdownFrame {
    Enter {
        node_id: NativeNodeId,
        list_depth: usize,
        remaining_tree_depth: usize,
        target: usize,
    },
    Append {
        target: usize,
        text: &'static str,
    },
    FinishInline {
        source: usize,
        target: usize,
        kind: MarkdownInlineKind,
    },
}

enum MarkdownInlineKind {
    Paragraph,
    Anchor { href: String },
    ListItem { list_depth: usize },
    Strong,
    Emphasis,
    Code,
}

fn render_markdown_node(
    dom: &NativeDom,
    node_id: NativeNodeId,
    out: &mut String,
    list_depth: usize,
    remaining_tree_depth: usize,
) {
    render_markdown_nodes(dom, [node_id], out, list_depth, remaining_tree_depth);
}

fn render_markdown_nodes(
    dom: &NativeDom,
    roots: impl IntoIterator<Item = NativeNodeId>,
    out: &mut String,
    list_depth: usize,
    remaining_tree_depth: usize,
) {
    let mut buffers = vec![String::new()];
    let mut stack = roots
        .into_iter()
        .map(|node_id| MarkdownFrame::Enter {
            node_id,
            list_depth,
            remaining_tree_depth,
            target: 0,
        })
        .collect::<Vec<_>>();
    stack.reverse();

    while let Some(frame) = stack.pop() {
        match frame {
            MarkdownFrame::Enter {
                node_id,
                list_depth,
                remaining_tree_depth,
                target,
            } => render_markdown_enter_node(
                dom,
                node_id,
                &mut buffers,
                &mut stack,
                list_depth,
                remaining_tree_depth,
                target,
            ),
            MarkdownFrame::Append { target, text } => buffers[target].push_str(text),
            MarkdownFrame::FinishInline {
                source,
                target,
                kind,
            } => {
                let text = std::mem::take(&mut buffers[source]);
                finish_inline_markdown(kind, &text, &mut buffers[target]);
            }
        }
    }

    out.push_str(&buffers[0]);
}

fn render_markdown_enter_node(
    dom: &NativeDom,
    node_id: NativeNodeId,
    buffers: &mut Vec<String>,
    stack: &mut Vec<MarkdownFrame>,
    list_depth: usize,
    remaining_tree_depth: usize,
    target: usize,
) {
    let Some(next_tree_depth) = remaining_tree_depth.checked_sub(1) else {
        return;
    };
    let Some(node) = dom.node(node_id) else {
        return;
    };

    match node.kind() {
        NodeData::Document(_) | NodeData::DocumentFragment(_) => {
            push_child_markdown_frames(stack, dom, node_id, list_depth, next_tree_depth, target);
        }
        NodeData::Text(text) => {
            push_markdown_text(&mut buffers[target], text.data());
        }
        NodeData::CDataSection(cdata) => {
            push_markdown_text(&mut buffers[target], cdata.data());
        }
        NodeData::Comment(_) | NodeData::DocumentType(_) | NodeData::ProcessingInstruction(_) => {}
        NodeData::Element(element) => match element.local_name() {
            "head" | "script" | "style" | "noscript" => {}
            "br" => buffers[target].push('\n'),
            "hr" => buffers[target].push_str("\n---\n"),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = element.local_name()[1..].parse::<usize>().unwrap_or(1);
                let text = collapse_whitespace(&dom.text_content(node_id).unwrap_or_default());
                if !text.is_empty() {
                    buffers[target].push('\n');
                    buffers[target].push_str(&"#".repeat(level));
                    buffers[target].push(' ');
                    buffers[target].push_str(&text);
                    buffers[target].push_str("\n\n");
                }
            }
            "p" => {
                push_inline_children_markdown_frames(
                    buffers,
                    stack,
                    dom,
                    node_id,
                    next_tree_depth,
                    target,
                    MarkdownInlineKind::Paragraph,
                );
            }
            "pre" => {
                let text = dom.text_content(node_id).unwrap_or_default();
                if !text.trim().is_empty() {
                    buffers[target].push_str("\n```text\n");
                    buffers[target].push_str(text.trim_end());
                    buffers[target].push_str("\n```\n\n");
                }
            }
            "code" => {
                let text = dom.text_content(node_id).unwrap_or_default();
                finish_inline_markdown(MarkdownInlineKind::Code, &text, &mut buffers[target]);
            }
            "a" => {
                let href = dom.get_attribute(node_id, "href").unwrap_or_default();
                push_inline_children_markdown_frames(
                    buffers,
                    stack,
                    dom,
                    node_id,
                    next_tree_depth,
                    target,
                    MarkdownInlineKind::Anchor { href },
                );
            }
            "img" => {
                let alt = dom.get_attribute(node_id, "alt").unwrap_or_default();
                let src = dom.get_attribute(node_id, "src").unwrap_or_default();
                buffers[target].push_str("![");
                buffers[target].push_str(&alt);
                buffers[target].push_str("](");
                buffers[target].push_str(&src);
                buffers[target].push(')');
            }
            "ul" => {
                stack.push(MarkdownFrame::Append { target, text: "\n" });
                push_child_markdown_frames(
                    stack,
                    dom,
                    node_id,
                    list_depth + 1,
                    next_tree_depth,
                    target,
                );
                stack.push(MarkdownFrame::Append { target, text: "\n" });
            }
            "ol" => {
                stack.push(MarkdownFrame::Append { target, text: "\n" });
                push_child_markdown_frames(
                    stack,
                    dom,
                    node_id,
                    list_depth + 1,
                    next_tree_depth,
                    target,
                );
                stack.push(MarkdownFrame::Append { target, text: "\n" });
            }
            "li" => {
                push_inline_children_markdown_frames(
                    buffers,
                    stack,
                    dom,
                    node_id,
                    next_tree_depth,
                    target,
                    MarkdownInlineKind::ListItem { list_depth },
                );
            }
            "strong" | "b" => {
                push_inline_children_markdown_frames(
                    buffers,
                    stack,
                    dom,
                    node_id,
                    next_tree_depth,
                    target,
                    MarkdownInlineKind::Strong,
                );
            }
            "em" | "i" => {
                push_inline_children_markdown_frames(
                    buffers,
                    stack,
                    dom,
                    node_id,
                    next_tree_depth,
                    target,
                    MarkdownInlineKind::Emphasis,
                );
            }
            _ => {
                if matches!(
                    element.local_name(),
                    "div" | "section" | "article" | "main" | "header" | "footer" | "aside" | "nav"
                ) {
                    buffers[target].push('\n');
                    stack.push(MarkdownFrame::Append { target, text: "\n" });
                }
                push_child_markdown_frames(
                    stack,
                    dom,
                    node_id,
                    list_depth,
                    next_tree_depth,
                    target,
                );
            }
        },
    }
}

fn push_inline_children_markdown_frames(
    buffers: &mut Vec<String>,
    stack: &mut Vec<MarkdownFrame>,
    dom: &NativeDom,
    node_id: NativeNodeId,
    remaining_tree_depth: usize,
    target: usize,
    kind: MarkdownInlineKind,
) {
    let source = buffers.len();
    buffers.push(String::new());
    stack.push(MarkdownFrame::FinishInline {
        source,
        target,
        kind,
    });
    push_child_markdown_frames(stack, dom, node_id, 0, remaining_tree_depth, source);
}

fn push_child_markdown_frames(
    stack: &mut Vec<MarkdownFrame>,
    dom: &NativeDom,
    node_id: NativeNodeId,
    list_depth: usize,
    remaining_tree_depth: usize,
    target: usize,
) {
    let child_ids = dom.child_ids(node_id).collect::<Vec<_>>();
    for child_id in child_ids.into_iter().rev() {
        stack.push(MarkdownFrame::Enter {
            node_id: child_id,
            list_depth,
            remaining_tree_depth,
            target,
        });
    }
}

fn finish_inline_markdown(kind: MarkdownInlineKind, text: &str, out: &mut String) {
    // Spaces at inline element edges belong outside Markdown delimiters, but
    // still separate this element's text from its siblings. Block edges trim.
    let preserve_spacing = matches!(
        kind,
        MarkdownInlineKind::Anchor { .. }
            | MarkdownInlineKind::Strong
            | MarkdownInlineKind::Emphasis
            | MarkdownInlineKind::Code
    );
    let trailing_space = preserve_spacing && text.ends_with(char::is_whitespace);
    if preserve_spacing && text.starts_with(char::is_whitespace) {
        push_markdown_text(out, " ");
    }
    let text = collapse_whitespace(text);
    let text = text.as_str();
    match kind {
        MarkdownInlineKind::Paragraph => {
            if !text.is_empty() {
                out.push('\n');
                out.push_str(text);
                out.push_str("\n\n");
            }
        }
        MarkdownInlineKind::Anchor { href } => {
            if href.is_empty() {
                out.push_str(text);
            } else {
                let label = if text.is_empty() { href.as_str() } else { text };
                out.push('[');
                out.push_str(label);
                out.push_str("](");
                out.push_str(&href);
                out.push(')');
            }
        }
        MarkdownInlineKind::ListItem { list_depth } => {
            if !text.is_empty() {
                out.push_str(&"  ".repeat(list_depth.saturating_sub(1)));
                out.push_str("- ");
                out.push_str(text);
                out.push('\n');
            }
        }
        MarkdownInlineKind::Strong => {
            if !text.is_empty() {
                out.push_str("**");
                out.push_str(text);
                out.push_str("**");
            }
        }
        MarkdownInlineKind::Emphasis => {
            if !text.is_empty() {
                out.push('*');
                out.push_str(text);
                out.push('*');
            }
        }
        MarkdownInlineKind::Code => {
            if !text.is_empty() {
                out.push('`');
                out.push_str(text);
                out.push('`');
            }
        }
    }
    if trailing_space {
        push_markdown_text(out, " ");
    }
}

fn push_markdown_text(out: &mut String, text: &str) {
    // Collapse whitespace across text nodes without discarding leading,
    // trailing, or whitespace-only separators in the source inline flow.
    for character in text.chars() {
        if character.is_whitespace() {
            if !out.ends_with(char::is_whitespace) {
                out.push(' ');
            }
        } else {
            out.push(character);
        }
    }
}

fn normalize_markdown(input: &str) -> String {
    let mut out = String::new();
    let mut previous_blank = false;
    for line in input.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            if !previous_blank && !out.is_empty() {
                out.push('\n');
            }
            previous_blank = true;
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(trimmed);
        previous_blank = false;
    }
    out.trim().to_owned()
}

fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use moli_parser::HtmlParser;
    use url::Url;

    use super::*;

    fn test_url() -> Url {
        Url::parse("https://example.test/").expect("test URL should parse")
    }

    fn append_text(dom: &mut NativeDom, parent: NativeNodeId, text: &str) {
        let text = dom.create_text_node(text);
        assert!(dom.append_child(parent, text));
    }

    fn markdown_from_html(html: &str) -> String {
        let dom = HtmlParser::SCRIPTING_DISABLED.parse(test_url(), html.to_owned());
        render_markdown_document(&dom)
    }

    #[test]
    fn markdown_renderer_preserves_common_inline_and_list_shape() {
        let mut dom = NativeDom::new_html(test_url());
        let body = dom.create_element("body");
        assert!(dom.append_child(dom.document_node_id(), body));

        let paragraph = dom.create_element("p");
        assert!(dom.append_child(body, paragraph));
        append_text(&mut dom, paragraph, "Go ");
        let link = dom.create_element("a");
        assert!(dom.set_attribute(link, "href", "https://example.test/docs"));
        assert!(dom.append_child(paragraph, link));
        append_text(&mut dom, link, " docs ");
        append_text(&mut dom, paragraph, " now");

        let list = dom.create_element("ul");
        let item = dom.create_element("li");
        assert!(dom.append_child(body, list));
        assert!(dom.append_child(list, item));
        append_text(&mut dom, item, "One");

        assert_eq!(
            render_markdown_document(&dom),
            "Go [docs](https://example.test/docs) now\n\n- One"
        );
    }

    #[test]
    fn markdown_renderer_preserves_whitespace_across_inline_nodes() {
        for (html, expected) in [
            (
                "<p>by <small>Albert Einstein</small></p>",
                "by Albert Einstein",
            ),
            (
                "<p><span>538 points</span> by <a href='/user'>onderkalaci</a></p>",
                "538 points by [onderkalaci](/user)",
            ),
            (
                "<div><span>one</span> \n\t <span>two</span></div>",
                "one two",
            ),
            (
                "<p><span>one </span><span> two</span>   three</p>",
                "one two three",
            ),
            (
                "<p>a<span> </span>b<strong> </strong>c<em> </em>d<a> </a>e</p>",
                "a b c d e",
            ),
            ("<p>one&nbsp;<span>two</span></p>", "one two"),
            (
                "<p><span>one</span><!-- comment --> <span>two</span></p>",
                "one two",
            ),
            (
                "<ul><li>one <span>two</span> three</li></ul>",
                "- one two three",
            ),
        ] {
            assert_eq!(markdown_from_html(html), expected, "HTML: {html}");
        }
    }

    #[test]
    fn markdown_renderer_keeps_boundary_spaces_outside_inline_markup() {
        for (html, expected) in [
            (
                "<p>before<a href='/docs'> docs </a>after</p>",
                "before [docs](/docs) after",
            ),
            (
                "<p>before<strong> bold </strong>after</p>",
                "before **bold** after",
            ),
            (
                "<p>before<em> emphasis </em>after</p>",
                "before *emphasis* after",
            ),
            (
                "<p>before<code> code </code>after</p>",
                "before `code` after",
            ),
            (
                "<p>before<strong> <em> nested </em> </strong>after</p>",
                "before ***nested*** after",
            ),
            ("<p>a<code> \t\n </code>b</p>", "a b"),
        ] {
            assert_eq!(markdown_from_html(html), expected, "HTML: {html}");
        }
    }

    #[test]
    fn markdown_renderer_preserves_intentional_inline_adjacency() {
        for (html, expected) in [
            ("<p>word<span>piece</span></p>", "wordpiece"),
            ("<p>日<span>本</span>語</p>", "日本語"),
            ("<p>foo<strong>bar</strong>baz</p>", "foo**bar**baz"),
            (
                "<p>Read <a href='/docs'>docs</a>, <em>now</em>!</p>",
                "Read [docs](/docs), *now*!",
            ),
        ] {
            assert_eq!(markdown_from_html(html), expected, "HTML: {html}");
        }
    }

    #[test]
    fn markdown_renderer_separates_text_at_block_boundaries() {
        for (html, expected) in [
            (
                "<span>Main menu</span><div>Main menu</div>",
                "Main menu\nMain menu",
            ),
            ("<div>outer<div>inner</div>tail</div>", "outer\ninner\ntail"),
            (
                "<span>by <small>Albert Einstein</small></span><div>Tags:\n <a>change</a>\n <a>deep-thoughts</a> <a>thinking</a> <a>world</a></div>",
                "by Albert Einstein\nTags: change deep-thoughts thinking world",
            ),
        ] {
            assert_eq!(markdown_from_html(html), expected, "HTML: {html}");
        }
    }

    #[test]
    fn markdown_renderer_collapses_split_text_and_cdata_whitespace() {
        for use_cdata in [false, true] {
            let mut dom = NativeDom::new_html(test_url());
            let body = dom.create_element("body");
            assert!(dom.append_child(dom.document_node_id(), body));
            for chunk in ["one", " \t", "\n ", "two ", " ", "three"] {
                let node = if use_cdata {
                    dom.create_cdata_section(chunk)
                } else {
                    dom.create_text_node(chunk)
                };
                assert!(dom.append_child(body, node));
            }
            assert_eq!(render_markdown_document(&dom), "one two three");
        }
    }

    #[test]
    fn markdown_renderer_preserves_preformatted_spacing() {
        assert_eq!(
            markdown_from_html("<pre>  first  line\n    second  line\n</pre>"),
            "```text\n  first  line\n    second  line\n```",
        );
    }

    #[test]
    fn markdown_renderer_truncates_deep_tree_with_heap_stack_walk() {
        let mut dom = NativeDom::new_html(test_url());
        let body = dom.create_element("body");
        assert!(dom.append_child(dom.document_node_id(), body));

        let mut parent = body;
        for _ in 0..(MAX_DOM_OUTPUT_TREE_DEPTH + 32) {
            let child = dom.create_element("div");
            assert!(dom.append_child(parent, child));
            parent = child;
        }
        append_text(&mut dom, parent, "too deep");

        assert_eq!(render_markdown_document(&dom), "");
    }
}
