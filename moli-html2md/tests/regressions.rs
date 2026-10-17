mod support;

use moli_html2md::{Converter, Options};
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
fn empty_links_use_their_own_accessible_label_without_replacing_visible_text() {
    for (html, expected) in [
        (
            "<a href='/report' aria-label='Open report'></a>",
            "[Open report](/report)",
        ),
        (
            "<a href='/report' aria-label='Open report'>Visible report</a>",
            "[Visible report](/report)",
        ),
        (
            "<a href='/report' aria-label='  Open   report  '> </a>",
            "[Open report](/report)",
        ),
        ("<a href='/report' aria-label='   '></a>", "[](/report)"),
        (
            "<a href='/report' aria-label='[Open] *report*'></a>",
            r"[\[Open\] \*report\*](/report)",
        ),
        (
            "<a href='/one' aria-label='One'><span></span></a><a href='/two' aria-label='Two'></a>",
            "[One](/one)[Two](/two)",
        ),
        (
            "<a href='/report' aria-label='Fallback'><img src='/cover.png' alt='Cover'></a>",
            "[![Cover](/cover.png)](/report)",
        ),
        ("<a aria-label='No destination'></a>", ""),
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
        assert_eq!(markdown(&html, false), "[link](/a \"first\nsecond\")");
        let html =
            format!("<img src='/i' alt='first{separator}second' title='first{separator}second'>");
        assert_eq!(
            markdown(&html, false),
            "![first\nsecond](/i \"first\nsecond\")"
        );
    }
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
    for html in [
        "a<img>b",
        "a<img alt='label'>b",
        "a<img src='' alt='label'>b",
    ] {
        assert_eq!(markdown(html, false), "ab");
    }
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
fn native_media_elements_keep_declared_resource_urls() {
    let result = markdown(
        "<video src='/direct.mp4' poster='/poster.jpg'><source src='/movie.webm'><source src='/movie.mp4'><track src='/captions.vtt'></video><audio src='/episode.ogg'></audio>",
        false,
    );
    for expected in [
        "![Video poster](/poster.jpg)",
        "[Video](/direct.mp4)",
        "[Media source](/movie.webm)",
        "[Media source](/movie.mp4)",
        "[Media track](/captions.vtt)",
        "[Audio](/episode.ogg)",
    ] {
        assert!(result.contains(expected), "missing {expected}: {result}");
    }
}

#[test]
fn media_elements_do_not_emit_active_script_urls() {
    for unsafe_url in [
        "javascript:alert(1)",
        "java&#10;script:alert(1)",
        "vbscript:bad",
        "data:text/html,bad",
        "about:blank",
        "#fragment",
    ] {
        let source = format!("<video src='{unsafe_url}'><source src='{unsafe_url}'></video>");
        let result = markdown(&source, false);
        assert!(result.is_empty(), "{source}: {result}");
    }
}

#[test]
fn preformatted_directory_links_keep_destinations_outside_literal_text() {
    let html = "<pre><a href='release/file.mp4'>file.mp4</a> <a href='release/checksum.txt'>checksum</a></pre>";
    for preformatted in [false, true] {
        assert_eq!(
            markdown(html, preformatted),
            "```\nfile.mp4 checksum\n```\n\n[file.mp4](release/file.mp4)\n[checksum](release/checksum.txt)"
        );
    }
}

#[test]
fn preformatted_code_cross_references_preserve_literal_source() {
    let html = "<pre><code class='language-rust'><a href='/macros/println'>println!</a>(<span>&quot;hello&quot;</span>);\nlet values = <a href='/types/Vec'><span>Vec</span></a>::new();</code></pre>";
    for preformatted in [false, true] {
        assert_eq!(
            markdown(html, preformatted),
            "```rust\nprintln!(\"hello\");\nlet values = Vec::new();\n```\n\n[println!](/macros/println)\n[Vec](/types/Vec)"
        );
    }
}

#[test]
fn preformatted_directory_links_survive_around_code_examples() {
    let html = "<pre><a href='before.txt'>before</a>\n<span><code><a href='/types/Value'>Value</a> value;</code></span>\n<a href='after.txt'>after</a></pre>";
    for preformatted in [false, true] {
        assert_eq!(
            markdown(html, preformatted),
            "```\nbefore\nValue value;\nafter\n```\n\n[before](before.txt)\n[Value](/types/Value)\n[after](after.txt)"
        );
    }
}

#[test]
fn preformatted_signatures_and_language_blocks_keep_source_without_code_wrappers() {
    for (html, expected) in [
        (
            "<pre>public <a href='https://docs.example/String'>String</a> getText()</pre>",
            "```\npublic String getText()\n```\n\n[String](https://docs.example/String)",
        ),
        (
            "<pre class='language-rust'>let value: <a href='/types/String'>String</a> = String::new();</pre>",
            "```rust\nlet value: String = String::new();\n```\n\n[String](/types/String)",
        ),
    ] {
        for preformatted in [false, true] {
            assert_eq!(markdown(html, preformatted), expected, "{html}");
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
fn accessible_link_fallback_never_duplicates_visible_inline_or_block_descendants() {
    for content in [
        "Visible",
        "<img src='/cover' alt='Cover'>",
        "<code>code</code>",
        "<pre>preformatted</pre>",
        "<pre><code>nested code</code></pre>",
        "<p>Paragraph</p>",
        "<h2>Heading</h2>",
        "<ul><li>Item</li></ul>",
        "<blockquote>Quotation</blockquote>",
        "<table><tr><td>Cell</td></tr></table>",
    ] {
        let plain = format!("<a href='/target'>{content}</a>");
        let labeled = format!("<a href='/target' aria-label='Fallback'>{content}</a>");
        assert_eq!(
            markdown(&labeled, false),
            markdown(&plain, false),
            "{content}"
        );
    }
}

#[test]
fn empty_link_fallback_belongs_only_to_that_anchor() {
    assert_eq!(
        markdown(
            "<div aria-label='Ancestor'><a href='/one'></a><span aria-label='Adjacent'></span><a href='/two' aria-label='Own'></a></div>",
            false
        ),
        "[](/one)[Own](/two)"
    );
    assert_eq!(
        markdown(
            "<a href='/target' aria-label='Own'><span></span><code></code></a>",
            false
        ),
        "[Own](/target)"
    );
}
