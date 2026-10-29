mod support;

use moli_html2md::{Converter, Dom, Options, convert};
use support::{Tree, rendered_html};

fn containers(content: &str) -> [String; 3] {
    [
        format!("<div>{content}</div>"),
        format!("<table><tr><td>{content}</td><td>Other</td></tr></table>"),
        format!("<table><tr><td colspan='2'>{content}</td></tr></table>"),
    ]
}

fn descendants(tree: &Tree, root: usize) -> Vec<usize> {
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        found.push(node);
        let mut children = Vec::new();
        let mut child = tree.first_child(node);
        while let Some(node) = child {
            children.push(node);
            child = tree.next_sibling(node);
        }
        stack.extend(children.into_iter().rev());
    }
    found
}

fn text(tree: &Tree, root: usize) -> String {
    descendants(tree, root)
        .into_iter()
        .filter_map(|node| tree.nodes[node].text.as_deref())
        .collect()
}

fn with_attribute(tree: &Tree, name: &str, value: &str) -> Vec<usize> {
    descendants(tree, tree.root)
        .into_iter()
        .filter(|&node| tree.attribute(node, name) == Some(value))
        .collect()
}

#[test]
fn descendant_targets_preserve_author_link_ownership() {
    for tag in ["span", "kbd"] {
        let content =
            format!("<a href='#x'>Jump</a><a href='/doc'><{tag} id='x'>linked text</{tag}></a>");
        for source in containers(&content) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let links = with_attribute(&output, "href", "/doc");
            assert_eq!(links.len(), 1, "{source}: {markdown}");
            assert_eq!(text(&output, links[0]), "linked text", "{markdown}");
            let targets = with_attribute(&output, "id", "x");
            assert_eq!(targets.len(), 1, "{markdown}");
            assert!(
                descendants(&output, links[0]).contains(&targets[0]),
                "{markdown}"
            );
        }
    }
}

#[test]
fn media_sources_keep_the_author_link_around_its_text() {
    for tag in ["video", "audio", "iframe"] {
        let content = format!("<a href='/watch'><{tag} src='/resource'></{tag}>Watch</a>");
        for source in containers(&content) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let links = with_attribute(&output, "href", "/watch");
            assert!(
                links
                    .iter()
                    .any(|&node| text(&output, node).contains("Watch")),
                "{source}: {markdown}"
            );
            assert!(
                links
                    .iter()
                    .any(|&node| text(&output, node).contains("/resource")),
                "{source}: {markdown}"
            );
            assert!(
                with_attribute(&output, "href", "/resource").is_empty(),
                "{markdown}"
            );
        }
    }
}

fn block_text(tree: &Tree, root: usize) -> String {
    let mut result = String::new();
    for node in descendants(tree, root) {
        if tree.nodes[node].tag.as_deref().is_some_and(|tag| {
            matches!(
                tag,
                "p" | "div"
                    | "table"
                    | "tr"
                    | "td"
                    | "th"
                    | "br"
                    | "section"
                    | "article"
                    | "aside"
                    | "main"
                    | "nav"
                    | "header"
                    | "footer"
                    | "figure"
                    | "figcaption"
                    | "form"
                    | "fieldset"
                    | "legend"
                    | "address"
            )
        }) {
            result.push('\n');
        }
        if let Some(text) = &tree.nodes[node].text {
            result.push_str(text);
        }
    }
    result
}

#[test]
fn authored_block_boundaries_survive_every_output_path() {
    for tag in [
        "section",
        "article",
        "aside",
        "main",
        "nav",
        "header",
        "footer",
        "figure",
        "figcaption",
        "form",
        "fieldset",
        "legend",
        "address",
    ] {
        for source in containers(&format!("<{tag}>A</{tag}><{tag}>B</{tag}>")) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let text = block_text(&output, output.root);
            let words: Vec<_> = text.split_whitespace().collect();
            assert!(
                words.windows(2).any(|words| words == ["A", "B"]),
                "{source}: {markdown}\n{text}"
            );
        }
    }
}

#[test]
fn empty_targets_and_formatting_do_not_hide_accessible_names() {
    for icon in [
        "<span id='icon'></span>",
        "<code><span id='icon'></span></code>",
        "<pre><code><span id='icon'></span></code></pre>",
        "<h2><span id='icon'></span></h2>",
        "<blockquote><span id='icon'></span></blockquote>",
        "<ul><li><span id='icon'></span></li></ul>",
        "<i class='fa fa-search'></i>",
        "<em><strong></strong></em>",
        "<sub></sub>",
        "<span>&nbsp;</span>",
    ] {
        for control in ["button", "a href='/search'"] {
            let tag = control.split_whitespace().next().unwrap();
            let content =
                format!("<a href='#icon'>Jump</a><{control} aria-label='Search'>{icon}</{tag}>");
            for source in containers(&content) {
                let input = Tree::parse(&source);
                let markdown = convert(&input, input.root);
                let output = Tree::parse(&rendered_html(&markdown));
                assert_eq!(
                    text(&output, output.root).matches("Search").count(),
                    1,
                    "{source}: {markdown}"
                );
                if tag == "a" {
                    let links = with_attribute(&output, "href", "/search");
                    assert!(
                        links
                            .iter()
                            .any(|&node| text(&output, node).contains("Search")),
                        "{markdown}"
                    );
                }
            }
        }
    }
}

#[test]
fn meaningful_objects_still_suppress_redundant_accessible_names() {
    for object in [
        "<img src='/icon.svg' alt=''>",
        "<math><mi>x</mi></math>",
        "<video src='/clip'></video>",
    ] {
        for source in containers(&format!("<button aria-label='Search'>{object}</button>")) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            assert!(
                !text(&output, output.root).contains("Search"),
                "{source}: {markdown}"
            );
        }
    }
}

#[test]
fn explicit_header_references_keep_ids_on_actual_header_cells() {
    struct NoFragments(Tree);
    impl Dom for NoFragments {
        type NodeId = usize;
        fn node_kind(&self, node: usize) -> moli_html2md::NodeKind<'_> {
            self.0.node_kind(node)
        }
        fn first_child(&self, node: usize) -> Option<usize> {
            self.0.first_child(node)
        }
        fn next_sibling(&self, node: usize) -> Option<usize> {
            self.0.next_sibling(node)
        }
        fn attribute(&self, node: usize, name: &str) -> Option<&str> {
            self.0.attribute(node, name)
        }
        fn may_have_fragment_links(&self) -> bool {
            false
        }
    }
    for span in ["", " colspan='2'"] {
        let source = format!(
            "<table><tr><th id='group'{span}>Group</th><th id='detail'>Detail</th></tr>\
             <tr><td headers='group detail'>A</td><td headers='group'>B</td></tr></table>"
        );
        let tree = Tree::parse(&source);
        let hinted = NoFragments(tree.clone());
        for markdown in [convert(&tree, tree.root), convert(&hinted, hinted.0.root)] {
            let output = Tree::parse(&rendered_html(&markdown));
            let tables = with_attribute(&output, "id", "group");
            assert_eq!(tables.len(), 1, "{markdown}");
            for cell in output
                .nodes
                .iter()
                .filter(|node| node.tag.as_deref() == Some("td"))
            {
                let references = cell.attrs.iter().find(|(key, _)| key == "headers");
                assert!(references.is_some(), "{markdown}");
                for id in references.unwrap().1.split_ascii_whitespace() {
                    let targets = with_attribute(&output, "id", id);
                    assert_eq!(targets.len(), 1, "{markdown}");
                    assert_eq!(
                        output.nodes[targets[0]].tag.as_deref(),
                        Some("th"),
                        "{markdown}"
                    );
                }
            }
        }
    }
}

#[test]
fn wide_math_targets_keep_order_and_are_emitted_once() {
    use std::fmt::Write as _;

    let count = 2048;
    let mut source = String::new();
    for index in 0..count {
        write!(source, "<a href='#m{index}'>Term</a>").unwrap();
    }
    source.push_str("<math>");
    for index in 0..count {
        write!(source, "<mrow id='m{index}'><mi>x</mi></mrow>").unwrap();
    }
    source.push_str("<mrow id='m0'><mi>y</mi></mrow></math>");
    let input = Tree::parse(&source);
    let markdown = convert(&input, input.root);
    let output = Tree::parse(&rendered_html(&markdown));
    let targets: Vec<_> = descendants(&output, output.root)
        .into_iter()
        .filter_map(|node| output.attribute(node, "id"))
        .filter(|id| *id != "turndown-root")
        .collect();
    let expected: Vec<_> = (0..count).map(|index| format!("m{index}")).collect();
    assert_eq!(targets, expected);
}

#[test]
fn nested_presentational_tables_expand_without_affecting_inner_data_tables() {
    for role in ["presentation", "none", " PRESENTATION "] {
        for inner_table in [
            "",
            "<table><tr><th>Key</th><th>Value</th></tr><tr><td>C</td><td>D</td></tr></table>",
        ] {
            let content =
                format!("<table role='{role}'><tr><td>A</td><td>B{inner_table}</td></tr></table>");
            for (source, outer) in [
                (content.clone(), 0),
                (
                    format!("<table><tr><td colspan='2'>{content}</td></tr></table>"),
                    1,
                ),
            ] {
                let input = Tree::parse(&source);
                let markdown = convert(&input, input.root);
                let output = Tree::parse(&rendered_html(&markdown));
                let tables = output
                    .nodes
                    .iter()
                    .filter(|node| node.tag.as_deref() == Some("table"))
                    .count();
                assert_eq!(
                    tables,
                    outer + usize::from(!inner_table.is_empty()),
                    "{source}: {markdown}"
                );
                let text = block_text(&output, output.root);
                let words: Vec<_> = text.split_whitespace().collect();
                assert!(
                    words.windows(2).any(|words| words == ["A", "B"]),
                    "{markdown}"
                );
            }
        }
    }
}

#[test]
fn gfm_section_targets_land_in_their_first_cell() {
    let source = "<a href='#first'>First</a><a href='#second'>Second</a><a href='#empty'>Empty</a>\
        <table><tbody id='first'><tr><td>A</td><td>B</td></tr></tbody>\
        <tbody id='second'><tr><td>C</td><td>D</td></tr></tbody><tbody id='empty'></tbody></table>";
    let input = Tree::parse(source);
    let markdown = convert(&input, input.root);
    assert!(markdown.contains("| --- |"), "{markdown}");
    let output = Tree::parse(&rendered_html(&markdown));
    for (id, expected_cell) in [("first", "A"), ("second", "C")] {
        let targets = with_attribute(&output, "id", id);
        assert_eq!(targets.len(), 1, "{markdown}");
        let cells: Vec<_> = descendants(&output, output.root)
            .into_iter()
            .filter(|&node| {
                output.nodes[node].tag.as_deref() == Some("td")
                    && descendants(&output, node).contains(&targets[0])
            })
            .collect();
        assert_eq!(cells.len(), 1, "{markdown}");
        assert_eq!(text(&output, cells[0]).trim(), expected_cell, "{markdown}");
    }
    assert_eq!(
        with_attribute(&output, "id", "empty").len(),
        1,
        "{markdown}"
    );
}

#[test]
fn input_values_keep_their_word_boundaries_in_every_output_path() {
    for (content, expected) in [
        ("<input value='10'><input value='20'>", vec!["10", "20"]),
        (
            "<label>Min<input value='10'></label><label>Max<input value='20'></label>",
            vec!["Min", "10", "Max", "20"],
        ),
    ] {
        for source in containers(content) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let cell = descendants(&output, output.root)
                .into_iter()
                .find(|&node| output.nodes[node].tag.as_deref() == Some("td"))
                .unwrap_or(output.root);
            let content = text(&output, cell);
            assert_eq!(
                content.split_whitespace().collect::<Vec<_>>(),
                expected,
                "{source}: {markdown}"
            );
        }
    }
}

#[test]
fn retained_dialogs_and_details_preserve_their_open_state() {
    for tag in ["dialog", "details"] {
        for state in ["open", "open=''", "open='false'", ""] {
            let input = Tree::parse(&format!(
                "<table><tr><td colspan='2'><{tag} {state}>Important content</{tag}></td></tr></table>"
            ));
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let elements: Vec<_> = descendants(&output, output.root)
                .into_iter()
                .filter(|&node| output.nodes[node].tag.as_deref() == Some(tag))
                .collect();
            assert_eq!(elements.len(), 1, "{markdown}");
            assert_eq!(
                output.attribute(elements[0], "open").is_some(),
                !state.is_empty(),
                "{markdown}"
            );
            assert_eq!(
                text(&output, elements[0]),
                "Important content",
                "{markdown}"
            );
        }
    }
}

#[test]
fn inline_html_fallback_inherits_and_releases_the_writer_link_context() {
    for media in ["video", "audio", "iframe"] {
        let content = format!(
            "<a href='/watch'><code id='snippet'><{media} src='/v.mp4'></{media}></code>Watch</a>\
             <a href='#snippet'>Jump</a>\
             <code id='outside'><video src='/outside.mp4'></video></code><a href='#outside'>Outside</a>"
        );
        for source in containers(&content) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let links = with_attribute(&output, "href", "/watch");
            assert_eq!(links.len(), 1, "{source}: {markdown}");
            let body = text(&output, links[0]);
            assert!(
                body.contains("Watch") && body.contains("/v.mp4"),
                "{source}: {markdown}"
            );
            let targets = with_attribute(&output, "id", "snippet");
            assert_eq!(targets.len(), 1, "{source}: {markdown}");
            assert!(
                descendants(&output, links[0]).contains(&targets[0]),
                "{markdown}"
            );
            assert!(
                with_attribute(&output, "href", "/v.mp4").is_empty(),
                "{markdown}"
            );
            let outside = with_attribute(&output, "href", "/outside.mp4");
            assert_eq!(outside.len(), 1, "{markdown}");
            assert!(
                !descendants(&output, links[0]).contains(&outside[0]),
                "{markdown}"
            );
        }
    }
}

#[test]
fn code_whitespace_names_follow_the_preformatted_code_option() {
    for (payload, default_name, preserved_name) in [
        ("", true, true),
        (" ", true, false),
        (" \t\n", true, false),
        ("&nbsp;", true, false),
        ("<span> </span>", true, false),
        ("x", false, false),
    ] {
        for tag in ["code", "pre"] {
            for referenced in [false, true] {
                let target = if referenced { "id='blank'" } else { "" };
                let content = format!(
                    "<a href='#blank'>Jump</a><a href='/search' aria-label='Search'><{tag} {target}>{payload}</{tag}></a>"
                );
                for source in containers(&content) {
                    let input = Tree::parse(&source);
                    for preformatted_code in [false, true] {
                        let converter = Converter::new(Options {
                            preformatted_code,
                            ..Default::default()
                        });
                        let markdown = converter.convert(&input, input.root);
                        let output = Tree::parse(&rendered_html(&markdown));
                        let expected = if tag == "code" && preformatted_code {
                            preserved_name
                        } else {
                            default_name
                        };
                        assert_eq!(
                            text(&output, output.root).matches("Search").count(),
                            usize::from(expected),
                            "preformatted_code={preformatted_code}: {source}: {markdown}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn serialized_state_filter_applies_before_special_element_branches() {
    let payload = format!("{{\"x\":\"{}\"}}", "a".repeat(512));
    let prose = "Useful hidden content ".repeat(30);
    for (attributes, body, filtered) in [
        ("hidden", payload.as_str(), true),
        ("style='display:none'", payload.as_str(), true),
        (
            "style='display:none!important;display:block'",
            payload.as_str(),
            true,
        ),
        ("", payload.as_str(), false),
        ("hidden", prose.as_str(), false),
        ("hidden", "{\"short\":1}", false),
        (
            "style='display:none;display:block'",
            payload.as_str(),
            false,
        ),
    ] {
        for tag in ["textarea", "math", "code", "span"] {
            let content = format!("<{tag} {attributes}>{body}</{tag}>");
            for source in containers(&content) {
                let input = Tree::parse(&source);
                let markdown = convert(&input, input.root);
                let output = Tree::parse(&rendered_html(&markdown));
                assert_eq!(
                    text(&output, output.root).contains(body.trim()),
                    !filtered,
                    "{source}: {markdown}"
                );
            }
        }
    }
}

#[test]
fn percent_encoded_fragments_replace_invalid_utf8_after_literal_matching() {
    for (fragment, target) in [
        ("%FF", "�"),
        ("%E4%B8", "�"),
        ("%C0%AF", "��"),
        ("caf%C3%A9%FF", "café�"),
        ("caf%C3%A9", "café"),
    ] {
        let content = format!("<a href='#{fragment}'>Jump</a><p id='{target}'>Target</p>");
        for source in containers(&content) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            assert_eq!(
                with_attribute(&output, "id", target).len(),
                1,
                "{source}: {markdown}"
            );
            assert_eq!(
                with_attribute(&output, "href", &format!("#{fragment}")).len(),
                1,
                "{markdown}"
            );
        }
    }
    for source in containers("<a href='#%FF'>Jump</a><p id='%FF'>Literal</p><p id='�'>Decoded</p>")
    {
        let input = Tree::parse(&source);
        let markdown = convert(&input, input.root);
        let output = Tree::parse(&rendered_html(&markdown));
        assert_eq!(with_attribute(&output, "id", "%FF").len(), 1, "{markdown}");
        assert!(with_attribute(&output, "id", "�").is_empty(), "{markdown}");
    }
}

#[test]
fn code_links_preserve_source_text_without_accessible_name_fallbacks() {
    for payload in ["", " ", "\t", "&nbsp;", "<span> </span>", "symbol"] {
        for pre in [false, true] {
            let code = format!(
                "<code>x<a href='/symbol' title='Tooltip' aria-label='Symbol'>{payload}</a>y</code>"
            );
            let content = if pre {
                format!("<pre>{code}</pre>")
            } else {
                code
            };
            for source in containers(&content) {
                let input = Tree::parse(&source);
                let original_code = descendants(&input, input.root)
                    .into_iter()
                    .find(|&node| input.nodes[node].tag.as_deref() == Some("code"))
                    .unwrap();
                for preformatted_code in [false, true] {
                    let markdown = Converter::new(Options {
                        preformatted_code,
                        ..Default::default()
                    })
                    .convert(&input, input.root);
                    let output = Tree::parse(&rendered_html(&markdown));
                    let codes: Vec<_> = descendants(&output, output.root)
                        .into_iter()
                        .filter(|&node| output.nodes[node].tag.as_deref() == Some("code"))
                        .collect();
                    assert_eq!(codes.len(), 1, "{source}: {markdown}");
                    assert_eq!(
                        text(&output, codes[0]),
                        text(&input, original_code),
                        "preformatted_code={preformatted_code}: {source}: {markdown}"
                    );
                    assert_eq!(
                        with_attribute(&output, "href", "/symbol").len(),
                        1,
                        "{markdown}"
                    );
                }
            }
        }
    }
}

#[test]
fn block_html_fallback_preserves_the_enclosing_author_link() {
    for block in [
        "<table><tr><td>Product</td><td>Other</td></tr></table>",
        "<table><tr><td colspan='2'>Product</td></tr></table>",
        "<table><tr><th scope='col'>Product<div>Details</div></th></tr></table>",
        "<table><tr><td colspan='2'>Product<video src='/preview.mp4'></video></td></tr></table>",
        "<table><tr><td colspan='2'><table><tr><td>Product</td></tr></table></td></tr></table>",
        "<pre id='source'><code>Product<span> </span>\n\n[](*x*)\n</code></pre>",
        "<h2><div>Product</div></h2>",
    ] {
        for surrounding_text in [false, true] {
            let (before, after) = if surrounding_text {
                ("Before", "After")
            } else {
                ("", "")
            };
            let source = format!(
                "<a href='/product?q=1&amp;x=2' title='A &quot;product&quot; &amp; more'>{before}{block}{after}</a><a href='/next'>Next</a><a href='#source'>Jump</a>"
            );
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let links = with_attribute(&output, "href", "/product?q=1&x=2");
            assert!(!links.is_empty(), "{source}: {markdown}");
            let linked_text: String = links.iter().map(|&node| text(&output, node)).collect();
            assert!(linked_text.contains("Product"), "{source}: {markdown}");
            assert_eq!(
                linked_text.matches("Before").count(),
                usize::from(surrounding_text),
                "{markdown}"
            );
            assert_eq!(
                linked_text.matches("After").count(),
                usize::from(surrounding_text),
                "{markdown}"
            );
            for &link in &links {
                assert!(!text(&output, link).trim().is_empty(), "{markdown}");
                assert_eq!(
                    output.attribute(link, "title"),
                    Some("A \"product\" & more"),
                    "{markdown}"
                );
            }
            assert!(
                with_attribute(&output, "href", "/preview.mp4").is_empty(),
                "{markdown}"
            );
            let next = with_attribute(&output, "href", "/next");
            assert_eq!(next.len(), 1, "{markdown}");
            assert_eq!(text(&output, next[0]), "Next", "{markdown}");
            if block.starts_with("<pre") {
                let original = descendants(&input, input.root)
                    .into_iter()
                    .find(|&node| input.nodes[node].tag.as_deref() == Some("code"))
                    .unwrap();
                let codes: Vec<_> = descendants(&output, output.root)
                    .into_iter()
                    .filter(|&node| output.nodes[node].tag.as_deref() == Some("code"))
                    .collect();
                assert_eq!(codes.len(), 1, "{markdown}");
                assert_eq!(
                    text(&output, codes[0]),
                    text(&input, original),
                    "{markdown}"
                );
            }
        }
    }
}

#[test]
fn strike_elements_retain_their_deletion_semantics_in_every_output_path() {
    for tag in ["strike", "s", "del"] {
        for source in containers(&format!("<{tag}>100</{tag}> 50")) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let deletions: Vec<_> = descendants(&output, output.root)
                .into_iter()
                .filter(|&node| {
                    matches!(
                        output.nodes[node].tag.as_deref(),
                        Some("strike" | "s" | "del")
                    )
                })
                .collect();
            assert_eq!(deletions.len(), 1, "{source}: {markdown}");
            assert_eq!(text(&output, deletions[0]), "100", "{markdown}");
            assert!(text(&output, output.root).contains("100 50"), "{markdown}");
        }
    }
}

#[test]
fn expanded_svg_links_preserve_targets_inside_the_author_link() {
    for label in ["", "Inner"] {
        let content = format!(
            "<a href='/outer'><svg><a id='inner' href='/inner'><text>{label}</text></a></svg>\
             Outer<video src='/preview.mp4'></video></a>\
             <a href='#inner'>Jump</a><a href='/after'>After</a>"
        );
        for source in containers(&content) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let targets = with_attribute(&output, "id", "inner");
            assert_eq!(targets.len(), 1, "{source}: {markdown}");
            let links = with_attribute(&output, "href", "/outer");
            assert_eq!(links.len(), 1, "{markdown}");
            assert!(
                descendants(&output, links[0]).contains(&targets[0]),
                "{markdown}"
            );
            let body = text(&output, links[0]);
            assert!(
                body.contains("Outer") && body.contains("/preview.mp4"),
                "{markdown}"
            );
            assert!(body.contains(label), "{markdown}");
            assert!(
                with_attribute(&output, "href", "/inner").is_empty(),
                "{markdown}"
            );
            assert!(
                with_attribute(&output, "href", "/preview.mp4").is_empty(),
                "{markdown}"
            );
            let after = with_attribute(&output, "href", "/after");
            assert_eq!(after.len(), 1, "{markdown}");
            assert_eq!(text(&output, after[0]), "After", "{markdown}");
        }
    }
}

#[test]
fn expanded_presentational_columns_preserve_referenced_targets() {
    let content = "<a href='#columns'>Columns</a><a href='#column'>Column</a>\
        <table role='presentation'><colgroup id='columns'><col id='column'></colgroup>\
        <tr><td>A</td><td>B</td></tr></table>";
    for source in containers(content) {
        let input = Tree::parse(&source);
        let markdown = convert(&input, input.root);
        let output = Tree::parse(&rendered_html(&markdown));
        for id in ["columns", "column"] {
            assert_eq!(
                with_attribute(&output, "id", id).len(),
                1,
                "{source}: {markdown}"
            );
        }
        assert!(
            !output
                .nodes
                .iter()
                .any(|node| matches!(node.tag.as_deref(), Some("col" | "colgroup"))),
            "{markdown}"
        );
    }
}

#[test]
fn generated_html_code_content_preserves_literal_labels_and_link_ownership() {
    for label in [
        "[API](/wrong)",
        "**literal**",
        "`literal`",
        "![image](/wrong)",
    ] {
        for tag in ["video", "audio", "iframe", "img"] {
            for pre in [false, true] {
                let resource = if tag == "img" {
                    format!("<img src='javascript:invalid' alt='{label}'>")
                } else {
                    format!("<{tag} src='/resource' title='{label}'></{tag}>")
                };
                let code = format!("<code id='snippet'>x{resource}y</code>");
                let code = if pre {
                    format!("<pre>{code}</pre>")
                } else {
                    code
                };
                for source in containers(&format!("{code}<a href='#snippet'>Jump</a>")) {
                    let input = Tree::parse(&source);
                    let markdown = convert(&input, input.root);
                    let output = Tree::parse(&rendered_html(&markdown));
                    let codes: Vec<_> = descendants(&output, output.root)
                        .into_iter()
                        .filter(|&node| output.nodes[node].tag.as_deref() == Some("code"))
                        .collect();
                    assert_eq!(codes.len(), 1, "{source}: {markdown}");
                    assert!(
                        text(&output, codes[0]).contains(label),
                        "{source}: {markdown}"
                    );
                    assert!(
                        with_attribute(&output, "href", "/wrong").is_empty(),
                        "{markdown}"
                    );
                    assert!(
                        with_attribute(&output, "src", "/wrong").is_empty(),
                        "{markdown}"
                    );
                    if tag != "img" {
                        let links = with_attribute(&output, "href", "/resource");
                        assert_eq!(links.len(), 1, "{markdown}");
                        assert_eq!(text(&output, links[0]), label, "{markdown}");
                        assert!(
                            descendants(&output, codes[0]).contains(&links[0]),
                            "{markdown}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn linked_html_code_quotes_generated_labels_and_visible_resource_urls() {
    let label = "[API](/wrong)";
    let url = "/v[URL](/wrong).mp4";
    for tag in ["video", "audio", "iframe"] {
        let content = format!(
            "<a href='/outer'><code id='snippet'><{tag} src='{url}' aria-label='{label}'></{tag}></code>Watch</a>\
             <a href='#snippet'>Jump</a>"
        );
        for source in containers(&content) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let links = with_attribute(&output, "href", "/outer");
            assert_eq!(links.len(), 1, "{source}: {markdown}");
            let body = text(&output, links[0]);
            assert!(
                body.contains(label) && body.contains(url) && body.contains("Watch"),
                "{source}: {markdown}"
            );
            assert!(
                with_attribute(&output, "href", "/wrong").is_empty(),
                "{markdown}"
            );
            assert!(
                with_attribute(&output, "href", url).is_empty(),
                "{markdown}"
            );
            let targets = with_attribute(&output, "id", "snippet");
            assert_eq!(targets.len(), 1, "{markdown}");
            assert!(
                descendants(&output, links[0]).contains(&targets[0]),
                "{markdown}"
            );
        }
    }
}

#[test]
fn html_code_normalizes_line_endings_only_in_preformatted_inline_mode() {
    for (payload, preformatted_code) in [
        ("  a\n  b\n  c  ", false),
        ("  a\n  b\n  c  ", true),
        ("a&#13;&#10;b&#13;c&#10;d", true),
        ("a<span>&#13;</span><span>&#10;</span>b", true),
        ("a\n<span>  b\n</span>c", true),
        ("a\n<a href='/term'>  b\n</a>c", true),
    ] {
        for pre in [false, true] {
            let content = format!("<code id='snippet'>{payload}</code>");
            let content = if pre {
                format!("<pre>{content}</pre>")
            } else {
                content
            };
            for source in containers(&format!("{content}<a href='#snippet'>Jump</a>")) {
                let input = Tree::parse(&source);
                let original = descendants(&input, input.root)
                    .into_iter()
                    .find(|&node| input.nodes[node].tag.as_deref() == Some("code"))
                    .unwrap();
                let original_text = text(&input, original);
                let markdown = Converter::new(Options {
                    preformatted_code,
                    ..Default::default()
                })
                .convert(&input, input.root);
                let output = Tree::parse(&rendered_html(&markdown));
                let codes: Vec<_> = descendants(&output, output.root)
                    .into_iter()
                    .filter(|&node| output.nodes[node].tag.as_deref() == Some("code"))
                    .collect();
                assert_eq!(codes.len(), 1, "{source}: {markdown}");
                let expected = if preformatted_code && !pre {
                    original_text
                        .replace("\r\n", " ")
                        .replace(['\r', '\n'], " ")
                } else {
                    original_text.clone()
                };
                assert_eq!(
                    text(&output, codes[0]),
                    expected,
                    "preformatted_code={preformatted_code}: {source}: {markdown}"
                );
            }
        }
    }
    let converter = Converter::new(Options {
        preformatted_code: true,
        ..Default::default()
    });
    for reference in ["", "<a href='#snippet'>Jump</a>"] {
        let input = Tree::parse(&format!(
            "<code id='snippet'>  a\n<span>  b&#13;</span><span>&#10;c  </span></code>{reference}"
        ));
        let markdown = converter.convert(&input, input.root);
        let output = Tree::parse(&rendered_html(&markdown));
        let code = descendants(&output, output.root)
            .into_iter()
            .find(|&node| output.nodes[node].tag.as_deref() == Some("code"))
            .unwrap();
        assert_eq!(text(&output, code), "  a   b c  ", "{markdown}");
    }
    for source in containers(
        "<code id='first'>a&#13;</code><code id='second'>&#10;b</code>\
         <a href='#first'>First</a><a href='#second'>Second</a>",
    ) {
        let input = Tree::parse(&source);
        let markdown = converter.convert(&input, input.root);
        let output = Tree::parse(&rendered_html(&markdown));
        let codes: Vec<_> = descendants(&output, output.root)
            .into_iter()
            .filter(|&node| output.nodes[node].tag.as_deref() == Some("code"))
            .map(|node| text(&output, node))
            .collect();
        assert_eq!(codes, ["a ", " b"], "{source}: {markdown}");
    }
}

#[test]
fn pipe_target_ids_round_trip_without_table_specific_backslash_processing() {
    for (fragment, source_id, id) in [
        ("a%7Cb", "a|b", "a|b"),
        ("a%5C%7Cb", "a\\|b", "a\\|b"),
        ("a%26%23124%3Bb", "a&amp;#124;b", "a&#124;b"),
        ("%E8%8A%82%7C%E7%82%B9", "节|点", "节|点"),
    ] {
        let jump = format!("<a href='#{fragment}'>Go</a>");
        for target in [
            format!("<p id='{source_id}'>Target</p>"),
            format!("<table><tr><td id='{source_id}'>Target</td></tr></table>"),
            format!("<table><tr><td id='{source_id}' colspan='2'>Target</td></tr></table>"),
            format!(
                "<table><tr><td><code id='{source_id}'>Target</code></td><td>Other</td></tr></table>"
            ),
        ] {
            let input = Tree::parse(&format!("{jump}{target}"));
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            assert_eq!(with_attribute(&output, "id", id).len(), 1, "{markdown}");
            assert_eq!(
                with_attribute(&output, "href", &format!("#{fragment}")).len(),
                1,
                "{markdown}"
            );
            // Raw HTML attributes must also decode correctly before any
            // renderer-specific removal of GFM backslash escapes.
            let raw_html = Tree::parse(&markdown);
            assert_eq!(with_attribute(&raw_html, "id", id).len(), 1, "{markdown}");
            assert!(!markdown.contains("\\|"), "{markdown}");
        }
    }
}

#[test]
fn block_html_fallback_preserves_the_enclosing_formatting_stack() {
    let formats: &[(&str, &str, &[&str])] = &[
        ("<del>", "</del>", &["del"]),
        ("<s>", "</s>", &["del"]),
        ("<strike>", "</strike>", &["del"]),
        ("<strong>", "</strong>", &["strong"]),
        ("<b>", "</b>", &["strong"]),
        ("<em>", "</em>", &["em"]),
        ("<i>", "</i>", &["em"]),
        ("<sup>", "</sup>", &["sup"]),
        ("<sub>", "</sub>", &["sub"]),
        (
            "<del><strong><em>",
            "</em></strong></del>",
            &["del", "strong", "em"],
        ),
        ("<del><a href='/product'>", "</a></del>", &["del", "a"]),
        ("<a href='/product'><del>", "</del></a>", &["a", "del"]),
    ];
    for &(open, close, tags) in formats {
        for block in [
            "<table><tr><td>Product</td><td>Other</td></tr></table>",
            "<table><tr><td colspan='2'>Product</td></tr></table>",
            "<table><tr><td colspan='2'><table><tr><td>Product</td></tr></table></td></tr></table>",
            "<pre id='source'><code>Product\n\n[](*x*)\n</code></pre>",
            "<h2><div>Product</div></h2>",
        ] {
            for (before, after) in [("", ""), ("Before", "After")] {
                let source = format!(
                    "{open}{before}{block}{after}{close}<p>Outside</p><a href='#source'>Jump</a>"
                );
                let input = Tree::parse(&source);
                let markdown = convert(&input, input.root);
                let output = Tree::parse(&rendered_html(&markdown));
                let mut parent = output.root;
                for &tag in tags {
                    parent = descendants(&output, output.root)
                        .into_iter()
                        .find(|&node| {
                            output.nodes[node].tag.as_deref() == Some(tag)
                                && text(&output, node).contains("Product")
                        })
                        .unwrap_or_else(|| panic!("missing {tag}: {source}: {markdown}"));
                    assert!(!text(&output, parent).contains("Outside"), "{markdown}");
                }
                if tags.contains(&"a") {
                    let links = with_attribute(&output, "href", "/product");
                    assert!(
                        links
                            .iter()
                            .any(|&node| text(&output, node).contains("Product")),
                        "{markdown}"
                    );
                    assert!(
                        links
                            .iter()
                            .all(|&node| !text(&output, node).trim().is_empty()),
                        "{markdown}"
                    );
                }
                if block.starts_with("<pre") {
                    let code = descendants(&output, parent)
                        .into_iter()
                        .find(|&node| output.nodes[node].tag.as_deref() == Some("code"))
                        .unwrap();
                    assert_eq!(text(&output, code), "Product\n\n[](*x*)\n", "{markdown}");
                }
            }
        }
    }
}

#[test]
fn math_content_ends_code_crlf_continuity_but_transparent_markup_does_not() {
    for middle in [
        "<math><mi>x</mi></math>",
        "<span><math><mrow><mi>x</mi></mrow></math></span>",
        "<span><math><mi>x</mi></math><span aria-hidden='true'>x</span></span>",
        "<math><semantics><mi>x</mi><annotation>alternate</annotation></semantics></math>",
        "<span>x</span>",
    ] {
        for pre in [false, true] {
            let code = format!("<code id='snippet'>a&#13;{middle}&#10;b</code>");
            let code = if pre {
                format!("<pre>{code}</pre>")
            } else {
                code
            };
            for source in containers(&format!("{code}<a href='#snippet'>Jump</a>")) {
                let input = Tree::parse(&source);
                for preformatted_code in [false, true] {
                    let markdown = Converter::new(Options {
                        preformatted_code,
                        ..Default::default()
                    })
                    .convert(&input, input.root);
                    let output = Tree::parse(&rendered_html(&markdown));
                    let codes: Vec<_> = descendants(&output, output.root)
                        .into_iter()
                        .filter(|&node| output.nodes[node].tag.as_deref() == Some("code"))
                        .collect();
                    assert_eq!(codes.len(), 1, "{source}: {markdown}");
                    let actual = text(&output, codes[0]);
                    assert_eq!(
                        actual.split_whitespace().collect::<Vec<_>>(),
                        ["a", "x", "b"],
                        "preformatted_code={preformatted_code}: {source}: {markdown}"
                    );
                    if preformatted_code && !pre {
                        assert_eq!(actual, "a x b", "{markdown}");
                    }
                }
            }
        }
    }
    for middle in [
        "",
        "<span></span>",
        "<span><i></i></span>",
        "<span id='empty'></span>",
    ] {
        for source in containers(&format!(
            "<code id='snippet'>a&#13;{middle}&#10;b</code>\
             <a href='#snippet'>Jump</a><a href='#empty'>Empty</a>"
        )) {
            let input = Tree::parse(&source);
            let markdown = Converter::new(Options {
                preformatted_code: true,
                ..Default::default()
            })
            .convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let code = descendants(&output, output.root)
                .into_iter()
                .find(|&node| output.nodes[node].tag.as_deref() == Some("code"))
                .unwrap();
            assert_eq!(text(&output, code), "a b", "{source}: {markdown}");
        }
    }
}

#[test]
fn placeholder_anchors_keep_resource_links_and_referenced_targets() {
    let targets: &[(&str, &[&str])] = &[
        ("", &[]),
        ("name='clip'", &["clip"]),
        ("id='clip'", &["clip"]),
        ("id='clip' name='alias'", &["clip", "alias"]),
    ];
    for &(attributes, ids) in targets {
        for (href, author) in [
            ("", None),
            ("href=''", None),
            ("href='javascript:bad()'", None),
            ("href='java&#10;script:bad()'", None),
            ("href='data:text/html,blocked'", None),
            ("href='/watch'", Some("/watch")),
            ("href='app://watch'", Some("app://watch")),
        ] {
            for media in ["video", "audio", "iframe"] {
                let body = format!(
                    "<a {attributes} {href}><{media} src='/clip.mp4' title='Clip'></{media}>Watch</a>\
                     <{media} src='/after.mp4' title='After'></{media}>"
                );
                for wrapper in ["plain", "code", "pre"] {
                    let content = match wrapper {
                        "code" => format!("<code id='snippet'>{body}</code>"),
                        "pre" => format!("<pre id='snippet'>{body}</pre>"),
                        _ => body.clone(),
                    };
                    let jumps: String = ids
                        .iter()
                        .map(|id| format!("<a href='#{id}'>Jump</a>"))
                        .collect();
                    let content = format!("{content}{jumps}<a href='#snippet'>Snippet</a>");
                    for source in containers(&content) {
                        let input = Tree::parse(&source);
                        let markdown = convert(&input, input.root);
                        let output = Tree::parse(&rendered_html(&markdown));
                        let resources = with_attribute(&output, "href", "/clip.mp4");
                        if let Some(author) = author {
                            let links = with_attribute(&output, "href", author);
                            let body: String =
                                links.iter().map(|&node| text(&output, node)).collect();
                            assert!(
                                body.contains("Clip")
                                    && body.contains("/clip.mp4")
                                    && body.contains("Watch"),
                                "{source}: {markdown}"
                            );
                            assert!(!body.contains("After"), "{markdown}");
                            assert!(resources.is_empty(), "{markdown}");
                        } else {
                            assert_eq!(resources.len(), 1, "{source}: {markdown}");
                            assert_eq!(text(&output, resources[0]), "Clip", "{markdown}");
                        }
                        let after = with_attribute(&output, "href", "/after.mp4");
                        assert_eq!(after.len(), 1, "{source}: {markdown}");
                        assert_eq!(text(&output, after[0]), "After", "{markdown}");
                        for &id in ids {
                            let targets = with_attribute(&output, "id", id);
                            assert_eq!(targets.len(), 1, "{source}: {markdown}");
                            if author.is_none() {
                                assert_eq!(
                                    output.nodes[targets[0]].tag.as_deref(),
                                    Some("span"),
                                    "{markdown}"
                                );
                            }
                        }
                        for node in descendants(&output, output.root) {
                            if output.nodes[node].tag.as_deref() == Some("a") {
                                assert!(
                                    output
                                        .attribute(node, "href")
                                        .is_some_and(|href| !href.is_empty()),
                                    "{source}: {markdown}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn expanded_anchors_preserve_inherited_author_link_context() {
    for href in ["", "href=''", "href='javascript:bad()'", "href='/inner'"] {
        let content = format!(
            "<a href='/watch'><code id='snippet'><svg><a id='clip' {href}></a></svg>\
             <video src='/clip.mp4' title='Clip'></video>Watch</code></a>\
             <a href='#clip'>Jump</a><a href='#snippet'>Snippet</a>"
        );
        for source in containers(&content) {
            let input = Tree::parse(&source);
            let markdown = convert(&input, input.root);
            let output = Tree::parse(&rendered_html(&markdown));
            let links = with_attribute(&output, "href", "/watch");
            assert_eq!(links.len(), 1, "{source}: {markdown}");
            let body = text(&output, links[0]);
            assert!(
                body.contains("Clip") && body.contains("/clip.mp4") && body.contains("Watch"),
                "{source}: {markdown}"
            );
            for id in ["clip", "snippet"] {
                let targets = with_attribute(&output, "id", id);
                assert_eq!(targets.len(), 1, "{markdown}");
                assert!(
                    descendants(&output, links[0]).contains(&targets[0]),
                    "{markdown}"
                );
            }
            assert!(
                with_attribute(&output, "href", "/clip.mp4").is_empty(),
                "{markdown}"
            );
            assert!(
                with_attribute(&output, "href", "/inner").is_empty(),
                "{markdown}"
            );
        }
    }
}
