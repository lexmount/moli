mod support;

use moli_html2md::{Converter, Options, convert};
use support::{Tree, rendered_html};

fn markdown(html: &str) -> String {
    let dom = Tree::parse(html);
    let before = dom.clone();
    let result = convert(&dom, dom.root);
    assert_eq!(dom, before);
    result
}

#[test]
fn hacker_news_layout_tables_preserve_story_and_metadata_order() {
    let result = markdown(include_str!("fixtures/hacker-news-layout.html"));
    assert_eq!(
        result,
        include_str!("fixtures/hacker-news-layout.md").trim_end()
    );
    let html = rendered_html(&result);
    assert!(!html.contains("<table>"));
    assert!(!html.contains("<br>"));
    assert_eq!(html.matches("<a ").count(), 14);
}

#[test]
fn nested_tables_expand_cells_in_dom_order() {
    let html = "<table><tr><td>before</td></tr><tr><td><table><tr><th>Key</th><th>Value</th></tr><tr><td>a|b</td><td><code>c|d</code></td></tr></table></td></tr><tr><td>after</td></tr></table>";
    let actual = markdown(html);
    assert_eq!(actual, "before\n\nKey\n\nValue\n\na|b\n\n`c|d`\n\nafter");
    assert_eq!(
        rendered_html(&actual),
        "<p>before</p>\n<p>Key</p>\n<p>Value</p>\n<p>a|b</p>\n<p><code>c|d</code></p>\n<p>after</p>\n"
    );
}

#[test]
fn wrapping_a_table_preserves_its_content() {
    let data =
        "<table><tr><td>one</td><td>two</td></tr><tr><td>three</td><td>four</td></tr></table>";
    assert_eq!(
        markdown(&format!("<table><tr><td>{data}</td></tr></table>")),
        markdown(data)
    );
    assert_eq!(markdown(data), "one\n\ntwo\n\nthree\n\nfour");
}

#[test]
fn table_roles_do_not_change_conversion() {
    for role in [
        "",
        "presentation",
        "none",
        " PRESENTATION ",
        "table",
        "grid",
    ] {
        let html = format!(
            "<table role='{role}'><tr><th>Name</th><th>Value</th></tr><tr><td>one</td><td>two</td></tr></table>"
        );
        assert_eq!(markdown(&html), "Name\n\nValue\n\none\n\ntwo");
    }
}

#[test]
fn table_cells_preserve_block_syntax_and_code_whitespace() {
    let html = "<table><tr><td>intro</td><td><h2>Heading</h2><pre><code>  a\n  b\n</code></pre><blockquote>quote</blockquote><ul><li>item</li></ul></td><td>end</td></tr></table>";
    assert_eq!(
        markdown(html),
        "intro\n\n## Heading\n\n```\n  a\n  b\n```\n\n> quote\n\n- item\n\nend"
    );
}

#[test]
fn captions_sections_headers_and_spacer_rows_preserve_content_order() {
    for (html, expected) in [
        (
            "<table role='table'><tr><td>a</td><td>b</td></tr><tr></tr><tr><td>c</td><td>d</td></tr></table>",
            "a\n\nb\n\nc\n\nd",
        ),
        (
            "<table><caption>Data</caption><tr><td>a</td><td>b</td></tr><tr></tr><tr><td>c</td><td>d</td></tr></table>",
            "Data\n\na\n\nb\n\nc\n\nd",
        ),
        (
            "<table><thead><tr><td>a</td><td>b</td></tr></thead><tbody><tr></tr><tr><td>c</td><td>d</td></tr></tbody><tfoot><tr><td>end</td></tr></tfoot></table>",
            "a\n\nb\n\nc\n\nd\n\nend",
        ),
        (
            "<table><tr><th>a</th><th>b</th></tr><tr></tr><tr><td>c</td><td>d</td></tr></table>",
            "a\n\nb\n\nc\n\nd",
        ),
    ] {
        assert_eq!(markdown(html), expected, "{html}");
    }
}

#[test]
fn table_presentation_attributes_do_not_change_conversion() {
    for attributes in [
        "",
        "border='0' width='100%'",
        "border='1'",
        "style='border:0'",
    ] {
        let html = format!(
            "<table {attributes}><tr><td><a href='/'>Home</a></td><td><a href='/help'>Help</a></td></tr></table>"
        );
        assert_eq!(markdown(&html), "[Home](/)\n\n[Help](/help)");
    }
}

#[test]
fn nested_tables_obey_the_conversion_depth_limit() {
    let dom = Tree::parse(
        "<table><tr><td>visible<div><table><tr><td>too deep</td></tr></table></div></td></tr></table>",
    );
    for max_depth in [6, 7] {
        let converter = Converter::new(Options {
            max_depth,
            ..Options::default()
        });
        assert_eq!(converter.convert(&dom, dom.root), "visible");
    }
    assert_eq!(convert(&dom, dom.root), "visible\n\ntoo deep");
}
