mod support;

use moli_html2md::{Converter, Dom, NodeKind, Options};
use std::cell::Cell;
use support::{Tree, rendered_html};

fn markdown(html: &str, preformatted_code: bool) -> String {
    let dom = Tree::parse(html);
    Converter::new(Options {
        preformatted_code,
        ..Options::default()
    })
    .convert(&dom, dom.root)
}

#[test]
fn scientific_indices_keep_their_meaning_in_markdown() {
    let result = markdown(
        "<p>Water H<sub>2</sub>O; area x<sup>2</sup>; note<sup><a href='#n'>1</a></sup>.</p>",
        false,
    );
    assert!(result.contains("H<sub>2</sub>O"), "{result}");
    assert!(result.contains("x<sup>2</sup>"), "{result}");
    let html = rendered_html(&result);
    assert!(html.contains("<sup><a href=\"#n\">1</a></sup>"), "{html}");
}

#[test]
fn tex_math_keeps_commands_indices_and_matrix_separators() {
    let source = r"<p>Model $x_i^2 + \alpha$ and $$\begin{matrix} a &amp; b \\ c &amp; d \end{matrix}$$.</p>";
    let result = markdown(source, false);
    assert!(result.contains(r"$x_i^2 + \alpha$"), "{result}");
    assert!(
        result.contains(r"$$\begin{matrix} a & b \\ c & d \end{matrix}$$"),
        "{result}"
    );
}

#[test]
fn math_recognition_does_not_reinterpret_prices_or_literal_code() {
    let result = markdown(
        r"<p>Pay $5 for a_b and $10 for c_d; <code>$x_i$</code>; $unfinished_name</p>",
        false,
    );
    assert!(result.contains(r"$5 for a\_b and $10 for c\_d"), "{result}");
    assert!(result.contains("`$x_i$`"), "{result}");
    assert!(result.contains(r"$unfinished\_name"), "{result}");
    let result = markdown(r"<p>括号 \(x_i + \alpha\)；方括号 \[y_1=2\]。</p>", false);
    assert!(result.contains(r"\(x_i + \alpha\)"), "{result}");
    assert!(result.contains(r"\[y_1=2\]"), "{result}");
}

#[test]
fn dollar_wrapped_literal_html_remains_inert_without_a_math_extension() {
    let result = markdown(r"<p>$&lt;img src=x onerror=alert(1)&gt;_i$</p>", false);
    let html = rendered_html(&result);
    assert!(!html.contains("<img"), "{html}");
    assert!(html.contains("&lt;img"), "{html}");
}

#[test]
fn tex_payload_is_usable_by_a_markdown_math_reader() {
    use pulldown_cmark::{Event, Options as MarkdownOptions, Parser};
    let result = markdown(r"<p>$x_i + \alpha$</p>", false);
    let payloads: Vec<_> = Parser::new_ext(&result, MarkdownOptions::ENABLE_MATH)
        .filter_map(|event| match event {
            Event::InlineMath(text) => Some(text.into_string()),
            _ => None,
        })
        .collect();
    assert_eq!(payloads, [r"x_i + \alpha"]);
}

#[test]
fn transparent_inline_wrappers_do_not_change_tex_or_activate_markdown_resources() {
    use pulldown_cmark::{Event, Options as MarkdownOptions, Parser};
    for source in [
        r"<p>$x_i<span> + </span>\alpha$</p>",
        r"<p>$x_i<!--note--> + \alpha$</p>",
    ] {
        let result = markdown(source, false);
        let payloads: Vec<_> = Parser::new_ext(&result, MarkdownOptions::ENABLE_MATH)
            .filter_map(|event| match event {
                Event::InlineMath(text) => Some(text.into_string()),
                _ => None,
            })
            .collect();
        assert_eq!(payloads, [r"x_i + \alpha"], "{result}");
    }
    for source in [
        "$[download](https://example.org/file)$",
        "$![diagram](https://example.org/pixel)$",
    ] {
        let result = rendered_html(&markdown(&format!("<p>{source}</p>"), false));
        assert!(!result.contains("<a "), "{result}");
        assert!(!result.contains("<img "), "{result}");
    }
}

#[test]
fn math_streams_across_transparent_wrappers_in_every_inline_container() {
    use pulldown_cmark::{Event, Options as MarkdownOptions, Parser};
    for source in [
        r"<p>Model $x_i<span> + </span>\alpha$.</p>",
        r"<h2>Model $x_i<span> + </span>\alpha$.</h2>",
        r"<ul><li>Model $x_i<span> + </span>\alpha$.</li></ul>",
        r"<table><tr><td>Model $x_i<span> + </span>\alpha$.</td></tr></table>",
    ] {
        let result = markdown(source, false);
        let payloads: Vec<_> = Parser::new_ext(&result, MarkdownOptions::ENABLE_MATH)
            .filter_map(|event| match event {
                Event::InlineMath(text) => Some(text.into_string()),
                _ => None,
            })
            .collect();
        assert_eq!(payloads, [r"x_i + \alpha"], "{source}: {result}");
    }
    let result = markdown(
        r"<a href='#equation'>Equation</a><p id='equation'>$x<span> + </span>y$</p><p>$z<span> + </span>1$</p>",
        false,
    );
    assert!(result.contains("<a id=\"equation\"></a>"), "{result}");
    assert!(result.contains("$x + y$\n\n$z + 1$"), "{result}");
}

#[test]
fn mathematical_structures_survive_markdown_rendering() {
    let source = "<p>Rate <math><mfrac><mi>dQ</mi><mi>dt</mi></mfrac><mo>=</mo><msup><mi>x</mi><mn>2</mn></msup><mo>+</mo><msqrt><mi>y</mi></msqrt></math>.</p>";
    let result = markdown(source, false);
    let html = rendered_html(&result);
    for structure in [
        "<mfrac><mi>dQ</mi><mi>dt</mi></mfrac>",
        "<msup><mi>x</mi><mn>2</mn></msup>",
        "<msqrt><mi>y</mi></msqrt>",
    ] {
        assert!(html.contains(structure), "{html}");
    }
}

#[test]
fn nested_powers_retain_each_level() {
    let result = markdown(
        "<p>x<sup>y<sup>2</sup></sup> + a<sub>b<sub>i</sub></sub></p>",
        false,
    );
    assert!(result.contains("x<sup>y<sup>2</sup></sup>"), "{result}");
    assert!(result.contains("a<sub>b<sub>i</sub></sub>"), "{result}");
}

#[test]
fn body_metadata_does_not_become_article_text() {
    let result = markdown(
        "<article><h1>News</h1><p>Actual story</p></article><footer><title>Untitled template</title>Publisher</footer>",
        false,
    );
    assert!(result.contains("Actual story"));
    assert!(result.contains("Publisher"));
    assert!(!result.contains("Untitled template"), "{result}");
    let graphic = markdown(
        "<svg><title>Chart description</title><text>Revenue</text></svg><title>Template metadata</title>",
        false,
    );
    assert!(graphic.contains("Chart description"), "{graphic}");
    assert!(graphic.contains("Revenue"), "{graphic}");
    assert!(!graphic.contains("Template metadata"), "{graphic}");
}

#[test]
fn empty_layout_tables_do_not_invent_data() {
    let result = markdown(
        "<p>Article</p><table><caption>Video unavailable</caption><tr><td><object data='video.swf'></object></td></tr></table><table><tr><th>Metric</th><th>Value</th></tr><tr><td>Rate</td><td></td></tr></table>",
        false,
    );
    assert!(result.contains("Video unavailable"), "{result}");
    assert!(!result.contains("|  |\n| --- |\n|  |"), "{result}");
    assert!(result.contains("| Rate |  |"), "{result}");
}

#[test]
fn form_choices_keep_option_and_group_boundaries() {
    let result = markdown(
        "<label>Period<select><option>All posts</option><optgroup label='Recent'><option>1 day</option><option label='1 week'>Internal long label</option></optgroup></select></label>",
        false,
    );
    assert!(result.contains("All posts\n"), "{result}");
    assert!(result.contains("Recent\n1 day\n1 week"), "{result}");
    assert!(!result.contains("Internal long label"), "{result}");
    assert_eq!(markdown("<p>web<span>site</span></p>", false), "website");
}

#[test]
fn embedded_media_retains_usable_sources() {
    let result = markdown(
        "<p>Watch the interview:</p><iframe src='https://video.example/embed/42' title='Interview'></iframe><video poster='/cover.jpg'><source src='/movie.webm'><source src='/movie.mp4'><source src='/movie.mp4'></video><audio src='/episode.ogg' title='Episode'></audio><p>End.</p>",
        false,
    );
    for link in [
        "[Interview](https://video.example/embed/42)",
        "(/movie.webm)",
        "(/movie.mp4)",
        "[Episode](/episode.ogg)",
        "(/cover.jpg)",
    ] {
        assert!(result.contains(link), "{result}");
    }
    assert_eq!(result.matches("(/movie.mp4)").count(), 1, "{result}");
    assert!(result.ends_with("End."), "{result}");
}

#[test]
fn responsive_image_without_src_retains_largest_candidate() {
    assert_eq!(
        markdown(
            "<img alt='Course cover' src='' srcset='//cdn.example/cover-250.jpg 250w, //cdn.example/cover-500.jpg 500w'>",
            false,
        ),
        "![Course cover](//cdn.example/cover-500.jpg)"
    );
}

#[test]
fn concrete_loaded_image_wins_over_an_unexpanded_lazy_template() {
    let result = markdown(
        "<img alt='Basin' src='https://cdn.example/basin_300x.jpg' data-src='//cdn.example/basin_{width}x.jpg'>",
        false,
    );
    assert_eq!(result, "![Basin](https://cdn.example/basin_300x.jpg)");
}

#[test]
fn lazy_media_uses_declared_resources_instead_of_spacers() {
    let source = "<img src='/placeholder.gif' data-original='/photo-one.jpg' alt='One'><img src='/placeholder.gif' data-original='/photo-two.jpg' alt='Two'><iframe src='data:image/gif;base64,placeholder' data-src='//video.example/lecture' title='Lecture'></iframe><img src='/ordinary.jpg' data-src=' '>";
    let result = markdown(source, false);
    for target in [
        "(/photo-one.jpg)",
        "(/photo-two.jpg)",
        "(//video.example/lecture)",
        "(/ordinary.jpg)",
    ] {
        assert!(result.contains(target), "{result}");
    }
    assert!(!result.contains("placeholder"), "{result}");
    let table = markdown(
        "<table><tr><td colspan='2'><img src='/spacer.gif' data-src='/actual.jpg' alt='Photo'></td></tr></table>",
        false,
    );
    assert!(table.contains("src=\"/actual.jpg\""), "{table}");
    assert!(!table.contains("spacer.gif"), "{table}");
}

#[test]
fn merged_cells_keep_calendar_and_group_associations() {
    let source = "<table><tr><th>Mon</th><th>Tue</th><th>Wed</th></tr><tr><td colspan='2'></td><td>1</td></tr></table><table><tr><td rowspan='2'>Group</td><td>A</td></tr><tr><td>B</td></tr></table>";
    let result = markdown(source, false);
    let html = rendered_html(&result);
    assert!(html.contains("<td colspan=\"2\"></td><td>1</td>"), "{html}");
    assert!(
        html.contains("<td rowspan=\"2\">Group</td><td>A</td>"),
        "{html}"
    );
    assert_eq!(html.matches(">Group<").count(), 1);
}

#[test]
fn nested_tables_stay_with_their_column_labels() {
    let source = "<table><tr><th>Today</th><th>Month</th></tr><tr><td><table><tr><td>Alice</td><td>20</td></tr></table></td><td><table><tr><td>Bob</td><td>100</td></tr></table></td></tr></table>";
    let html = rendered_html(&markdown(source, false));
    assert!(html.contains("<th>Today</th><th>Month</th>"), "{html}");
    assert!(html.contains("<td><table>"), "{html}");
    assert!(html.contains("<td>Alice</td><td>20</td>"), "{html}");
    assert!(html.contains("<td>Bob</td><td>100</td>"), "{html}");
}

#[test]
fn complex_tables_keep_media_choices_and_literal_content_inert() {
    let source = "<table onclick='bad()'><tr><td colspan='2'><iframe src='/lecture' title='Lecture'></iframe><select><option label='Day'>long label</option><option>Night</option></select><pre>&lt;script&gt;literal&lt;/script&gt;\n\nline</pre><script>bad()</script><a href='javascript:bad()'>Bad link</a></td></tr></table>";
    let html = rendered_html(&markdown(source, false));
    assert!(html.contains("href=\"/lecture\">Lecture</a>"), "{html}");
    assert!(
        html.contains("<option label=\"Day\">long label</option><option>Night</option>"),
        "{html}"
    );
    assert!(
        html.contains("&lt;script&gt;literal&lt;/script&gt;"),
        "{html}"
    );
    assert!(!html.contains("onclick"), "{html}");
    assert!(!html.contains("<script>"), "{html}");
    assert!(!html.contains("javascript:"), "{html}");
}

#[test]
fn math_alternate_encodings_do_not_duplicate_the_equation() {
    let result = markdown(
        "<math><semantics><mrow><mn>1</mn><mo>/</mo><mn>72</mn></mrow><annotation encoding='application/x-tex'>1/72</annotation><annotation-xml encoding='text/html'><p>alternative</p></annotation-xml></semantics></math>",
        false,
    );
    assert!(!result.contains("annotation"), "{result}");
    assert!(!result.contains("alternative"), "{result}");
    assert_eq!(result.matches("72").count(), 1, "{result}");
}

#[test]
fn math_visual_alternatives_do_not_duplicate_semantic_math() {
    let source = "<p>Point is <span class='any-renderer'><span><math><mn>1</mn><mo>/</mo><mn>72</mn></math></span><span aria-hidden='true'>1/72</span></span> inch.</p>";
    let result = markdown(source, false);
    assert_eq!(result.matches("72").count(), 1, "{result}");
    assert!(result.contains("Point is"));
    assert!(result.contains("inch."));
    let prose = markdown(
        "<p><math><mi>x</mi></math> explanatory text <span aria-hidden='true'>decoration</span></p>",
        false,
    );
    assert!(prose.contains("explanatory text"), "{prose}");
    let blocks = markdown(
        "before<p><math><mi>x</mi></math><span aria-hidden='true'>x</span></p>after",
        false,
    );
    assert!(blocks.starts_with("before\n\n<math>"), "{blocks}");
    assert!(blocks.ends_with("</math>\n\nafter"), "{blocks}");
}

#[test]
fn math_preserves_structure_attributes_and_inert_text() {
    let result = markdown(
        "<math display='block' onclick='bad()'><mfenced open='[' close=']' separators=';'><mtable><mtr><mtd><mi mathvariant='bold'>x</mi></mtd><mtd><mtext>&lt;img src=x onerror=bad()&gt; &amp;</mtext></mtd></mtr></mtable></mfenced></math>",
        false,
    );
    assert!(
        result.contains("open=\"[\" close=\"]\" separators=\";\""),
        "{result}"
    );
    assert!(result.contains("mathvariant=\"bold\""), "{result}");
    let html = rendered_html(&result);
    assert!(!html.contains("onclick="), "{html}");
    assert!(!html.contains("<img"), "{html}");
    assert!(html.contains("&lt;img"), "{html}");
}

#[test]
fn empty_blocks_separate_text_even_when_they_contain_only_whitespace() {
    for tag in [
        "p",
        "div",
        "blockquote",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "ul",
        "ol",
        "pre",
        "table",
    ] {
        for contents in ["", " \n\t "] {
            let html = format!("before<{tag}>{contents}</{tag}>after");
            assert_eq!(markdown(&html, false), "before\n\nafter", "{html}");
        }
    }
}

#[test]
fn empty_links_keep_destinations_across_adjacent_and_nested_elements() {
    for (html, expected) in [
        ("<a href='/a'></a><a href='/b'></a>", "[](/a)[](/b)"),
        (
            "before<a href='/a'><em> </em></a>after",
            "before [](/a)after",
        ),
        ("<strong><a href='/a'></a></strong>", "**[](/a)**"),
        ("<a href='/a'><h2>heading</h2></a>", "## [heading](/a)"),
        (
            "<a href='/a'><blockquote>quote</blockquote></a>",
            "> [quote](/a)",
        ),
        ("<a href='/a'><ul><li>item</li></ul></a>", "- [item](/a)"),
        ("<a href='/a'><img src='/image'></a>", "[![](/image)](/a)"),
    ] {
        assert_eq!(markdown(html, false), expected, "{html}");
    }
}

#[test]
fn href_without_a_destination_does_not_create_an_empty_markdown_link() {
    for html in [
        "<a>label</a>",
        "<a id='target'>label</a>",
        "<a href=''>label</a>",
    ] {
        assert_eq!(markdown(html, false), "label");
    }
}

#[test]
fn reusable_converter_does_not_leak_link_state_between_documents() {
    let converter = Converter::default();
    for _ in 0..3 {
        for (html, expected) in [
            ("<a href='/one'>one</a>", "[one](/one)"),
            ("<a href='/two'></a>", "[](/two)"),
        ] {
            let dom = Tree::parse(html);
            assert_eq!(converter.convert(&dom, dom.root), expected);
        }
    }
}

#[test]
fn preformatted_code_keeps_leading_trailing_and_internal_spaces() {
    for (html, expected, expected_html) in [
        (
            "<code>  first</code>",
            "`  first`",
            "<p><code>  first</code></p>\n",
        ),
        (
            "<code>last  </code>",
            "`last  `",
            "<p><code>last  </code></p>\n",
        ),
        (
            "<code> a  b </code>",
            "`  a  b  `",
            "<p><code> a  b </code></p>\n",
        ),
        ("<code>   </code>", "`   `", "<p><code>   </code></p>\n"),
        (
            "<code>\tword\t</code>",
            "`\tword\t`",
            "<p><code>\tword\t</code></p>\n",
        ),
        (
            "<code>` a `</code>",
            "`` ` a ` ``",
            "<p><code>` a `</code></p>\n",
        ),
    ] {
        let result = markdown(html, true);
        assert_eq!(result, expected, "{html}");
        assert_eq!(rendered_html(&result), expected_html, "{html}");
    }
}

#[test]
fn preformatted_code_normalizes_line_endings_without_collapsing_spaces() {
    let html = "<code>  a\r\n  b\r  c\n  d  </code>";
    let actual = markdown(html, true);
    assert_eq!(
        rendered_html(&actual),
        "<p><code>  a   b   c   d  </code></p>\n"
    );
    assert_eq!(markdown(html, false), "`a b c d`");
}

#[test]
fn normal_code_moves_unicode_boundary_spaces_outside_delimiters() {
    for space in ['\u{00a0}', '\u{2003}', '\u{2009}', '\u{202f}'] {
        let html = format!("before<code>{space}word{space}</code>after");
        assert_eq!(
            markdown(&html, false),
            format!("before{space}`word`{space}after")
        );
        assert_eq!(
            rendered_html(&markdown(&html, true)),
            format!("<p>before<code>{space}word{space}</code>after</p>\n")
        );
    }
}

#[test]
fn adjacent_code_elements_keep_distinct_values() {
    let html = "<div><code>name</code><code>string</code></div>";
    assert_eq!(
        rendered_html(&markdown(html, false)),
        "<p><code>name</code><code>string</code></p>\n"
    );
    assert_eq!(
        markdown("<code>first</code>suffix<code>second</code>", false),
        "`first`suffix`second`"
    );
}

#[test]
fn empty_inline_elements_between_code_do_not_join_or_invent_text() {
    for empty in [
        "<code></code>",
        "<code><!-- source note --></code>",
        "<em></em>",
        "<strong></strong>",
        "<del></del>",
        "<em><strong></strong></em>",
        "<span></span>",
    ] {
        let source = format!("<code>name</code>{empty}<code>string</code>");
        for preformatted in [false, true] {
            let result = markdown(&source, preformatted);
            assert_eq!(
                rendered_html(&result),
                "<p><code>name</code><code>string</code></p>\n",
                "{source}, preformatted={preformatted}: {result}"
            );
        }
    }
}

#[test]
fn code_remains_distinct_when_surrounding_styles_coalesce() {
    for tag in ["em", "strong", "del"] {
        let source = format!("<{tag}><code>name</code></{tag}><{tag}><code>string</code></{tag}>");
        for preformatted in [false, true] {
            let result = markdown(&source, preformatted);
            assert_eq!(
                rendered_html(&result),
                format!("<p><{tag}><code>name</code><code>string</code></{tag}></p>\n"),
                "{source}, preformatted={preformatted}: {result}"
            );
        }
    }
}

#[test]
fn actual_output_separates_markdown_code_spans() {
    for (source, expected) in [
        (
            "<code>name</code><em> </em><code>string</code>",
            "`name` `string`",
        ),
        (
            "<code>name</code><strong>&nbsp;</strong><code>string</code>",
            "`name`\u{a0}`string`",
        ),
        (
            "<code>name</code><em>and</em><code>string</code>",
            "`name`*and*`string`",
        ),
        (
            "<em><code>name</code></em><code>string</code>",
            "*`name`*`string`",
        ),
        (
            "<code>name</code><em><code>string</code></em>",
            "`name`*`string`*",
        ),
        (
            "<code>name</code><a href='/a'></a><code>string</code>",
            "`name`[](/a)`string`",
        ),
        (
            "<code>name</code><img src='/i'><code>string</code>",
            "`name`![](/i)`string`",
        ),
        (
            "<code>name</code><br><code>string</code>",
            "`name`  \n`string`",
        ),
        (
            "<code>name</code><p></p><code>string</code>",
            "`name`\n\n`string`",
        ),
        (
            "<code>a</code><blockquote><code>b</code></blockquote>x<code>c</code>",
            "`a`\n\n> `b`\n\nx`c`",
        ),
    ] {
        for preformatted in [false, true] {
            assert_eq!(
                markdown(source, preformatted),
                expected,
                "{source}, preformatted={preformatted}"
            );
        }
    }
    for source in [
        "<code>name </code><code>string</code>",
        "<code>name</code><code> string</code>",
        "<code>name</code><code> </code><code>string</code>",
    ] {
        assert_eq!(markdown(source, false), "`name` `string`", "{source}");
    }
}

#[test]
fn neighboring_code_values_keep_their_text_and_node_boundaries() {
    for (source, preformatted, expected) in [
        (
            "<code>git </code><code>status</code>",
            true,
            "<p><code>git </code><code>status</code></p>\n",
        ),
        (
            "<code>--</code><code>help</code>",
            false,
            "<p><code>--</code><code>help</code></p>\n",
        ),
        (
            "<code>*[x]|</code><code>name</code>",
            false,
            "<p><code>*[x]|</code><code>name</code></p>\n",
        ),
        (
            "<code>first</code><code></code><code>second</code>",
            true,
            "<p><code>first</code><code>second</code></p>\n",
        ),
        (
            "<code>first</code><em></em><code>*[x]|&lt;&amp;&gt;`</code><strong></strong><code>third</code><del></del><code>fourth</code>",
            false,
            "<p><code>first</code><code>*[x]|&lt;&amp;&gt;`</code><code>third</code><code>fourth</code></p>\n",
        ),
        (
            "<code>word</code><em></em><code> a  b </code>",
            true,
            "<p><code>word</code><code> a  b </code></p>\n",
        ),
    ] {
        let result = markdown(source, preformatted);
        assert_eq!(rendered_html(&result), expected, "{source}: {result}");
    }
}

#[test]
fn preformatted_block_children_keep_text_boundaries() {
    let html = "<pre><div>ts</div><div><code>function identity() {}</code></div></pre>";
    let output = markdown(html, false);
    assert_eq!(output, "```\nts\nfunction identity() {}\n```");
    assert_eq!(
        rendered_html(&output),
        "<pre><code>ts\nfunction identity() {}\n</code></pre>\n"
    );
}

#[test]
fn adjacent_code_preserves_documentation_examples_as_literal_text() {
    let code_values = |html: &str| {
        let tree = Tree::parse(html);
        tree.nodes
            .iter()
            .filter(|node| node.tag.as_deref() == Some("code"))
            .map(|node| {
                let mut text = String::new();
                let mut child = node.first;
                while let Some(id) = child {
                    let node = &tree.nodes[id];
                    assert!(node.tag.is_none(), "code gained formatting: {html}");
                    text.push_str(node.text.as_deref().unwrap_or_default());
                    child = node.next;
                }
                text
            })
            .collect::<Vec<_>>()
    };
    for value in [
        r"curl https://example.com/a?x=1&amp;y=2",
        r"![icon](image.png) **bold** _name_ ~~old~~",
        r"&lt;T&gt; &amp;amp; &#96;value&#96; C:\work\file",
        r"!&quot;#$%&amp;'()*+,-./:;&lt;=&gt;?@[\]^_&#96;{|}~",
        "变量—café 🙂",
    ] {
        // The HTML source is the oracle: Markdown conversion must preserve
        // each code value, including syntax, entities and Unicode characters.
        let source = format!("<code>{value}</code><code>next</code>");
        let expected = code_values(&source);
        for preformatted in [false, true] {
            let output = markdown(&source, preformatted);
            assert_eq!(
                code_values(&rendered_html(&output)),
                expected,
                "{source}: {output}"
            );
        }
    }
}

#[test]
fn attribute_newlines_remove_indentation_without_joining_words() {
    for separator in ["\n ", "\r\n\t", "\n  \n \t  "] {
        let html = format!("<a href='/a' title='first{separator}second'>link</a>");
        assert_eq!(
            rendered_html(&markdown(&html, false)),
            "<p><a href=\"/a\" title=\"first\nsecond\">link</a></p>\n"
        );
        let html =
            format!("<img src='/i' alt='first{separator}second' title='first{separator}second'>");
        assert_eq!(
            rendered_html(&markdown(&html, false)),
            "<p><img src=\"/i\" alt=\"first second\" title=\"first\nsecond\" /></p>\n"
        );
    }
}

#[test]
fn fragment_links_keep_explicit_targets_without_copying_unreferenced_ids() {
    let source = "<p><a href='#chapter-7'>Chapter</a> <a href='#%E6%B3%A8%E9%87%8A'>Note</a></p><h2 id='chapter-7'>Translated chapter</h2><p id='unreferenced'>Text</p><a name='注释'></a><p>Note body</p>";
    let result = rendered_html(&markdown(source, false));
    assert!(result.contains("href=\"#chapter-7\""), "{result}");
    assert!(result.contains("id=\"chapter-7\""), "{result}");
    assert!(result.contains("id=\"注释\""), "{result}");
    assert!(result.contains("<h2>Translated chapter</h2>"), "{result}");
    assert!(!result.contains("id=\"unreferenced\""), "{result}");
}

#[test]
fn percent_encoded_named_anchor_matches_its_fragment_link() {
    let result = markdown(
        "<a href='#Run%20the%20program'>Run</a><h2><a name='Run the program'></a>Run the program</h2>",
        false,
    );
    assert!(result.contains("[Run](#Run%20the%20program)"), "{result}");
    assert!(
        result.contains("## <a id=\"Run the program\"></a>Run the program"),
        "{result}"
    );
}

#[test]
fn fragment_decoding_does_not_rewrite_dom_ids() {
    let result = markdown("<a href='#x%2520y'>Go</a><p id='x%20y'>Target</p>", false);
    assert!(result.contains("[Go](#x%2520y)"), "{result}");
    assert!(result.contains("<a id=\"x%20y\"></a>"), "{result}");
    assert!(result.contains("Target"), "{result}");
}

#[test]
fn literal_fragment_target_wins_before_percent_decoding() {
    let result = markdown(
        "<a href='#x%41'>Go</a><p id='x%41'>Original target</p><p id='xA'>Different target</p>",
        false,
    );
    assert!(result.contains("<a id=\"x%41\"></a>"), "{result}");
    assert!(!result.contains("<a id=\"xA\"></a>"), "{result}");
}

#[test]
fn fragment_targets_prefer_ids_and_only_accept_legacy_anchor_names() {
    let result = rendered_html(&markdown(
        "<a href='#target'>Go</a><p><a name='target'>Legacy</a></p><p id='target'>ID target</p>",
        false,
    ));
    assert_eq!(result.matches("id=\"target\"").count(), 1, "{result}");
    assert!(result.find("ID target").unwrap() > result.find("id=\"target\"").unwrap());
    assert!(!result.contains("<a id=\"target\"></a>Legacy"), "{result}");

    let result = rendered_html(&markdown(
        "<a href='#target'>Go</a><table><tr><td colspan='2'><mark name='target'>Wrong</mark><a name='target'>Right</a></td></tr></table>",
        false,
    ));
    assert!(!result.contains("id=\"target\"></a>Wrong"), "{result}");
    assert!(result.contains("name=\"target\""), "{result}");
}

#[test]
fn table_fragment_targets_preserve_rows_cells_and_legacy_named_anchors() {
    let source = "<a href='#row'>Row</a><a href='#cell'>Cell</a><a href='#legacy'>Legacy</a><table><tr id='row'><td id='cell'><a name='legacy'></a>Value</td><td>Other</td></tr></table>";
    let result = rendered_html(&markdown(source, false));
    for target in ["id=\"row\"", "id=\"cell\"", "name=\"legacy\""] {
        assert!(result.contains(target), "missing {target}: {result}");
    }
    assert!(result.contains("<td>Other</td>"), "{result}");
}

#[test]
fn image_and_link_attributes_escape_markdown_and_preserve_literal_entities() {
    let html = "<a href='/a\\*?x=&amp;copy;' title='a &quot;quote&quot; and \\ slash'>[label]</a>";
    let result = markdown(html, false);
    assert_eq!(
        rendered_html(&result),
        "<p><a href=\"/a%5C*?x=&amp;copy;\" title=\"a &quot;quote&quot; and \\ slash\">[label]</a></p>\n"
    );
    let html = "<img src='/a(b)' alt='[a] *b* `c` &amp;copy;'>";
    assert_eq!(
        rendered_html(&markdown(html, false)),
        "<p><img src=\"/a(b)\" alt=\"[a] *b* `c` &amp;copy;\" /></p>\n"
    );
}

#[test]
fn table_attributes_keep_the_same_meaning_as_attributes_outside_tables() {
    for content in [
        "<a href='/a' title='first\n  second'>link</a>",
        "<img src='/i' alt='first\n  second' title='first\n  second'>",
    ] {
        let inline = rendered_html(&markdown(content, false));
        let html = format!("<table><tr><th>{content}</th></tr></table>");
        let inline = inline
            .strip_prefix("<p>")
            .unwrap()
            .strip_suffix("</p>\n")
            .unwrap();
        assert_eq!(
            rendered_html(&markdown(&html, false)),
            format!("<table><thead><tr><th>{inline}</th></tr></thead><tbody>\n</tbody></table>\n"),
            "{html}"
        );
    }
}

#[test]
fn heading_attributes_keep_the_same_meaning_as_attributes_outside_headings() {
    for content in [
        "<a href='/a' title='first\n  second'>link</a>",
        "<img src='/i' alt='first\n  second' title='first\n  second'>",
    ] {
        let inline = rendered_html(&markdown(content, false));
        let inline = inline
            .strip_prefix("<p>")
            .unwrap()
            .strip_suffix("</p>\n")
            .unwrap();
        let html = format!("<h2>{content}</h2>");
        assert_eq!(
            rendered_html(&markdown(&html, false)),
            format!("<h2>{inline}</h2>\n")
        );
    }
}

#[test]
fn images_without_sources_do_not_emit_placeholder_markdown() {
    assert_eq!(markdown("a<img>b", false), "ab");
    for image in [
        "<img alt='Email'>",
        "<img src='' alt='Email'>",
        "<img src=' ' alt='Email'>",
    ] {
        let source = format!("<a href='mailto:contact@example.test'>{image}</a>");
        assert_eq!(
            rendered_html(&markdown(&source, false)),
            "<p><a href=\"mailto:contact@example.test\">Email</a></p>\n"
        );
    }
}

#[test]
fn adjacent_controls_and_dates_remain_separate_text_items() {
    let result = rendered_html(&markdown(
        "<button>Cancel</button><button><strong>Subscribe</strong></button><p><time>2024-01-02</time><time>2025-03-04</time></p>",
        false,
    ));
    assert_eq!(
        result,
        "<p>Cancel\n<strong>Subscribe</strong></p>\n<p>2024-01-02\n2025-03-04</p>\n"
    );
}

#[test]
fn empty_accessible_element_retains_its_declared_text() {
    let result = markdown(
        "<p>By Reporter</p><div aria-label='Published: 1:16 p.m. Updated: 10:28 a.m.'></div><a href='/search' aria-label='Search'></a>",
        false,
    );
    assert!(
        result.contains("Published: 1:16 p.m. Updated: 10:28 a.m."),
        "{result}"
    );
    assert!(result.contains("[Search](/search)"), "{result}");
}

#[test]
fn conventional_quote_container_retains_quote_semantics() {
    let result = markdown(
        "<div class='quoteblock selected'>Earlier statement.</div><p>Current reply.</p>",
        false,
    );
    assert!(result.contains("> Earlier statement."), "{result}");
    assert!(result.contains("Current reply."), "{result}");
}

#[test]
fn quote_classes_do_not_replace_list_or_table_semantics() {
    let list = markdown(
        "<ul class='quote'><li>First</li><li>Second</li></ul>",
        false,
    );
    assert_eq!(list, "- First\n- Second");
    let table = rendered_html(&markdown(
        "<table class='quote'><tr><th>Name</th></tr><tr><td>Ada</td></tr></table>",
        false,
    ));
    assert!(table.contains("<table>"), "{table}");
    assert!(table.contains("<td>Ada</td>"), "{table}");
}

#[test]
fn tooltip_trigger_retains_its_reader_facing_detail() {
    let result = markdown(
        "<p>Paid Orientation <span data-toggle='tooltip' data-original-title='$100/day'>Details</span></p>",
        false,
    );
    assert_eq!(result, "Paid Orientation Details");
    let link = markdown(
        "<a href='/download' data-toggle='tooltip' data-original-title='Download the latest version'>Download</a>",
        false,
    );
    assert_eq!(link, "[Download](/download)");
    let fallback = markdown(
        "<span data-toggle='tooltip' data-original-title='$100/day'></span>",
        false,
    );
    assert_eq!(fallback, "$100/day");
    let fallback_link = markdown(
        "<a href='/download' data-toggle='tooltip' data-original-title='Download'></a>",
        false,
    );
    assert_eq!(fallback_link, "[Download](/download)");
    let metadata = markdown(
        "<span data-toggle='tooltip' title='Published'>January 24, 2018</span><span data-toggle='tooltip' title='Reading Time'>3 mins read</span>",
        false,
    );
    assert_eq!(metadata, "January 24, 2018\n3 mins read");
}

#[test]
fn resource_only_headings_are_not_dropped() {
    let result = markdown(
        "<h1><a href='/home'><img src='/logo.svg'></a></h1><h2><span aria-label='Accessible title'></span></h2><p>Article</p>",
        false,
    );
    assert!(result.contains("# [![](/logo.svg)](/home)"), "{result}");
    assert!(result.contains("## Accessible title"), "{result}");
    assert!(result.contains("Article"), "{result}");
    let alternative = markdown(
        "<h1><img alt='Product manual'></h1><h2><img src='javascript:bad()' alt='Safe title'></h2>",
        false,
    );
    assert!(alternative.contains("# Product manual"), "{alternative}");
    assert!(alternative.contains("## Safe title"), "{alternative}");
}

#[test]
fn fallback_labels_preserve_all_children_and_remain_literal_text() {
    let result = markdown(
        "<div title='Tools'><button aria-label='Save'></button><button aria-label='Delete'></button></div><h1><a href='/one' aria-label='One'></a><a href='/two' aria-label='Two'></a></h1><h2 aria-label='[Open](/unintended)'></h2>",
        false,
    );
    assert!(result.contains("Save"), "{result}");
    assert!(result.contains("Delete"), "{result}");
    assert!(result.contains("# [One](/one)[Two](/two)"), "{result}");
    let rendered = rendered_html(&result);
    assert!(!rendered.contains("href=\"/unintended\""), "{rendered}");
    assert!(rendered.contains("[Open](/unintended)"), "{rendered}");
}

#[test]
fn responsive_image_falls_back_from_unusable_lazy_candidates() {
    for lazy in ["", "javascript:bad() 1x"] {
        let result = markdown(
            &format!(
                "<img alt='Chart' data-srcset='{lazy}' srcset='/chart-small.png 1x, /chart.png 2x'>"
            ),
            false,
        );
        assert_eq!(result, "![Chart](/chart.png)");
    }
}

#[test]
fn html_fallback_preserves_referenced_targets_for_special_nodes() {
    let code = markdown(
        "<a href='#L1'>Line 1</a><pre><code><mark id='L1'>let x = 1;</mark></code></pre>",
        false,
    );
    assert!(code.contains("<a id=\"L1\"></a>"), "{code}");
    let table = markdown(
        "<a href='#email'>Email</a><a href='#eq1'>Equation</a><a href='#video1'>Video</a><table><tr><td><input id='email' value='a@example.test'></td><td><math id='eq1'><mi>x</mi></math></td><td><video id='video1' src='/clip.mp4'></video></td></tr></table>",
        false,
    );
    for id in ["email", "eq1", "video1"] {
        assert!(table.contains(&format!("<a id=\"{id}\"></a>")), "{table}");
    }
}

#[test]
fn invalid_nested_and_empty_headings_do_not_emit_literal_markers() {
    let result = markdown(
        "<h3><center><h3>&nbsp;</h3><h2>Department</h2><h2> </h2></center></h3><p>Rule text.</p>",
        false,
    );
    assert!(result.contains("## Department"), "{result}");
    assert!(!result.contains("### ###"), "{result}");
    assert_eq!(result.matches("## Department").count(), 1, "{result}");
}

#[test]
fn preformatted_navigation_retains_link_targets() {
    let result = markdown(
        "<pre>ALL GAMES: <a href='home.shtml'>HOME GAMES</a> : <a href='away.shtml'>AWAY GAMES</a>\nScores stay aligned</pre>",
        false,
    );
    assert!(result.contains("<pre>"), "{result}");
    assert!(
        result.contains("<a href=\"home.shtml\">HOME GAMES</a>"),
        "{result}"
    );
    assert!(
        result.contains("<a href=\"away.shtml\">AWAY GAMES</a>"),
        "{result}"
    );
    assert!(result.contains("Scores stay aligned"), "{result}");
}

#[test]
fn preformatted_code_preserves_externally_referenced_targets() {
    let result = markdown(
        "<a href='#L1'>Line 1</a><pre><code><span id='L1'>let x = 1;</span>\n<span id='L2'>let y = 2;</span></code></pre>",
        false,
    );
    assert!(result.contains("[Line 1](#L1)"), "{result}");
    assert!(
        result.contains("<span id=\"L1\">let x = 1;</span>"),
        "{result}"
    );
}

#[test]
fn inline_code_retains_term_link_targets() {
    let result = markdown(
        "<p>Returns <code><a href='/terms/null'>Null</a></code> when absent.</p>",
        false,
    );
    assert!(
        result.contains("<code><a href=\"/terms/null\">Null</a></code>"),
        "{result}"
    );
    assert!(result.contains("when absent."), "{result}");
}

#[test]
fn empty_check_icon_retains_positive_state() {
    let result = markdown(
        "<div>Image recognition <i class='table-check check'></i></div><div>Speech recognition <i class='table-check'></i></div>",
        false,
    );
    assert!(result.contains("Image recognition ✓"), "{result}");
    assert!(result.contains("Speech recognition"), "{result}");
    assert_eq!(result.matches('✓').count(), 1, "{result}");
}

#[test]
fn code_excludes_embedded_copy_controls_and_scripts() {
    let result = markdown(
        "<pre><code class='language-java'>run();<span class='copy-code-btn'>Copy code</span><script>advert()</script></code></pre>",
        false,
    );
    assert!(result.contains("run();"), "{result}");
    assert!(!result.contains("Copy code"), "{result}");
    assert!(!result.contains("advert"), "{result}");
}

#[test]
fn block_content_inside_heading_keeps_its_own_boundary() {
    let result = markdown(
        "<h1>Concept title<div class='popover'><div>Resource Information</div><p>Long explanation.</p></div></h1>",
        false,
    );
    assert!(result.contains("<h1>Concept title<div"), "{result}");
    assert!(
        result.contains("<div>Resource Information</div>"),
        "{result}"
    );
    assert!(!result.starts_with("# Concept title Resource"), "{result}");
}

#[test]
fn form_values_preserve_current_readable_state_without_secrets() {
    let source = "<table><tr><th>Period</th><th>Mon</th><th>Sun</th></tr><tr><td>Morning</td><td><input type=checkbox checked disabled></td><td><input type=checkbox disabled></td></tr></table><p><input value='Search term'><input type=email value='' placeholder='Your Email'><input type=submit value='Search'><input type=image alt='Map button'><input type=password value='secret' aria-label='Password'><input type=hidden value='token'><input type=file value='private.pdf'><textarea>Notes</textarea></p>";
    let result = rendered_html(&markdown(source, false));
    for value in [
        "Morning",
        "☑",
        "☐",
        "Search term",
        "Your Email",
        "Search",
        "Map button",
        "Password",
        "Notes",
    ] {
        assert!(result.contains(value), "missing {value}: {result}");
    }
    for secret in ["secret", "token", "private.pdf"] {
        assert!(!result.contains(secret), "leaked {secret}: {result}");
    }
}

#[test]
fn unsafe_resource_urls_keep_labels_without_emitting_active_links() {
    let source = "<p><a href='javascript:alert(1)'>Open panel</a><img src='javascript:alert(2)' alt='Poster'><a href='mailto:a@example.test'>Email</a></p>";
    let result = markdown(source, false);
    assert!(result.contains("Open panel"), "{result}");
    assert!(result.contains("Poster"), "{result}");
    assert!(
        result.contains("[Email](mailto:a@example.test)"),
        "{result}"
    );
    assert!(!result.contains("javascript:"), "{result}");
}

#[test]
fn embedded_media_uses_the_same_url_policy_in_all_output_paths() {
    let media = "<video src='/clip.mp4' poster='javascript:alert(1)'></video><iframe src='data:image/png;base64,AAAA' title='Frame'></iframe><audio src='data:image/png;base64,BBBB' title='Audio'></audio>";
    for source in [
        media.to_owned(),
        format!("<table><tr><td colspan='2'>{media}</td></tr></table>"),
    ] {
        let result = markdown(&source, false);
        assert!(result.contains("/clip.mp4"), "{result}");
        assert!(!result.contains("javascript:"), "{result}");
        assert!(!result.contains("data:image"), "{result}");
    }
}

#[test]
fn complex_table_preserves_checkbox_state() {
    let source = "<table><tr><th rowspan=2>Status</th><td><input type=radio checked></td></tr><tr><td><input type=radio></td></tr></table>";
    let result = rendered_html(&markdown(source, false));
    assert!(result.contains("rowspan=\"2\""), "{result}");
    assert!(result.contains("☑"), "{result}");
    assert!(result.contains("☐"), "{result}");
}

#[test]
fn embedded_image_references_survive_regardless_of_pixel_content() {
    for src in [
        "data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20viewBox='0%200%20720%20960'%3E%3C/svg%3E",
        "data:image/svg+xml,%3Csvg%20style='background:red'%20width='32'%20height='32'/%3E",
        "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAAAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==",
        "data:image/gif;base64,R0lGODdhAQABAIEAAP8AAAAAAAAAAAAAACwAAAAAAQABAAAIBAABBAQAOw==",
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGBgAAAABQABpfZFQAAAAABJRU5ErkJggg==",
    ] {
        let html = format!("<img alt='Status image' src=\"{src}\">");
        assert!(markdown(&html, false).contains(src), "{src}");
        let responsive = format!("<picture><source srcset='/status-2x.png 2x'>{html}</picture>");
        assert!(markdown(&responsive, false).contains(src), "{responsive}");
    }
}

#[test]
fn responsive_images_preserve_url_commas_and_use_declared_candidates() {
    let comma_url = "https://cdn.example/c_fill,w_640,h_480/photo.jpg";
    for content in [
        format!("<img alt='Photo' srcset='{comma_url} 1x'>"),
        format!("<a href='/photo'><img alt='Photo' srcset='{comma_url} 1x'></a>"),
        format!(
            "<table><tr><td colspan='2'><img alt='Photo' srcset='{comma_url} 1x'></td></tr></table>"
        ),
    ] {
        let result = markdown(&content, false);
        assert!(result.contains(comma_url), "{result}");
    }
    let result = markdown(
        "<img alt='Photo' src='/current.jpg' data-original='/old.jpg'>",
        false,
    );
    assert!(result.contains("/current.jpg"), "{result}");
    assert!(!result.contains("/old.jpg"), "{result}");
    let result = markdown(
        "<img alt='Photo' src='/placeholder.gif' data-srcset='/small.jpg 1x, /large.jpg 2x'>",
        false,
    );
    assert!(result.contains("/large.jpg"), "{result}");
    for source in [
        "<img alt='Photo' src='/current.jpg' data-srcset='/old.jpg 2x'>",
        "<img alt='Photo' src='data:image/gif;base64,R0lGODdhAQABAIEAAP8AAAAAAAAAAAAAACwAAAAAAQABAAAIBAABBAQAOw==' data-src='/old.jpg'>",
    ] {
        let result = markdown(source, false);
        assert!(!result.contains("/old.jpg"), "{result}");
    }
    let defaults = markdown(
        "<img alt='Photo' srcset='/small.jpg, /large.jpg 2x'><img alt='Safe' srcset='/safe.jpg 1x, javascript:bad() 2x'>",
        false,
    );
    assert!(defaults.contains("![Photo](/large.jpg)"), "{defaults}");
    assert!(defaults.contains("![Safe](/safe.jpg)"), "{defaults}");
}

#[test]
fn fragment_targets_keep_all_aliases_and_delegated_math_descendants() {
    let result = markdown(
        "<a href='#modern'>Modern</a><a href='#legacy'>Legacy</a><a href='#eq'>Equation</a><a id='modern' name='legacy'>Target</a><math><mrow id='eq'><mi>x</mi></mrow></math>",
        false,
    );
    for id in ["modern", "legacy", "eq"] {
        assert!(result.contains(&format!("<a id=\"{id}\"></a>")), "{result}");
    }
}

struct CountingDom<'a> {
    tree: &'a Tree,
    calls: Cell<usize>,
}

impl Dom for CountingDom<'_> {
    type NodeId = usize;

    fn node_kind(&self, node: usize) -> NodeKind<'_> {
        self.calls.set(self.calls.get() + 1);
        self.tree.node_kind(node)
    }
    fn first_child(&self, node: usize) -> Option<usize> {
        self.calls.set(self.calls.get() + 1);
        self.tree.first_child(node)
    }
    fn next_sibling(&self, node: usize) -> Option<usize> {
        self.calls.set(self.calls.get() + 1);
        self.tree.next_sibling(node)
    }
    fn attribute(&self, node: usize, name: &str) -> Option<&str> {
        self.calls.set(self.calls.get() + 1);
        self.tree.attribute(node, name)
    }
}

#[test]
fn transparent_depth_keeps_dom_queries_linear() {
    let measure = |depth: usize| {
        let html = format!(
            "{}content{}",
            "<span>".repeat(depth),
            "</span>".repeat(depth)
        );
        let tree = Tree::parse(&html);
        let dom = CountingDom {
            tree: &tree,
            calls: Cell::new(0),
        };
        let result = Converter::default().convert(&dom, tree.root);
        assert_eq!(result, "content");
        dom.calls.get()
    };
    let shallow = measure(64);
    let deep = measure(128);
    assert!(
        deep < shallow * 3,
        "query growth was not linear: {shallow} -> {deep}"
    );
}

#[test]
fn readable_fallbacks_survive_all_table_representations() {
    for wrapper in [
        "<p>{}</p>",
        "<table><tr><td>{}</td></tr></table>",
        "<table><tr><td colspan='2'>{}</td></tr></table>",
    ] {
        let content = "<a href='/download' aria-label='Download report'> </a><button aria-label='Open panel'><span aria-hidden='true'></span></button><svg><title>Chart title</title></svg>";
        let result = markdown(&wrapper.replace("{}", content), false);
        for expected in ["Download report", "Open panel", "Chart title"] {
            assert!(result.contains(expected), "missing {expected}: {result}");
        }
    }
}

#[test]
fn list_paragraphs_and_code_blocks_require_blank_lines_between_items() {
    for (html, expected) in [
        ("<ul><li>one</li><li>two</li></ul>", "- one\n- two"),
        ("<ul><li><p>one</p></li><li>two</li></ul>", "- one\n\n- two"),
        ("<ul><li>one</li><li><p>two</p></li></ul>", "- one\n\n- two"),
        ("<ol><li></li><li>two</li></ol>", "2. two"),
        (
            "<ul><li><pre><code>x</code></pre></li><li>tail</li></ul>",
            "- ```\n  x\n  ```\n\n- tail",
        ),
    ] {
        assert_eq!(markdown(html, false), expected, "{html}");
    }
    assert_eq!(
        rendered_html(&markdown("<ul><li><p>one</p></li><li>two</li></ul>", false)),
        "<ul>\n<li>\n<p>one</p>\n</li>\n<li>\n<p>two</p>\n</li>\n</ul>\n"
    );
}

#[test]
fn empty_pre_blocks_disappear_and_nonempty_pre_keeps_blank_lines() {
    for contents in ["", " ", "\n", "\n \n"] {
        assert_eq!(
            markdown(&format!("<pre><code>{contents}</code></pre>"), false),
            ""
        );
    }
    assert_eq!(
        markdown("<pre><code>\nfirst\n\nlast\n\n</code></pre>", false),
        "```\n\nfirst\n\nlast\n\n```"
    );
}

#[test]
fn fallback_block_elements_separate_neighboring_words() {
    for tag in ["audio", "canvas", "output"] {
        assert_eq!(
            markdown(&format!("before<{tag}>content</{tag}>after"), false),
            "before\n\ncontent\n\nafter"
        );
    }
}

#[test]
fn live_textarea_value_replaces_or_clears_serialized_default_in_all_paths() {
    for wrapper in [
        "<div title='Group'>{}</div>",
        "<table><tr><td colspan='2'>{}</td></tr></table>",
    ] {
        let current = markdown(
            &wrapper.replace(
                "{}",
                "<textarea value='Current user text'>Old default</textarea>",
            ),
            false,
        );
        assert!(current.contains("Current user text"), "{current}");
        assert!(!current.contains("Old default"), "{current}");

        let cleared = markdown(
            &wrapper.replace("{}", "<textarea value=''>Old default</textarea>"),
            false,
        );
        assert!(!cleared.contains("Old default"), "{cleared}");
    }
}

#[test]
fn html_table_math_delegation_preserves_referenced_descendant_targets() {
    let result = markdown(
        "<a href='#L1'>Term</a><table><tr><td colspan='2'><math><mrow id='L1'><mi>x</mi></mrow></math></td></tr></table>",
        false,
    );
    assert!(result.contains("<a id=\"L1\"></a>"), "{result}");
    assert!(result.contains("<math>"), "{result}");

    let alternative = markdown(
        "<a href='#wrapped'>Term</a><table><tr><td colspan='2'><span id='wrapped'><span><math><mi>x</mi></math></span><span aria-hidden='true'>visual</span></span></td></tr></table>",
        false,
    );
    assert!(
        alternative.contains("<a id=\"wrapped\"></a>"),
        "{alternative}"
    );
    assert!(alternative.contains("<math>"), "{alternative}");
}

#[test]
fn srcset_rejects_invalid_numbers_and_commas_inside_parentheses() {
    let result = markdown(
        "<img alt='Density' srcset='/safe.jpg 1x, /invalid.jpg +2x, /also-invalid.jpg 2.x'><img alt='Parentheses' srcset='/invalid.jpg test(a, /phantom.jpg 4x, b), /safe-parentheses.jpg 1x'>",
        false,
    );
    assert!(result.contains("![Density](/safe.jpg)"), "{result}");
    assert!(
        result.contains("![Parentheses](/safe-parentheses.jpg)"),
        "{result}"
    );
    assert!(!result.contains("phantom.jpg"), "{result}");
}
