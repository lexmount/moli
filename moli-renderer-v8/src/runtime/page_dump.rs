use crate::dom::native::{NativeDom, NativeNodeId};
use crate::runtime::page_surface::{
    RendererPageDumpFormat, RendererPageDumpOptions, RendererPageDumpStripOptions,
};
use moli_page_types::MAX_DOM_OUTPUT_TREE_DEPTH;

use super::page_vm::PageVm;

impl PageVm {
    pub(crate) fn render_page_dump(&mut self, options: RendererPageDumpOptions) -> String {
        if options.format == RendererPageDumpFormat::Markdown
            && !options.with_base
            && !options.with_frames
            && options.strip == RendererPageDumpStripOptions::default()
        {
            return render_markdown_document(self.vm().document_runtime.dom_host().dom());
        }
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
    moli_html2md::Converter::new(moli_html2md::Options {
        max_depth: MAX_DOM_OUTPUT_TREE_DEPTH,
        default_code_language: Some("text".to_owned()),
        ..Default::default()
    })
    .convert(dom, root)
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
            ("<p>one&nbsp;<span>two</span></p>", "one\u{00a0}two"),
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
                "<p>before<a href='/docs'><em> docs </em></a>after</p>",
                "before [*docs*](/docs) after",
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
                "Main menu\n\nMain menu",
            ),
            (
                "<div>outer<div>inner</div>tail</div>",
                "outer\n\ninner\n\ntail",
            ),
            (
                "<span>by <small>Albert Einstein</small></span><div>Tags:\n <a>change</a>\n <a>deep-thoughts</a> <a>thinking</a> <a>world</a></div>",
                "by Albert Einstein\n\nTags: change deep-thoughts thinking world",
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
    fn markdown_renderer_expands_table_cells() {
        assert_eq!(
            markdown_from_html(
                "<table><tr><th>Name</th><th>Count</th></tr><tr><td>moli</td><td>2</td></tr></table>"
            ),
            "Name\n\nCount\n\nmoli\n\n2"
        );
    }

    #[test]
    fn markdown_renderer_flattens_hacker_news_layout_tables() {
        let dom = HtmlParser::SCRIPTING_DISABLED.parse(
            test_url(),
            include_str!("../../../moli-html2md/tests/fixtures/hacker-news-layout.html").to_owned(),
        );
        let before = dom.serialize_document();
        assert_eq!(
            render_markdown_document(&dom),
            include_str!("../../../moli-html2md/tests/fixtures/hacker-news-layout.md").trim_end()
        );
        assert_eq!(dom.serialize_document(), before);
    }

    #[test]
    fn markdown_renderer_preserves_ordered_and_nested_lists() {
        assert_eq!(
            markdown_from_html(
                "<ol start='3'><li>three<ul><li>nested</li></ul></li><li>four</li></ol>"
            ),
            "3. three\n   - nested\n4. four"
        );
    }

    #[test]
    fn markdown_renderer_preserves_blockquotes_and_hard_breaks() {
        assert_eq!(
            markdown_from_html("<blockquote><p>first<br>second</p></blockquote>"),
            "> first  \n> second"
        );
    }

    #[test]
    fn markdown_renderer_preserves_code_language_fences_and_blank_lines() {
        assert_eq!(
            markdown_from_html(
                "<pre><code class='language-rust'>let s = \"```\";\n\n\nend\n</code></pre>"
            ),
            "````rust\nlet s = \"```\";\n\n\nend\n````"
        );
        assert_eq!(
            markdown_from_html("<p>before<code> a`b </code>after</p>"),
            "before ``a`b`` after"
        );
    }

    #[test]
    fn markdown_renderer_skips_non_content_tags() {
        assert_eq!(
            markdown_from_html(
                "<head><title>title</title></head><body><script>script</script><style>style</style><noscript>noscript</noscript><p>Visible</p></body>"
            ),
            "Visible"
        );
    }

    #[test]
    fn markdown_renderer_keeps_boundaries_around_empty_blocks() {
        for tag in ["p", "div", "h2", "blockquote", "ul", "ol", "pre", "table"] {
            for content in ["", " \n\t "] {
                let html = format!("before<{tag}>{content}</{tag}>after");
                assert_eq!(markdown_from_html(&html), "before\n\nafter", "{html}");
            }
        }
    }

    #[test]
    fn markdown_renderer_keeps_empty_links_and_links_around_blocks() {
        for (html, expected) in [
            ("<a href='/a'></a><a href='/b'></a>", "[](/a)[](/b)"),
            ("<a href=''>label</a>", "label"),
            ("<a id='target'>label</a>", "label"),
            ("<a href='/a'><h2>heading</h2></a>", "## [heading](/a)"),
            (
                "<a href='/a'><blockquote>quote</blockquote></a>",
                "> [quote](/a)",
            ),
            ("<a href='/a'><ul><li>item</li></ul></a>", "- [item](/a)"),
            ("<a href='/a'><img src='/i'></a>", "[![](/i)](/a)"),
        ] {
            assert_eq!(markdown_from_html(html), expected, "{html}");
        }
    }

    #[test]
    fn markdown_renderer_keeps_paragraphs_in_list_items() {
        for (html, expected) in [
            ("<ul><li><p>one</p></li><li>two</li></ul>", "- one\n\n- two"),
            ("<ul><li>one</li><li><p>two</p></li></ul>", "- one\n\n- two"),
            ("<ol start='3'><li></li><li>four</li></ol>", "4. four"),
            (
                "<ul><li>parent<ul><li>child</li></ul>tail</li></ul>",
                "- parent\n  - child\n\n  tail",
            ),
        ] {
            assert_eq!(markdown_from_html(html), expected, "{html}");
        }
    }

    #[test]
    fn markdown_renderer_normalizes_image_and_link_attribute_newlines() {
        for (html, expected) in [
            (
                "<a href='/a' title='one\n  two'>link</a>",
                "[link](/a \"one\ntwo\")",
            ),
            (
                "<img src='/i' alt='one\n  two' title='one\n  two'>",
                "![one\ntwo](/i \"one\ntwo\")",
            ),
            (
                "<h2><a href='/a' title='one\n  two'>link</a></h2>",
                "## [link](/a \"one&#10;two\")",
            ),
            ("a<img alt='label'>b<img src='' alt='label'>c", "abc"),
        ] {
            assert_eq!(markdown_from_html(html), expected, "{html}");
        }
    }

    #[test]
    fn markdown_renderer_preserves_literal_entities_and_split_list_markers() {
        assert_eq!(
            markdown_from_html("<p>&amp;copy; &lt;b&gt;literal&lt;/b&gt;</p>"),
            "&amp;copy; \\<b\\>literal\\</b\\>"
        );
        let mut dom = NativeDom::new_html(test_url());
        let body = dom.create_element("body");
        assert!(dom.append_child(dom.document_node_id(), body));
        for chunk in ["12", ".", " ", "item"] {
            append_text(&mut dom, body, chunk);
        }
        assert_eq!(render_markdown_document(&dom), "12\\. item");
    }

    #[test]
    fn markdown_converter_supports_preformatted_inline_code_on_native_dom() {
        let dom = HtmlParser::SCRIPTING_DISABLED.parse(
            test_url(),
            "<body>before<code> a  b </code>after</body>".to_owned(),
        );
        let before = dom.serialize_document();
        let root = dom.body_node_id().expect("body");
        let converter = moli_html2md::Converter::new(moli_html2md::Options {
            preformatted_code: true,
            ..Default::default()
        });
        assert_eq!(converter.convert(&dom, root), "before`  a  b  `after");
        assert_eq!(render_markdown_document(&dom), "before `a b` after");
        assert_eq!(dom.serialize_document(), before);
    }

    #[test]
    fn markdown_renderer_reads_dom_without_mutating_nodes() {
        let dom = HtmlParser::SCRIPTING_DISABLED.parse(
            test_url(),
            "<body><p><em>foo</em><i>bar</i></p><pre>  a  b\n</pre></body>".to_owned(),
        );
        let before = dom.serialize_document();
        assert_eq!(
            render_markdown_document(&dom),
            "*foobar*\n\n```text\n  a  b\n```"
        );
        assert_eq!(dom.serialize_document(), before);
    }

    #[test]
    fn markdown_renderer_truncates_deep_tree() {
        for tag in ["div", "span", "strong", "blockquote"] {
            let mut dom = NativeDom::new_html(test_url());
            let body = dom.create_element("body");
            assert!(dom.append_child(dom.document_node_id(), body));
            append_text(&mut dom, body, "Visible");
            let mut parent = body;
            for _ in 0..(MAX_DOM_OUTPUT_TREE_DEPTH + 32) {
                let child = dom.create_element(tag);
                assert!(dom.append_child(parent, child));
                parent = child;
            }
            append_text(&mut dom, parent, "too deep");
            assert_eq!(render_markdown_document(&dom), "Visible", "tag: {tag}");
        }
    }
}
