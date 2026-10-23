mod support;

use moli_html2md::convert;
use std::cell::Cell;
use support::{Tree, rendered_html};

struct Counted {
    tree: Tree,
    visits: Cell<usize>,
}
impl moli_html2md::Dom for Counted {
    type NodeId = usize;
    fn node_kind(&self, node: usize) -> moli_html2md::NodeKind<'_> {
        self.visits.set(self.visits.get() + 1);
        self.tree.node_kind(node)
    }
    fn first_child(&self, node: usize) -> Option<usize> {
        self.tree.first_child(node)
    }
    fn next_sibling(&self, node: usize) -> Option<usize> {
        self.tree.next_sibling(node)
    }
    fn attribute(&self, node: usize, name: &str) -> Option<&str> {
        self.tree.attribute(node, name)
    }
}

fn markdown(html: &str) -> String {
    let tree = Tree::parse(html);
    convert(&tree, tree.root)
}

#[test]
fn private_class_names_do_not_invent_document_semantics() {
    let result = markdown(
        "<div class='quote'>Request a price quote</div><i class='fa-check'></i>\
         <blockquote>Actual quotation</blockquote>\
         <span data-toggle='tooltip' data-original-title='Private plugin state'></span>\
         <button aria-label='Save'></button>",
    );
    let html = rendered_html(&result);
    assert!(!result.contains('✓'), "{result}");
    assert!(!result.contains("Private plugin state"), "{result}");
    assert!(html.contains("<p>Request a price quote</p>"), "{html}");
    assert!(html.contains("<blockquote>"), "{html}");
    assert!(result.contains("Actual quotation"), "{result}");
    assert!(result.contains("Save"), "{result}");
}

#[test]
fn standard_image_sources_are_not_overridden_by_filename_guesses() {
    for source in [
        "/photos/placeholder.gif",
        "/art/blank.png",
        "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///ywAAAAAAQABAAACAUwAOw==",
    ] {
        let result = markdown(&format!(
            "<img src='{source}' data-src='/private-alternative.jpg' alt='Original'>"
        ));
        assert!(result.contains(source), "{result}");
        assert!(!result.contains("private-alternative"), "{result}");
    }
    let result =
        markdown("<img src='javascript:invalid()' data-src='/declared-photo.jpg' alt='Fallback'>");
    assert!(result.contains("declared-photo.jpg"), "{result}");
}

#[test]
fn nested_accessible_labels_do_not_rescan_the_whole_tree() {
    let count = |depth| {
        let html = format!(
            "{}Readable terminal{}",
            "<span role='button' aria-label='Fallback'>".repeat(depth),
            "</span>".repeat(depth),
        );
        let dom = Counted {
            tree: Tree::parse(&html),
            visits: Cell::new(0),
        };
        assert_eq!(convert(&dom, dom.tree.root), "Readable terminal");
        dom.visits.get()
    };
    let small = count(100);
    let large = count(400);
    assert!(large <= small * 5, "{small} -> {large}");
}

#[test]
fn code_excludes_interactive_controls_without_guessing_class_names() {
    let result = markdown(
        "<pre><code>run();<button>Copy</button><span role='button'>Copy again</span>\
         <span class='copy-button'>literal_code</span><button class='unrelated'>Menu</button>\
         </code></pre>",
    );
    assert!(result.contains("run();literal_code"), "{result}");
    for excluded in ["Copy", "Menu"] {
        assert!(!result.contains(excluded), "{result}");
    }
}

#[test]
fn formulas_keep_their_order_across_output_boundaries() {
    let result = markdown(
        r"<p>$a_1$<img src='/plot.png' alt='Plot'>$b_2$<code>literal</code>
           $c_3$<a href='/target'>Link</a>$d_4$<br>$e_5$</p><blockquote>$f_6$</blockquote>",
    );
    let mut end = 0;
    for item in [
        "$a_1$", "plot.png", "$b_2$", "literal", "$c_3$", "Link", "$d_4$", "$e_5$", "$f_6$",
    ] {
        let start = result[end..]
            .find(item)
            .unwrap_or_else(|| panic!("missing {item}: {result}"));
        end += start + item.len();
    }
}

#[test]
fn textarea_uses_its_content_not_a_nonstandard_value_attribute() {
    for value in ["unrelated", ""] {
        let result = markdown(&format!(
            "<div title='Group'><textarea value='{value}'>Authored text</textarea></div>"
        ));
        assert!(result.contains("Authored text"), "{result}");
        assert!(!result.contains("unrelated"), "{result}");
    }
}

#[test]
fn accessible_label_queries_remain_linear_with_large_unreadable_branches() {
    let count = |depth| {
        let branch = "<svg><path d='M0 0'/><path d='M1 1'/></svg>".repeat(20);
        let html = format!(
            "{}{}{}",
            format!("<span role='button' aria-label='Fallback'>{branch}").repeat(depth),
            branch,
            "</span>".repeat(depth)
        );
        let dom = Counted {
            tree: Tree::parse(&html),
            visits: Cell::new(0),
        };
        let result = convert(&dom, dom.tree.root);
        assert_eq!(result.matches("Fallback").count(), 1, "{result}");
        dom.visits.get()
    };
    let small = count(30);
    let large = count(120);
    assert!(large <= small * 5, "{small} -> {large}");
}

#[test]
fn labels_cannot_replace_media_choices_or_rules() {
    for html in [
        "<div title='Demo'><video src='/demo.mp4'></video></div>",
        "<a href='/watch' title='Watch'><iframe src='https://example.test/movie'></iframe></a>",
        "<button aria-label='Demo'><audio><source src='/demo.ogg'></audio></button>",
    ] {
        let result = markdown(html);
        assert!(
            result.contains("demo.mp4")
                || result.contains("example.test/movie")
                || result.contains("demo.ogg"),
            "{html}: {result}"
        );
    }
    let result = markdown(
        "<button aria-label='Choices'><select><optgroup label='Group'><option label='First'></option></optgroup></select></button>",
    );
    assert!(
        result.contains("Group") && result.contains("First"),
        "{result}"
    );
    let result = markdown("<button aria-label='Rule'><hr></button>");
    assert!(result.contains("---"), "{result}");
}

#[test]
fn accessible_fallback_is_named_content_and_has_word_boundaries() {
    let result = markdown(
        "<p>x<button aria-label='Save'></button>y</p><nav aria-label='Pagination'></nav><div aria-label='Advertisement'></div><span title='tooltip'></span><button aria-hidden='true' aria-label='Hidden'></button><div role='progressbar' aria-label='Loading'></div><button aria-label='Download'><script type='application/json'>private state</script></button>",
    );
    assert!(result.contains("x Save y"), "{result}");
    assert!(result.contains("Download"), "{result}");
    for excluded in [
        "Pagination",
        "Advertisement",
        "tooltip",
        "Hidden",
        "Loading",
        "private state",
    ] {
        assert!(!result.contains(excluded), "{result}");
    }
}

#[test]
fn phrasing_controls_and_dates_keep_inline_styles_and_table_cells() {
    let result = markdown(
        "<strong>Posted <time>today</time></strong><a href='/p'>Posted <time>Oct 1</time></a><table><tr><th>Date</th><th>Action</th></tr><tr><td>x <time>Oct 1</time> y</td><td>a <button>Save</button> b</td></tr></table>",
    );
    let html = rendered_html(&result);
    assert!(html.contains("<strong>Posted today</strong>"), "{html}");
    assert!(html.contains("<a href=\"/p\">Posted Oct 1</a>"), "{html}");
    assert!(!result.contains("<br>"), "{result}");
}

#[test]
fn input_hints_do_not_masquerade_as_filled_values() {
    let result = markdown(
        "<input type='email' value='' placeholder='Your Email'><input value='Actual value'><input type='password' value='secret' aria-label='Password'>",
    );
    assert!(!result.contains("Your Email"), "{result}");
    assert!(
        result.contains("Actual value") && !result.contains("Password"),
        "{result}"
    );
    assert!(!result.contains("secret"), "{result}");
}

#[test]
fn links_retain_legitimate_non_web_destinations() {
    for href in [
        "magnet:?xt=urn:btih:123",
        "sms:+12025550123",
        "geo:1,2",
        "file:///tmp/manual.pdf",
        "myapp:open/123",
        "blob:https://example.test/id",
        "data:application/pdf;base64,JVBERi0=",
    ] {
        let result = markdown(&format!("<a href='{href}'>Open</a>"));
        assert!(result.contains(href), "{href}: {result}");
    }
    for href in [
        "javascript:alert(1)",
        "vbscript:run()",
        "data:text/html,unsafe",
    ] {
        let result = markdown(&format!("<a href='{href}'>Safe label</a>"));
        assert!(!result.contains(href), "{result}");
    }
}

#[test]
fn inline_media_keeps_links_emphasis_and_table_flow() {
    for (source, expected) in [
        (
            "<a href='/w'>Watch <video src='/v.mp4'></video></a>",
            "[Watch Video (/v.mp4)](/w)",
        ),
        (
            "<p>See <iframe src='/frame'></iframe> below</p>",
            "See [Embedded content](/frame) below",
        ),
        (
            "<em>Clip <video src='/a.mp4'></video> now</em>",
            "*Clip [Video](/a.mp4) now*",
        ),
    ] {
        assert_eq!(markdown(source), expected);
    }
    let result = markdown("<table><tr><td>Clip <video src='/a.mp4'></video> now</td></tr></table>");
    assert!(result.contains("Clip [Video](/a.mp4) now"), "{result}");
    assert!(!result.contains("<br>"), "{result}");
}

#[test]
fn named_images_and_poster_only_video_keep_their_alternatives() {
    for source in [
        "<video poster='/cover.jpg'></video>",
        "<video poster='/cover.jpg' src='blob:https://site/id'></video>",
    ] {
        assert!(markdown(source).contains("![Video poster](/cover.jpg)"));
    }
    assert_eq!(
        markdown("<a href='/home'><img src='' aria-label='Home'></a>"),
        "[Home](/home)"
    );
    assert_eq!(
        markdown("<img src='/logo.png' aria-label='Logo'>"),
        "![Logo](/logo.png)"
    );
    assert_eq!(
        markdown("<img src='/logo.png' alt='Authored' aria-label='Other'>"),
        "![Authored](/logo.png)"
    );
    assert_eq!(
        markdown("<a href='/t'><svg><title>Twitter</title><path/></svg>Follow us</a>"),
        "[Twitter Follow us](/t)"
    );
    assert_eq!(
        markdown("<svg><title>Chart description</title><text>Revenue</text></svg>"),
        "Revenue"
    );
}

#[test]
fn live_controls_replace_defaults_including_explicit_clears() {
    struct Live(Tree);
    impl moli_html2md::Dom for Live {
        type NodeId = usize;
        fn node_kind(&self, n: usize) -> moli_html2md::NodeKind<'_> {
            self.0.node_kind(n)
        }
        fn first_child(&self, n: usize) -> Option<usize> {
            self.0.first_child(n)
        }
        fn next_sibling(&self, n: usize) -> Option<usize> {
            self.0.next_sibling(n)
        }
        fn attribute(&self, n: usize, k: &str) -> Option<&str> {
            self.0.attribute(n, k)
        }
        fn control_value(&self, n: usize) -> Option<&str> {
            self.0.attribute(n, "data-live")
        }
    }
    for tag in ["input", "textarea"] {
        for value in ["Current", ""] {
            let html = if tag == "input" {
                format!("<input value='Stale' data-live='{value}' aria-label='Field'>")
            } else {
                format!("<textarea data-live='{value}'>Stale</textarea>")
            };
            let dom = Live(Tree::parse(&html));
            let result = convert(&dom, dom.0.root);
            assert!(!result.contains("Stale"), "{result}");
            if !value.is_empty() {
                assert_eq!(result, "Current");
            } else {
                assert_eq!(result, "");
            }
        }
    }
}

#[test]
fn media_fallback_and_decorative_images_keep_author_intent() {
    for src in ["", "blob:https://site/id"] {
        let output = markdown(&format!(
            "<video poster='/cover.jpg' src='{src}'><a href='/clip.mp4'>Download</a></video>"
        ));
        assert!(output.contains("![Video poster](/cover.jpg)"), "{output}");
        assert!(output.contains("[Download](/clip.mp4)"), "{output}");
    }
    assert_eq!(markdown("<img alt='' title='Avatar'>"), "");
    assert_eq!(
        markdown("<img src='/thumb.jpg' alt='' title='Gallery'>"),
        "![](/thumb.jpg \"Gallery\")"
    );
    assert_eq!(markdown("<input type=search aria-label='Search'>"), "");
    assert_eq!(
        markdown("<input type=submit aria-label='Search'>"),
        "Search"
    );
}

#[test]
fn authored_image_alternatives_survive_naming_exclusions() {
    for attribute in ["hidden", "aria-hidden=' true '", "role='presentation'"] {
        assert_eq!(
            markdown(&format!(
                "<img src='/c.png' alt='Revenue chart' {attribute}>"
            )),
            "![Revenue chart](/c.png)"
        );
        assert_eq!(
            markdown(&format!(
                "<img src='/c.png' aria-label='Synthetic' {attribute}>"
            )),
            "![](/c.png)"
        );
    }
    assert_eq!(
        markdown("<img src='/c.png' alt='' aria-label='   '>"),
        "![](/c.png)"
    );
    assert_eq!(
        markdown("<video poster=' /cover.jpg '></video>"),
        "![Video poster](/cover.jpg)"
    );
    for poster in ["#cover", "about:blank"] {
        assert_eq!(
            markdown(&format!("<video poster='{poster}'>Fallback</video>")),
            "Fallback"
        );
    }
    assert_eq!(
        markdown("<img src='/fallback.png' srcset='a.png ((x), b.png 2x'>"),
        "![](b.png)"
    );
}

#[test]
fn mixed_currency_inequalities_and_multiline_math_keep_math_payloads() {
    use pulldown_cmark::{Event, Options as MarkdownOptions, Parser};
    for (source, expected) in [
        ("<p>Pay $5 for $x_i$.</p>", "x_i"),
        ("<p>$0&lt;x&lt;1$</p>", "0< x<1"),
        ("<p>\\(n&lt;k\\)</p>", "n< k"),
        ("<p>$$a\n- b$$</p>", "a - b"),
        ("<table><tr><td>$$a\n- b$$</td></tr></table>", "a - b"),
        (r"<p>$$a \% + b$$</p>", r"a \% + b"),
        ("<p>$$a\n# b\n\n&gt; c$$</p>", "a # b > c"),
        ("<p>$$a % a comment\n- b$$</p>", "a - b"),
    ] {
        let result = markdown(source);
        let math: Vec<_> = Parser::new_ext(&result, MarkdownOptions::ENABLE_MATH)
            .filter_map(|event| match event {
                Event::InlineMath(t) | Event::DisplayMath(t) => Some(t.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(math.len(), 1, "{source}: {result}, {math:?}");
        assert_eq!(math[0].trim(), expected, "{result}");
        let html = rendered_html(&result);
        for unwanted in ["<ul>", "<blockquote>", "<h1>"] {
            assert!(!html.contains(unwanted), "{html}");
        }
    }
}

#[test]
fn mathml_operators_survive_commonmark_as_literal_math_text() {
    let result = markdown(
        r"<math><mi>A</mi><mo>\</mo><mi>B</mi><mo>*</mo><mi>C</mi><mo>*</mo><mo>_</mo><mi>D</mi><mo>_</mo><mo>~</mo><mo>`</mo><mo>[</mo></math>",
    );
    let html = rendered_html(&result);
    assert!(html.contains("<mo>\\</mo>"), "{html}");
    for unexpected in ["<em>", "<strong>", "&lt;/mo&gt;"] {
        assert!(!html.contains(unexpected), "{html}");
    }
}

#[test]
fn math_comments_literals_and_inline_markup_keep_distinct_boundaries() {
    use pulldown_cmark::{Event, Options as MarkdownOptions, Parser};
    for (source, expected) in [
        ("<p>$a%b$</p>", "a"),
        ("<p>$5%$</p>", "5"),
        ("<p>$%c\nx$</p>", "x"),
        (r"<p>$\verb|50%|$</p>", r"\verb|50%|"),
        (r"<p>$\verb|a  b|$</p>", r"\verb|a  b|"),
        (r"<p>$\text{a&lt;2}$</p>", r"\text{a<2}"),
        (r"<p>$\mbox{a&lt;2}$</p>", r"\mbox{a<2}"),
    ] {
        let result = markdown(source);
        let math: Vec<_> = Parser::new_ext(&result, MarkdownOptions::ENABLE_MATH)
            .filter_map(|e| match e {
                Event::InlineMath(x) => Some(x.into_string()),
                _ => None,
            })
            .collect();
        assert_eq!(math, vec![expected], "{source}: {result}");
    }
    for source in [
        "<p>$[a](b)$</p>",
        "<p>$x<a href='/d'>l</a>y$</p>",
        "<math><mi>$</mi><mn>5</mn><mo>-</mo><mi>$</mi><mn>3</mn></math>",
    ] {
        let result = markdown(source);
        assert!(
            !Parser::new_ext(&result, MarkdownOptions::ENABLE_MATH)
                .any(|e| matches!(e, Event::InlineMath(_) | Event::DisplayMath(_))),
            "{result}"
        );
        if source.contains("href") {
            assert!(rendered_html(&result).contains("href=\"/d\""), "{result}");
        }
    }
}

#[test]
fn tex_delimiters_control_spaces_and_empty_comments_survive_math_readers() {
    use pulldown_cmark::{Event, Options as O, Parser};
    for (source, expected, display) in [
        (r"<p>\(x_i\)2</p>", "x_i", false),
        (r"<p>\[y_1=2\]</p>", "y_1=2", true),
        (r"<p>$$a\ $$</p>", "a\\ ", true),
        (r"<p>\(x\ \)</p>", "x\\ {}", false),
        (r"<p>$\textrm{a&lt;2}$</p>", r"\textrm{a<2}", false),
        (r"<p>$\text {a&lt;2}$</p>", r"\text{a<2}", false),
    ] {
        let result = markdown(source);
        let payloads: Vec<_> = Parser::new_ext(&result, O::ENABLE_MATH)
            .filter_map(|e| match e {
                Event::InlineMath(v) => Some((v.into_string(), false)),
                Event::DisplayMath(v) => Some((v.into_string(), true)),
                _ => None,
            })
            .collect();
        assert_eq!(
            payloads,
            vec![(expected.to_owned(), display)],
            "{source}: {result}"
        );
    }
    for source in [
        r"<p>$%a$ b $%c$</p>",
        r"<p>$\text{&lt;img src=x onerror=alert(1)&gt;}$</p>",
        r"<p>$\textbf{&lt;b&gt;x}$</p>",
        r"<p>$\hbox {&lt;b&gt;}$</p>",
        r"<p>$\text{a{&lt;b&gt;}}$</p>",
        r"<p>$\text{&lt;b&gt;$</p>",
        r"<p>$\verb*|&lt;b&gt;|$</p>",
        r"<p>$\verb&lt;b&gt;x&lt;$</p>",
        r"<p>$\verb*&lt;img src=x&gt;a&lt;$</p>",
        r"<p>$\verb|&lt;b&gt;|$</p>",
        "<p>$$\\verb|a\n- b$$</p>",
    ] {
        let result = markdown(source);
        assert!(
            !Parser::new_ext(&result, O::ENABLE_MATH)
                .any(|e| matches!(e, Event::InlineMath(_) | Event::DisplayMath(_))),
            "{result}"
        );
        let html = rendered_html(&result);
        assert!(
            !html.contains("<img") && !html.contains("<b>") && !html.contains("<li>"),
            "{html}"
        );
    }
}

#[test]
fn adjacent_math_keeps_independent_delimiter_runs() {
    for (source, expected) in [
        (r"<p>\(a\)\(b\)</p>", "$a$<!-- -->$b$"),
        (r"<p>$a$\(b\)</p>", "$a$<!-- -->$b$"),
        (r"<p>\[a\]\[b\]</p>", "$$a$$<!-- -->$$b$$"),
    ] {
        assert_eq!(markdown(source), expected);
    }
}

#[test]
fn math_boundaries_survive_empty_wrappers_and_coalesced_styles() {
    use pulldown_cmark::{Event, Options, Parser};
    for source in [
        r"<p>\(a\)<b></b>\(b\)</p>",
        r"<p><b>\(a\)</b><b>\(b\)</b></p>",
        r"<p>$a$<i></i>$b$</p>",
    ] {
        let result = markdown(source);
        let values: Vec<_> = Parser::new_ext(&result, Options::ENABLE_MATH)
            .filter_map(|e| match e {
                Event::InlineMath(v) => Some(v.into_string()),
                _ => None,
            })
            .collect();
        assert_eq!(values, ["a", "b"], "{source}: {result}");
        assert!(result.contains("$<!-- -->$"), "{result}");
    }
    for source in [
        r"<p>\(x\)2</p>",
        r"<p>\(x\)<i></i>2</p>",
        r"<p>$x$<i></i>2</p>",
    ] {
        let result = markdown(source);
        assert_eq!(result, "$x$<!-- -->2", "{source}");
        assert!(
            Parser::new_ext(&result, Options::ENABLE_MATH)
                .any(|e| matches!(e, Event::InlineMath(_)))
        );
    }
}
