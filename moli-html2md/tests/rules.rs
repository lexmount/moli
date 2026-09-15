mod support;

use moli_html2md::{Converter, Options};
use support::{Tree, rendered_html};

fn markdown_with(html: &str, options: Options) -> String {
    let dom = Tree::parse(html);
    let before = dom.clone();
    let result = Converter::new(options).convert(&dom, dom.root);
    assert_eq!(dom, before);
    result
}

fn markdown(html: &str) -> String {
    markdown_with(html, Options::default())
}

#[test]
fn ordered_lists_use_start_and_sequential_numbers() {
    for (attributes, start) in [
        ("", 1),
        ("reversed", 1),
        ("start='4'", 4),
        ("reversed start='4'", 4),
    ] {
        let html = format!(
            "<ol {attributes}><li value='20'>first</li><li value='-3'>second</li><li>third</li></ol>"
        );
        let result = markdown(&html);
        assert_eq!(
            result,
            format!(
                "{start}. first\n{}. second\n{}. third",
                start + 1,
                start + 2
            )
        );
        let start_attribute = if start == 1 {
            String::new()
        } else {
            format!(" start=\"{start}\"")
        };
        assert_eq!(
            rendered_html(&result),
            format!(
                "<ol{start_attribute}>\n<li>first</li>\n<li>second</li>\n<li>third</li>\n</ol>\n"
            )
        );
    }
}

#[test]
fn nested_lists_keep_independent_start_numbers() {
    assert_eq!(
        markdown(
            "<ol start='4'><li>outer<ol reversed start='9'><li value='100'>inner</li><li>next</li></ol></li><li value='70'>tail</li></ol>"
        ),
        "4. outer\n   9. inner\n   10. next\n5. tail"
    );
}

#[test]
fn code_languages_preserve_punctuation_and_unicode() {
    for language in ["text/x-rust", "中文", "C#", "objective-c++", "foo:bar"] {
        let html = format!("<pre><code class='language-{language}'>x</code></pre>");
        let result = markdown(&html);
        assert_eq!(result, format!("```{language}\nx\n```"));
        assert_eq!(
            rendered_html(&result),
            format!("<pre><code class=\"language-{language}\">x\n</code></pre>\n")
        );
    }
}

#[test]
fn code_languages_preserve_literal_backslashes_and_entities() {
    for (language, expected) in [
        ("f\\*oo", "f\\*oo"),
        ("literal&amp;copy;", "literal&amp;copy;"),
    ] {
        let result = markdown(&format!(
            "<pre><code class='language-{language}'>x</code></pre>"
        ));
        assert_eq!(
            rendered_html(&result),
            format!("<pre><code class=\"language-{expected}\">x\n</code></pre>\n")
        );
    }
}

#[test]
fn language_backticks_use_a_tilde_fence_that_contains_the_entire_code() {
    let result = markdown("<pre><code class='language-a`b'>first\n~~~\nlast\n</code></pre>");
    assert_eq!(result, "~~~~a`b\nfirst\n~~~\nlast\n~~~~");
    assert_eq!(
        rendered_html(&result),
        "<pre><code class=\"language-a`b\">first\n~~~\nlast\n</code></pre>\n"
    );
}

#[test]
fn fallback_language_stays_on_the_fence_line() {
    for ending in ["\n", "\r\n", "\r"] {
        let result = markdown_with(
            "<pre>x</pre>",
            Options {
                default_code_language: Some(format!("text/x-rust{ending}```{ending}outside")),
                ..Options::default()
            },
        );
        assert_eq!(result, "```text/x-rust\nx\n```");
    }
}

#[test]
fn list_spacing_uses_converted_blocks_including_wrapped_blocks() {
    for (body, expected) in [
        ("<h2>Heading</h2>", "- ## Heading\n\n- tail"),
        ("intro<hr>", "- intro\n\n  ---\n\n- tail"),
        ("<section>one</section>", "- one\n\n- tail"),
        ("<span><p>one</p></span>", "- one\n\n- tail"),
    ] {
        let result = markdown(&format!("<ul><li>{body}</li><li>tail</li></ul>"));
        assert_eq!(result, expected, "{body}");
        assert!(rendered_html(&result).contains("<p>tail</p>"), "{body}");
    }
}

#[test]
fn nested_list_blocks_do_not_make_outer_items_loose() {
    for (inner, expected) in [
        (
            "<li>one</li><li>two</li>",
            "- outer\n  - one\n  - two\n- tail",
        ),
        (
            "<li><p>one</p></li><li><p>two</p></li>",
            "- outer\n  - one\n\n  - two\n- tail",
        ),
    ] {
        let result = markdown(&format!(
            "<ul><li>outer<ul>{inner}</ul></li><li>tail</li></ul>"
        ));
        assert_eq!(result, expected);
        assert!(rendered_html(&result).contains("<li>tail</li>"));
    }
    assert_eq!(
        markdown("<ul><li><p>outer</p><ul><li>inner</li></ul></li><li>tail</li></ul>"),
        "- outer\n\n  - inner\n\n- tail"
    );
}

#[test]
fn omitted_subtrees_do_not_affect_list_spacing() {
    let html = "<ul><li>one<span><h2>too deep</h2></span></li><li>two</li></ul>";
    assert_eq!(
        markdown_with(
            html,
            Options {
                max_depth: 4,
                ..Options::default()
            }
        ),
        "- one\n- two"
    );
    assert_eq!(
        markdown("<ul><li>one<script>ignored</script></li><li>two</li></ul>"),
        "- one\n- two"
    );
}

#[test]
fn emphasis_uses_markdown_between_adjacent_links() {
    for (tag, marker, html_tag) in [
        ("b", "**", "strong"),
        ("em", "*", "em"),
        ("del", "~~", "del"),
    ] {
        for label in ["Hacker News", "News!"] {
            let result = markdown(&format!(
                "<{tag}><a href='news'>{label}</a></{tag}><a href='newest'>new</a>"
            ));
            assert_eq!(
                result,
                format!("{marker}[{label}](news){marker}[new](newest)")
            );
            assert_eq!(
                rendered_html(&result),
                format!(
                    "<p><{html_tag}><a href=\"news\">{label}</a></{html_tag}><a href=\"newest\">new</a></p>\n"
                )
            );
        }
    }
    assert_eq!(
        markdown("<strong>News!</strong><a href='newest'>new</a>"),
        "**News!**[new](newest)"
    );
}

#[test]
fn emphasized_links_keep_html_at_intraword_boundaries() {
    for tag in ["strong", "em"] {
        for suffix in [" after", "after", "<a href='/next'>next</a>"] {
            let result = markdown(&format!(
                "before<{tag}><a href='/'>text</a></{tag}>{suffix}"
            ));
            let suffix_markdown = if suffix.starts_with('<') {
                "[next](/next)"
            } else {
                suffix
            };
            let suffix_html = if suffix.starts_with('<') {
                "<a href=\"/next\">next</a>"
            } else {
                suffix
            };
            assert_eq!(
                result,
                format!("before<{tag}>[text](/)</{tag}>{suffix_markdown}")
            );
            assert_eq!(
                rendered_html(&result),
                format!("<p>before<{tag}><a href=\"/\">text</a></{tag}>{suffix_html}</p>\n")
            );
        }
        let result = markdown(&format!("<{tag}><a href='/'>text</a></{tag}>after"));
        assert_eq!(result, format!("<{tag}>[text](/)</{tag}>after"));
    }
}
