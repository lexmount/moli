mod support;

use moli_html2md::convert;
use pulldown_cmark::{Event, Options, Parser};
use support::Tree;

fn markdown(source: &str) -> String {
    let tree = Tree::parse(source);
    convert(&tree, tree.root)
}

fn math_payloads(markdown: &str) -> Vec<String> {
    Parser::new_ext(markdown, Options::ENABLE_MATH)
        .filter_map(|event| match event {
            Event::InlineMath(value) | Event::DisplayMath(value) => Some(value.into_string()),
            _ => None,
        })
        .collect()
}

fn rendered_math_html(markdown: &str) -> String {
    let mut html = String::new();
    pulldown_cmark::html::push_html(
        &mut html,
        Parser::new_ext(
            markdown,
            Options::ENABLE_MATH | Options::ENABLE_STRIKETHROUGH,
        ),
    );
    html
}

#[test]
fn image_dollars_preserve_images_links_and_adjacent_formulas() {
    for (source, expected_math) in [
        (r#"<p><img src="/p.png" alt="$5">(\(x\))</p>"#, vec!["x"]),
        (
            r#"<p><a href="/details"><img src="/p.png" alt="$5"></a>\(x\)</p>"#,
            vec!["x"],
        ),
        (
            r#"<p>\(x\)<img src="/p.png" alt="$5"><a href="/details">details</a>\(y\)</p>"#,
            vec!["x", "y"],
        ),
    ] {
        let result = markdown(source);
        assert_eq!(math_payloads(&result), expected_math, "{source}: {result}");
        let html = rendered_math_html(&result);
        assert!(html.contains("<img src=\"/p.png\" alt=\"$5\""), "{html}");
        if source.contains("/details") {
            assert!(html.contains("<a href=\"/details\">"), "{html}");
        }
    }
}

#[test]
fn verbatim_dollars_fall_back_without_partial_math() {
    for expression in [
        r"\(\verb|$|\)",
        r"\[\verb|$|\]",
        r"\(\verb|a$b$c|\)",
        r"\(\verb$literal$\)",
        r"\(\verb|\\$|\)",
    ] {
        let result = markdown(&format!("<p>{expression}$x$</p>"));
        assert_eq!(math_payloads(&result), ["x"], "{expression}: {result}");
        let events: String = Parser::new_ext(&result, Options::ENABLE_MATH)
            .filter_map(|event| match event {
                Event::Text(value) => Some(value.into_string()),
                _ => None,
            })
            .collect();
        assert_eq!(events, expression, "{result}");
    }
    for payload in [r"\verb|a  b|", r"\verb|\$|", r"\verb*|50%|"] {
        let result = markdown(&format!(r"<p>\({payload}\)</p>"));
        assert_eq!(math_payloads(&result), [payload], "{result}");
    }
}

#[test]
fn unbalanced_verbatim_braces_fall_back_without_consuming_later_math() {
    for payload in [
        r"\verb|}|",
        r"\verb|{|",
        r"\verb|}{|",
        r"\verb*|a}b{|",
        r"\verb|\\}|",
        r"\verb|\\{|",
    ] {
        for (open, close) in [(r"\(", r"\)"), (r"\[", r"\]"), ("$", "$"), ("$$", "$$")] {
            let expression = format!("{open}{payload}{close}");
            for (following, expected_math) in [("", vec![]), (r"\(x\)", vec!["x"])] {
                let result = markdown(&format!("<p>{expression}{following}</p>"));
                assert_eq!(math_payloads(&result), expected_math, "{result}");
                let text: String = Parser::new_ext(&result, Options::ENABLE_MATH)
                    .filter_map(|event| match event {
                        Event::Text(value) => Some(value.into_string()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(text, expression, "{result}");
            }
        }
    }
}

#[test]
fn balanced_and_escaped_math_braces_remain_literal_payloads() {
    for payload in [
        r"\verb|{}|",
        r"\verb|a{b}c|",
        r"\verb|\}|",
        r"\verb|\{|",
        r"\verb|\\{}|",
        r"\frac{x_i}{y}",
        r"\left\{x\right\}",
    ] {
        let result = markdown(&format!(r"<p>\({payload}\)\(x\)</p>"));
        assert_eq!(math_payloads(&result), [payload, "x"], "{result}");
    }
}

#[test]
fn script_boundaries_preserve_ancestor_emphasis_and_nested_content() {
    for (outer, inner) in [("strong", "em"), ("em", "strong")] {
        for script in ["sub", "sup"] {
            for (content, expected_content, expected_math) in [
                ("b", "b", vec![]),
                (
                    r"\(b\)",
                    "<span class=\"math math-inline\">b</span>",
                    vec!["b"],
                ),
                ("<del>b</del>", "<del>b</del>", vec![]),
            ] {
                for tail in ["", "c"] {
                    for after in ["", "after"] {
                        let source = format!(
                            "<{outer}><{inner}>a</{inner}><{script}><{inner}>{content}</{inner}></{script}>{tail}</{outer}>{after}"
                        );
                        let result = markdown(&source);
                        assert_eq!(math_payloads(&result), expected_math, "{source}: {result}");
                        let expected = format!(
                            "<p><{outer}><{inner}>a</{inner}><{script}><{inner}>{expected_content}</{inner}></{script}>{tail}</{outer}>{after}</p>\n"
                        );
                        assert_eq!(rendered_math_html(&result), expected, "{source}: {result}");
                    }
                }
            }
        }
    }
}

#[test]
fn resource_like_verbatim_fallbacks_do_not_expose_inner_math() {
    for (open, close) in [(r"\(", r"\)"), (r"\[", r"\]")] {
        for resource in ["[a](b)", "[a][b]", "![[a]]"] {
            let expression = format!(r"{open}\verb|{resource}$x$|{close}");
            let result = markdown(&format!("<p>{expression}$y$</p>"));
            assert_eq!(math_payloads(&result), ["y"], "{expression}: {result}");
            let text: String = Parser::new_ext(&result, Options::ENABLE_MATH)
                .filter_map(|event| match event {
                    Event::Text(value) => Some(value.into_string()),
                    _ => None,
                })
                .collect();
            assert_eq!(text, expression, "{result}");
        }
    }
}

#[test]
fn dollar_resource_rejection_retries_at_the_closing_delimiter() {
    for resource in ["[a](b)", "[a][b]", "![[a]]"] {
        let result = markdown(&format!("<p>${resource}$x$</p>"));
        assert_eq!(math_payloads(&result), ["x"], "{resource}: {result}");
        let text: String = Parser::new_ext(&result, Options::ENABLE_MATH)
            .filter_map(|event| match event {
                Event::Text(value) => Some(value.into_string()),
                _ => None,
            })
            .collect();
        assert_eq!(text, format!("${resource}"), "{result}");
    }
}

#[test]
fn mathml_escaping_preserves_literal_syntax_and_attributes() {
    let source = concat!(
        "<math><mtext mathvariant=\"x&amp;&lt;&gt;&quot;'&#10;&#9;\">",
        "A&amp;B &lt;C&gt; &quot;D&quot; \\*_`~[]$ &amp;#36; café&#10;&#9;尾&nbsp;",
        "</mtext></math>"
    );
    let result = markdown(source);
    assert_eq!(
        result,
        concat!(
            "<math><mtext mathvariant=\"x&amp;&lt;&gt;&quot;'  \">",
            "A&amp;B &lt;C&gt; &quot;D&quot; &#92;&#42;&#95;&#96;&#126;&#91;&#93;&#36; &amp;#36; café  尾\u{a0}",
            "</mtext></math>"
        )
    );
    assert!(math_payloads(&result).is_empty(), "{result}");
    let rendered = Tree::parse(&rendered_math_html(&result));
    let mtext = rendered
        .nodes
        .iter()
        .find(|node| node.tag.as_deref() == Some("mtext"))
        .unwrap();
    assert_eq!(
        mtext.attrs,
        [("mathvariant".to_owned(), "x&<>\"'  ".to_owned())]
    );
    assert_eq!(
        rendered.nodes[mtext.first.unwrap()].text.as_deref(),
        Some("A&B <C> \"D\" \\*_`~[]$ &#36; café  尾\u{a0}")
    );
}

#[test]
fn mathml_alternatives_preserve_links_and_inline_semantics() {
    for (open, close) in [
        ("<a href=\"/details\">", "</a>"),
        ("<sup>", "</sup>"),
        ("<sub>", "</sub>"),
        ("<sup><a href=\"/details\">", "</a></sup>"),
    ] {
        for hidden_first in [false, true] {
            let primary = format!("{open}<math><mi>x</mi></math>{close}");
            let hidden = "<span aria-hidden=\"true\">visual-only x</span>";
            let children = if hidden_first {
                format!("{hidden}{primary}")
            } else {
                format!("{primary}{hidden}")
            };
            let result = markdown(&format!("<span>{children}</span>"));
            let html = rendered_math_html(&result);
            assert!(html.contains(&primary), "{result}: {html}");
            assert_eq!(html.matches("<math>").count(), 1, "{html}");
            assert!(!html.contains("visual-only"), "{html}");
        }
    }
}

#[test]
fn mathml_alternatives_keep_surrounding_whitespace() {
    for (source_space, expected_space) in [(" ", " "), ("&nbsp;", "\u{a0}")] {
        for hidden_first in [false, true] {
            let primary = "<math><mi>x</mi></math>";
            let hidden = "<span aria-hidden=\"true\">visual-only x</span>";
            let children = if hidden_first {
                format!("{hidden}{primary}")
            } else {
                format!("{primary}{hidden}")
            };
            let source = format!(
                "<p>Let<span>{source_space}{children}{source_space}</span>be positive.</p>"
            );
            let result = markdown(&source);
            let expected = format!("Let{expected_space}{primary}{expected_space}be positive.");
            assert_eq!(result, expected, "{source}");
            assert_eq!(rendered_math_html(&result), format!("<p>{expected}</p>\n"));
        }
    }
}

#[test]
fn mathml_actions_keep_the_first_branch_and_a_single_fraction_argument() {
    for (action, numerator) in [
        (
            "<maction><mn>1</mn><mn>2</mn></maction>",
            "<mrow><mn>1</mn></mrow>",
        ),
        (
            "<maction>ignored text<mn>1</mn><mn>2</mn></maction>",
            "<mrow><mn>1</mn></mrow>",
        ),
        ("<maction></maction>", "<mrow></mrow>"),
        (
            "<maction><mrow><mn>1</mn><mo>+</mo><mn>2</mn></mrow><mn>9</mn></maction>",
            "<mrow><mrow><mn>1</mn><mo>+</mo><mn>2</mn></mrow></mrow>",
        ),
    ] {
        let result = markdown(&format!("<math><mfrac>{action}<mn>3</mn></mfrac></math>"));
        let expected = format!("<math><mfrac>{numerator}<mn>3</mn></mfrac></math>");
        assert_eq!(result, expected, "{action}");
        assert!(rendered_math_html(&result).contains(&expected), "{result}");
        assert_eq!(rendered_math_html(&result), support::rendered_html(&result));
    }
}

#[test]
fn filtered_mathml_containers_keep_their_grouping() {
    let result =
        markdown("<math><mfrac><unknown><mn>1</mn><mn>2</mn></unknown><mn>3</mn></mfrac></math>");
    assert_eq!(
        result,
        "<math><mfrac><mrow><mn>1</mn><mn>2</mn></mrow><mn>3</mn></mfrac></math>"
    );
}

#[test]
fn tex_comments_preserve_text_and_control_word_boundaries() {
    for command in [
        "text",
        "mbox",
        "textrm",
        "textbf",
        "textit",
        "texttt",
        "textsf",
        "textnormal",
        "hbox",
    ] {
        for newline in ["\n", "\r\n", "\r"] {
            let result = markdown(&format!(
                "<p>\\(\\{command}{{foo% comment{newline}bar}}\\)</p>"
            ));
            assert_eq!(
                math_payloads(&result),
                [format!(r"\{command}{{foobar}}")],
                "{result}"
            );
        }
    }
    for (payload, expected) in [
        ("\\alpha% comment\nb", r"\alpha b"),
        ("\\alpha% first\n% second\nb", r"\alpha b"),
        ("\\alpha% first\nb% second\nc", r"\alpha bc"),
        ("\\\\alpha% comment\nb", r"\\alphab"),
        ("\\text{foo % comment\nbar}", r"\text{foo bar}"),
        (
            "\\text{foo% comment\n\\textbf{bar}}",
            r"\text{foo\textbf{bar}}",
        ),
    ] {
        let result = markdown(&format!(r"<p>\({payload}\)</p>"));
        assert_eq!(math_payloads(&result), [expected], "{result}");
    }
}
