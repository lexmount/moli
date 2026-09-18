use moli_html2md::{Converter, Dom, NodeKind, Options, convert};

// A second DOM implementation, independent of NativeDom or an HTML parser.
// Its stable arena IDs and borrowed strings are enough for the converter.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Node {
    kind: NodeKind<'static>,
    attrs: Vec<(&'static str, &'static str)>,
    first: Option<usize>,
    last: Option<usize>,
    next: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Tree(Vec<Node>);

impl Tree {
    fn new() -> Self {
        Self(vec![Node {
            kind: NodeKind::Document,
            attrs: Vec::new(),
            first: None,
            last: None,
            next: None,
        }])
    }

    fn add(&mut self, parent: usize, kind: NodeKind<'static>) -> usize {
        let id = self.0.len();
        self.0.push(Node {
            kind,
            attrs: Vec::new(),
            first: None,
            last: None,
            next: None,
        });
        if let Some(previous) = self.0[parent].last {
            self.0[previous].next = Some(id);
        } else {
            self.0[parent].first = Some(id);
        }
        self.0[parent].last = Some(id);
        id
    }

    fn element(&mut self, parent: usize, tag: &'static str) -> usize {
        self.add(parent, NodeKind::Element(tag))
    }

    fn text(&mut self, parent: usize, text: &'static str) {
        self.add(parent, NodeKind::Text(text));
    }

    fn attr(&mut self, node: usize, name: &'static str, value: &'static str) {
        self.0[node].attrs.push((name, value));
    }

    fn leaf(&mut self, parent: usize, tag: &'static str, text: &'static str) -> usize {
        let element = self.element(parent, tag);
        self.text(element, text);
        element
    }
}

impl Dom for Tree {
    type NodeId = usize;

    fn node_kind(&self, node: usize) -> NodeKind<'_> {
        self.0[node].kind
    }

    fn first_child(&self, node: usize) -> Option<usize> {
        self.0[node].first
    }

    fn next_sibling(&self, node: usize) -> Option<usize> {
        self.0[node].next
    }

    fn attribute(&self, node: usize, name: &str) -> Option<&str> {
        self.0[node]
            .attrs
            .iter()
            .find_map(|(key, value)| (*key == name).then_some(*value))
    }
}

#[test]
fn coalesces_styles_without_changing_the_tree() {
    let mut dom = Tree::new();
    dom.leaf(0, "em", "foo");
    dom.leaf(0, "i", "bar");
    dom.text(0, " ");
    let bold = dom.element(0, "strong");
    dom.leaf(bold, "b", "nested");
    dom.leaf(0, "strong", "!");
    let before = dom.clone();
    let converter = Converter::default();
    for _ in 0..2 {
        assert_eq!(converter.convert(&dom, 0), "*foobar* **nested!**");
        assert_eq!(dom, before);
    }
}

#[test]
fn moves_whitespace_outside_formatting_but_keeps_nonbreaking_spaces() {
    let mut dom = Tree::new();
    dom.text(0, "by");
    dom.leaf(0, "strong", " \t");
    dom.leaf(0, "small", "Albert Einstein");
    dom.text(0, "\u{a0}");
    let em = dom.element(0, "em");
    dom.leaf(em, "strong", " next ");
    dom.text(0, "word");
    assert_eq!(convert(&dom, 0), "by Albert Einstein\u{a0} ***next*** word");
}

#[test]
fn keeps_separate_links_and_escapes_destinations_and_titles() {
    let mut dom = Tree::new();
    for label in ["one", "two"] {
        let link = dom.leaf(0, "a", label);
        dom.attr(link, "href", "/a(b) c");
        dom.attr(link, "title", "a \"title\"");
    }
    assert_eq!(
        convert(&dom, 0),
        "[one](/a\\(b\\)%20c \"a \\\"title\\\"\")[two](/a\\(b\\)%20c \"a \\\"title\\\"\")"
    );
}

#[test]
fn escapes_literal_markdown_and_decoded_entities_across_text_nodes() {
    let mut dom = Tree::new();
    let p = dom.element(0, "p");
    dom.text(p, "12");
    dom.text(p, ". [x] *y* _z_ `a` <tag> &copy;");
    dom.leaf(0, "p", "# heading");
    dom.leaf(0, "p", "- item");
    dom.text(0, "!");
    let link = dom.leaf(0, "a", "link");
    dom.attr(link, "href", "/");
    assert_eq!(
        convert(&dom, 0),
        "12\\. \\[x\\] \\*y\\* \\_z\\_ \\`a\\` \\<tag\\> &amp;copy;\n\n\\# heading\n\n\\- item\n\n\\![link](/)"
    );
}

#[test]
fn keeps_adjacent_code_elements_distinct_and_chooses_safe_delimiters() {
    let mut dom = Tree::new();
    dom.leaf(0, "code", "a`");
    dom.leaf(0, "code", "`b");
    dom.text(0, " ");
    dom.leaf(0, "code", " ` ");
    dom.text(0, "after");
    assert_eq!(
        rendered_html(&convert(&dom, 0)),
        "<p><code>a`</code> <code>`b</code> <code>`</code> after</p>\n"
    );
}

#[test]
fn preserves_preformatted_text_including_blank_lines_and_nested_elements() {
    let mut dom = Tree::new();
    let pre = dom.element(0, "pre");
    let code = dom.element(pre, "code");
    dom.attr(code, "class", "highlight language-rust");
    dom.text(code, "  first\r\n\r\n");
    dom.leaf(code, "span", "```\n");
    dom.text(code, "\n");
    assert_eq!(convert(&dom, 0), "````rust\n  first\n\n```\n\n````");
}

#[test]
fn prefixes_multiline_list_items_and_quotes() {
    let mut dom = Tree::new();
    let quote = dom.element(0, "blockquote");
    let list = dom.element(quote, "ol");
    dom.attr(list, "start", "9");
    let item = dom.element(list, "li");
    dom.leaf(item, "p", "first");
    dom.leaf(item, "p", "second");
    dom.leaf(list, "li", "last");
    assert_eq!(
        convert(&dom, 0),
        "> 9. first\n>\n>    second\n>\n> 10. last"
    );
}

#[test]
fn supports_headings_images_and_non_content_elements() {
    let mut dom = Tree::new();
    dom.leaf(0, "h2", "Heading");
    let image = dom.element(0, "img");
    dom.attr(image, "src", "/image.png");
    dom.attr(image, "alt", "[image]");
    dom.element(0, "hr");
    for tag in ["head", "script", "style", "noscript", "template"] {
        dom.leaf(0, tag, "invisible");
    }
    dom.add(0, NodeKind::Other);
    dom.leaf(0, "p", "visible");
    assert_eq!(
        convert(&dom, 0),
        "## Heading\n\n![\\[image\\]](/image.png)\n\n---\n\nvisible"
    );
}

#[test]
fn adds_empty_headers_to_headerless_tables() {
    let mut dom = Tree::new();
    let table = dom.element(0, "table");
    let row = dom.element(table, "tr");
    dom.leaf(row, "td", "one");
    dom.leaf(row, "td", "two");
    assert_eq!(convert(&dom, 0), "|  |  |\n| --- | --- |\n| one | two |");
}

#[test]
fn preserves_table_cell_boundaries_hard_breaks_and_literal_pipes() {
    let mut dom = Tree::new();
    let table = dom.element(0, "table");
    let header = dom.element(table, "tr");
    let cell = dom.leaf(header, "th", "Key");
    dom.attr(cell, "align", "right");
    let row = dom.element(table, "tr");
    let cell = dom.element(row, "td");
    dom.text(cell, "a|b");
    dom.element(cell, "br");
    dom.text(cell, "c");
    assert_eq!(convert(&dom, 0), "| Key |\n| ---: |\n| a\\|b<br>c |");
}

#[test]
fn table_spans_do_not_duplicate_cell_text() {
    let mut dom = Tree::new();
    let table = dom.element(0, "table");
    let row = dom.element(table, "tr");
    let cell = dom.leaf(row, "td", "one");
    dom.attr(cell, "rowspan", "2");
    dom.leaf(row, "td", "two");
    let row = dom.element(table, "tr");
    let cell = dom.leaf(row, "td", "end");
    dom.attr(cell, "colspan", "999999999999");
    assert_eq!(convert(&dom, 0), "one\n\ntwo\n\nend");
}

#[test]
fn bounds_every_traversal_including_raw_code() {
    for tag in [
        "div",
        "span",
        "strong",
        "blockquote",
        "table",
        "pre",
        "code",
    ] {
        let mut dom = Tree::new();
        dom.text(0, "visible");
        let mut node = 0;
        for _ in 0..30 {
            node = dom.element(node, tag);
        }
        dom.text(node, "too deep");
        let converter = Converter::new(Options {
            max_depth: 10,
            ..Options::default()
        });
        let result = converter.convert(&dom, 0);
        assert!(!result.contains("too deep"), "{tag}: {result}");
        assert!(result.starts_with("visible"), "{tag}: {result}");
    }
}

#[test]
fn converts_deep_trees_on_a_small_thread_stack() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            for tag in ["span", "strong", "pre"] {
                let mut dom = Tree::new();
                let mut node = 0;
                for _ in 0..20_000 {
                    node = dom.element(node, tag);
                }
                dom.text(node, "deep");
                let converter = Converter::new(Options {
                    max_depth: usize::MAX,
                    ..Options::default()
                });
                let expected = match tag {
                    "strong" => "**deep**",
                    "pre" => "```\ndeep\n```",
                    _ => "deep",
                };
                assert_eq!(converter.convert(&dom, 0), expected);
            }
        })
        .expect("spawn small-stack test")
        .join()
        .expect("iterative conversion should complete");
}

#[test]
fn converts_deep_tables_on_a_small_thread_stack() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let mut dom = Tree::new();
            let mut node = 0;
            for _ in 0..10_000 {
                node = dom.element(node, "table");
                node = dom.element(node, "tr");
                node = dom.element(node, "td");
            }
            dom.text(node, "deep");
            let converter = Converter::new(Options {
                max_depth: usize::MAX,
                ..Options::default()
            });
            assert_eq!(converter.convert(&dom, 0), "|  |\n| --- |\n| deep |");
        })
        .expect("spawn small-stack test")
        .join()
        .expect("nested tables should not recurse");
}

#[test]
fn converts_nested_header_tables_on_a_small_thread_stack() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            let mut dom = Tree::new();
            let mut node = 0;
            for _ in 0..1_000 {
                let table = dom.element(node, "table");
                let row = dom.element(table, "tr");
                dom.leaf(row, "th", "Header");
                let row = dom.element(table, "tr");
                node = dom.element(row, "td");
            }
            dom.text(node, "deep");
            let converter = Converter::new(Options {
                max_depth: usize::MAX,
                ..Options::default()
            });
            let actual = converter.convert(&dom, 0);
            assert_eq!(actual.matches("Header").count(), 1_000);
            assert_eq!(actual.matches("| --- |").count(), 1);
            assert!(actual.ends_with("| deep |"));
        })
        .expect("spawn small-stack test")
        .join()
        .expect("header table conversion should not recurse");
}

#[test]
fn conversion_is_limited_to_the_supplied_subtree() {
    let mut dom = Tree::new();
    let first = dom.leaf(0, "p", "first");
    dom.leaf(0, "p", "second");
    assert_eq!(convert(&dom, first), "first");
    let converter = Converter::new(Options {
        max_depth: 0,
        ..Options::default()
    });
    assert_eq!(converter.convert(&dom, first), "");
}

fn rendered_html(markdown: &str) -> String {
    let parser = pulldown_cmark::Parser::new_ext(
        markdown,
        pulldown_cmark::Options::ENABLE_TABLES | pulldown_cmark::Options::ENABLE_STRIKETHROUGH,
    );
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, parser);
    html
}

#[test]
fn markdown_parser_recovers_adjacent_and_nested_formatting() {
    let mut dom = Tree::new();
    dom.leaf(0, "em", "foo");
    dom.leaf(0, "em", "bar");
    dom.text(0, " ");
    let strong = dom.element(0, "strong");
    dom.leaf(strong, "em", "both");
    dom.text(0, " ");
    dom.leaf(0, "del", "deleted");
    assert_eq!(
        rendered_html(&convert(&dom, 0)),
        "<p><em>foobar</em> <em><strong>both</strong></em> <del>deleted</del></p>\n"
    );
}

#[test]
fn markdown_parser_recovers_unicode_spaces_at_emphasis_boundaries() {
    let mut dom = Tree::new();
    dom.leaf(0, "em", "\u{a0} a \u{2003}");
    dom.text(0, "b");
    assert_eq!(
        rendered_html(&convert(&dom, 0)),
        "<p>\u{a0} <em>a</em> \u{2003}b</p>\n"
    );
    let mut dom = Tree::new();
    dom.leaf(0, "p", "\u{a0} a");
    assert_eq!(convert(&dom, 0), "\u{a0} a");
}

#[test]
fn markdown_parser_recovers_intraword_emphasis_next_to_punctuation() {
    for (tag, html_tag) in [("strong", "strong"), ("em", "em"), ("del", "del")] {
        for text in ["(word)", "word!", "!word", "日本語。"] {
            let mut dom = Tree::new();
            dom.text(0, "before");
            dom.leaf(0, tag, text);
            dom.text(0, "after");
            let markdown = convert(&dom, 0);
            assert_eq!(
                rendered_html(&markdown),
                format!("<p>before<{html_tag}>{text}</{html_tag}>after</p>\n"),
                "{markdown}"
            );
        }
    }
}

#[test]
fn markdown_parser_recovers_literal_hashes_at_the_end_of_headings() {
    let mut dom = Tree::new();
    dom.leaf(0, "h1", "title #");
    assert_eq!(rendered_html(&convert(&dom, 0)), "<h1>title #</h1>\n");
}

#[test]
fn markdown_parser_recovers_pipes_and_preformatted_code_inside_tables() {
    let mut dom = Tree::new();
    let table = dom.element(0, "table");
    let header = dom.element(table, "tr");
    dom.leaf(header, "th", "Code");
    let row = dom.element(table, "tr");
    let cell = dom.element(row, "td");
    dom.leaf(cell, "code", "a|b");
    let row = dom.element(table, "tr");
    let cell = dom.element(row, "td");
    dom.leaf(cell, "pre", "  <a>|b\nnext\n");
    let markdown = convert(&dom, 0);
    let html = rendered_html(&markdown);
    assert_eq!(
        html,
        "<p>Code</p>\n<p><code>a|b</code></p>\n<pre><code>  &lt;a&gt;|b\nnext\n</code></pre>\n",
        "{markdown}"
    );
}
