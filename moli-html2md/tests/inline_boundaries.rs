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
fn inline_code_breaks_separate_words_without_formatting_markers() {
    for html in [
        "<code>git<br>status</code>",
        "<code><strong>git<br></strong><em>status</em></code>",
    ] {
        for preformatted_code in [false, true] {
            let result = markdown_with(
                html,
                Options {
                    preformatted_code,
                    ..Options::default()
                },
            );
            assert_eq!(result, "`git status`", "{html}");
            assert_eq!(
                rendered_html(&result),
                "<p><code>git status</code></p>\n",
                "{html}"
            );
        }
    }
}

#[test]
fn inline_code_breaks_follow_the_selected_whitespace_mode() {
    let html = "<code>git<br><br>  status</code>";
    assert_eq!(markdown(html), "`git status`");
    let result = markdown_with(
        html,
        Options {
            preformatted_code: true,
            ..Options::default()
        },
    );
    assert_eq!(result, "`git    status`");
    assert_eq!(
        rendered_html(&result),
        "<p><code>git    status</code></p>\n"
    );
}

#[test]
fn inline_code_breaks_keep_separation_at_code_span_edges() {
    for (html, expected) in [
        (
            "before<code>git<br></code><code>status</code>after",
            "before`git` `status`after",
        ),
        ("before<code><br>git<br></code>after", "before `git` after"),
    ] {
        assert_eq!(markdown(html), expected, "{html}");
    }
    let result = markdown_with(
        "before<code>git<br></code><code>status</code>after",
        Options {
            preformatted_code: true,
            ..Options::default()
        },
    );
    assert_eq!(
        rendered_html(&result),
        "<p>before<code>git </code><code>status</code>after</p>\n"
    );
}

#[test]
fn inline_code_breaks_obey_the_depth_limit() {
    let html = "<code>git<span><br></span>status</code>";
    for (max_depth, expected) in [(3, "`gitstatus`"), (4, "`git status`")] {
        assert_eq!(
            markdown_with(
                html,
                Options {
                    max_depth,
                    ..Options::default()
                },
            ),
            expected
        );
    }
}

#[test]
fn pre_keeps_raw_text_and_line_endings() {
    let result = markdown("<pre><code><b>git</b>\n  <em>status</em></code></pre>");
    assert_eq!(result, "```\ngit\n  status\n```");
    assert_eq!(
        rendered_html(&result),
        "<pre><code>git\n  status\n</code></pre>\n"
    );
}

#[test]
fn preformatted_examples_keep_inline_values_and_separate_display_lines() {
    for (source, expected) in [
        (
            "<pre>const answer = <output>42</output>;</pre>",
            "const answer = 42;",
        ),
        (
            "<pre><h3>TypeScript</h3><code>function identity() {}</code></pre>",
            "TypeScript\nfunction identity() {}",
        ),
        (
            "<pre><ol><li>first()</li><li>second()</li></ol></pre>",
            "first()\nsecond()",
        ),
        ("<pre>first<br>second</pre>", "first\nsecond"),
    ] {
        let result = markdown(source);
        assert_eq!(
            rendered_html(&result),
            format!("<pre><code>{expected}\n</code></pre>\n"),
            "{source}: {result}"
        );
    }
}

#[test]
fn empty_wrappers_do_not_add_lines_to_preformatted_examples() {
    for (source, expected) in [
        ("<pre><div><!-- note --></div>print(42)</pre>", "print(42)"),
        (
            "<pre><section><div></div></section>print(42)</pre>",
            "print(42)",
        ),
        (
            "<pre>first<div>second</div>third</pre>",
            "first\nsecond\nthird",
        ),
        ("<pre><span>\n</span>print(42)</pre>", "\nprint(42)"),
    ] {
        let result = markdown(source);
        assert_eq!(
            rendered_html(&result),
            format!("<pre><code>{expected}\n</code></pre>\n"),
            "{source}: {result}"
        );
    }
}

#[test]
fn nested_emphasis_closes_with_markdown_before_a_word() {
    for (html, expected, expected_html) in [
        (
            "<em><strong>x</strong></em>y",
            "***x***y",
            "<p><em><strong>x</strong></em>y</p>\n",
        ),
        (
            "<strong><em>x</em></strong>y",
            "***x***y",
            "<p><em><strong>x</strong></em>y</p>\n",
        ),
        (
            "a<strong>b<em>c</em></strong>d",
            "a**b*c***d",
            "<p>a<strong>b<em>c</em></strong>d</p>\n",
        ),
        (
            "a<em>b<strong>c</strong></em>d",
            "a*b**c***d",
            "<p>a<em>b<strong>c</strong></em>d</p>\n",
        ),
        (
            "<strong><em>x</em>y</strong>z",
            "***x*y**z",
            "<p><strong><em>x</em>y</strong>z</p>\n",
        ),
        (
            "<em><strong>x</strong>y</em>z",
            "***x**y*z",
            "<p><em><strong>x</strong>y</em>z</p>\n",
        ),
    ] {
        let result = markdown(html);
        assert_eq!(result, expected, "{html}");
        assert_eq!(rendered_html(&result), expected_html, "{html}");
    }
}

#[test]
fn combining_marks_and_format_characters_use_markdown_emphasis() {
    for character in ['\u{0301}', '\u{20dd}', '\u{200d}'] {
        for (tag, delimiter) in [("strong", "**"), ("em", "*"), ("del", "~~")] {
            for content in [character.to_string(), format!("x{character}")] {
                let source = format!("a<{tag}>{content}</{tag}>b");
                let result = markdown(&source);
                assert_eq!(
                    result,
                    format!("a{delimiter}{content}{delimiter}b"),
                    "{source}"
                );
                assert_eq!(rendered_html(&result), format!("<p>{source}</p>\n"));
            }
        }
    }
}

#[test]
fn unicode_non_punctuation_preserves_emphasis_on_both_boundaries() {
    for character in ['\u{0301}', '\u{20dd}', '\u{200d}'] {
        for tag in ["em", "strong", "del"] {
            for source in [
                format!("<{tag}>!</{tag}>{character}"),
                format!("{character}<{tag}>!</{tag}>"),
                format!("{character}<{tag}>!</{tag}>{character}"),
                format!("{character}<{tag}><a href=\"/\">!</a></{tag}>{character}"),
            ] {
                let result = markdown(&source);
                assert_eq!(
                    rendered_html(&result),
                    format!("<p>{source}</p>\n"),
                    "{source}: {result}"
                );
            }
        }
    }
}

#[test]
fn unicode_punctuation_and_symbols_preserve_intraword_emphasis() {
    for character in ['。', '€', '🦀', '+'] {
        for tag in ["strong", "em", "del"] {
            for content in [character.to_string(), format!("x{character}")] {
                let source = format!("a<{tag}>{content}</{tag}>b");
                assert_eq!(
                    rendered_html(&markdown(&source)),
                    format!("<p>{source}</p>\n"),
                    "{source}"
                );
            }
        }
    }
}

#[test]
fn nested_emphasis_still_uses_html_after_literal_punctuation() {
    for text in ["word!", "word*", "word`", "word)", "日本語。"] {
        for (outer, inner) in [("em", "strong"), ("strong", "em")] {
            let html = format!("<{outer}><{inner}>{text}</{inner}></{outer}>after");
            let result = markdown(&html);
            assert!(result.contains(&format!("<{outer}>")), "{result}");
            assert_eq!(rendered_html(&result), format!("<p>{html}</p>\n"));
        }
    }
}

#[test]
fn closing_runs_stop_at_links_code_images_and_other_markers() {
    for (html, expected_html) in [
        (
            "<em><strong><a href='/'>x</a></strong></em>y",
            "<p><em><strong><a href=\"/\">x</a></strong></em>y</p>\n",
        ),
        (
            "<em><strong><code>x</code></strong></em>y",
            "<p><em><strong><code>x</code></strong></em>y</p>\n",
        ),
        (
            "<em><strong><img src='/x' alt='x'></strong></em>y",
            "<p><em><strong><img src=\"/x\" alt=\"x\" /></strong></em>y</p>\n",
        ),
        ("<em><del>x</del></em>y", "<p><em><del>x</del></em>y</p>\n"),
        ("<del><em>x</em></del>y", "<p><del><em>x</em></del>y</p>\n"),
    ] {
        assert_eq!(rendered_html(&markdown(html)), expected_html, "{html}");
    }
}

#[test]
fn separate_nested_spans_keep_html_when_delimiters_are_ambiguous() {
    let result = markdown("<em><strong>a</strong>b<strong>c</strong></em>d");
    assert_eq!(result, "<em>**a**b**c**</em>d");
    assert_eq!(
        rendered_html(&result),
        "<p><em><strong>a</strong>b<strong>c</strong></em>d</p>\n"
    );
    for text in ["x", "x!", "x*"] {
        for (outer, inner) in [("em", "strong"), ("strong", "em")] {
            let html = format!(
                "before<{outer}><{inner}>{text}</{inner}>b<{inner}>c</{inner}></{outer}>after"
            );
            assert_eq!(
                rendered_html(&markdown(&html)),
                format!("<p>{html}</p>\n"),
                "{html}"
            );
        }
    }
}

#[test]
fn multiline_image_alt_preserves_the_image() {
    let result = markdown("<img src='/x' alt='first\n# heading'>");
    // Alt newlines must not open a heading; the image and its text alternative survive.
    let actual_html = rendered_html(&result);
    let desired_html = "<p><img src=\"/x\" alt=\"first # heading\" /></p>\n";
    assert_eq!(
        actual_html, desired_html,
        "image alt survives Markdown rendering"
    );
}

#[test]
fn multiline_link_title_preserves_the_link() {
    let result = markdown("<a href='/' title='first\n# heading'>link</a>");
    // Keep the link intact while normalizing its title to one readable line.
    let actual_html = rendered_html(&result);
    let desired_html = "<p><a href=\"/\" title=\"first # heading\">link</a></p>\n";
    assert_eq!(
        actual_html, desired_html,
        "link title survives Markdown rendering"
    );
}

#[test]
fn multiple_nested_spans_keep_their_styles_at_paragraph_end() {
    // This guards the closing-edge fix: with no following character there
    // is no punctuation fallback, but sibling delimiters must not mispair.
    let result = markdown("<em><strong>x</strong>b<strong>c</strong></em>");
    let actual_html = rendered_html(&result);
    let desired_html = "<p><em><strong>x</strong>b<strong>c</strong></em></p>\n";
    assert_eq!(
        actual_html, desired_html,
        "nested sibling emphasis remains intact"
    );
}

#[test]
fn mixed_emphasis_markers_preserve_intraword_styles() {
    // The opening check sees the text 'x', although the nested styles emit
    // punctuation first. This is separate from closing a same-marker run.
    let result = markdown("before<em><del><strong>x</strong></del></em>");
    let actual_html = rendered_html(&result);
    let desired_html = "<p>before<em><del><strong>x</strong></del></em></p>\n";
    assert_eq!(actual_html, desired_html, "intraword styles remain intact");
}
