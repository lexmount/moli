mod support;

use moli_html2md::{Converter, Dom, Options, convert};
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
    fn has_block_layout(&self, node: usize) -> bool {
        self.tree.attribute(node, "style") == Some("display:block")
    }
    fn has_text_boundary(&self, node: usize) -> bool {
        self.tree.attribute(node, "style") == Some("display:inline-block")
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
fn local_image_sources_are_preserved_before_lazy_loading_fallbacks() {
    for (source, expected) in [
        (
            "<img src='file:///tmp/chart.png' alt='Chart'>",
            "![Chart](file:///tmp/chart.png)",
        ),
        (
            "<img src='file:///tmp/chart.png' data-src='/old.png' alt='Chart'>",
            "![Chart](file:///tmp/chart.png)",
        ),
        (
            "<img src='file:///tmp/chart.png'>",
            "![](file:///tmp/chart.png)",
        ),
        (
            "<img srcset='file:///tmp/small.png 1x, file:///tmp/chart.png 2x' src='/fallback.png' data-src='/old.png' alt='Chart'>",
            "![Chart](file:///tmp/chart.png)",
        ),
    ] {
        assert_eq!(markdown(source), expected, "{source}");
    }
}

#[test]
fn input_values_remain_distinct_from_adjacent_fields_and_labels() {
    for (source, expected) in [
        (
            "<input name='min' value='10'><input name='max' value='20'>",
            "10 20",
        ),
        (
            "<label>Min<input value='10'></label><label>Max<input value='20'></label>",
            "Min 10 Max 20",
        ),
        ("before<input value='10'>after", "before 10 after"),
        (
            "<strong><input value='10'></strong><em><input value='20'></em>",
            "**10** *20*",
        ),
        (
            "<label><input type='checkbox' checked>Enabled</label><label><input type='checkbox'>Disabled</label>",
            "☑ Enabled ☐ Disabled",
        ),
    ] {
        assert_eq!(markdown(source), expected, "{source}");
    }
}

#[test]
fn image_buttons_keep_alternative_names_without_using_submission_values() {
    for attributes in [
        "alt='Search' aria-label='Other' title='Hint'",
        "aria-label='Search' title='Hint'",
        "title='Search'",
        "alt='' aria-label='Search'",
        "alt='' title='Search'",
        "alt=' \t' aria-label='Search'",
        "aria-label=' \t' title='Search'",
    ] {
        let source =
            format!("<input type='image' src='/search.png' value='Submission value' {attributes}>");
        assert_eq!(markdown(&source), "Search", "{source}");
    }
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
fn code_controls_are_omitted_in_both_markdown_and_html_paths() {
    for source in [
        "<pre><code>run();<a href='/api'>call()</a><button>Copy</button></code></pre>",
        "<table><tr><td colspan='2'><pre><code>run();<a href='/api'>call()</a><button>Copy</button></code></pre></td></tr></table>",
    ] {
        let result = markdown(source);
        assert!(result.contains("run();"), "{result}");
        assert!(result.contains("call()"), "{result}");
        assert!(result.contains("/api"), "{result}");
        assert!(!result.contains("Copy"), "{result}");
    }
}

#[test]
fn referenced_targets_do_not_turn_presentational_tables_into_data_tables() {
    let result = markdown(
        "<a href='#details'>Details</a><table role='presentation' id='details'>\
         <tr><td><h2>Heading</h2></td><td>Content</td></tr></table>",
    );
    assert!(!result.contains("<table"), "{result}");
    assert!(result.contains("id=\"details\""), "{result}");
    assert!(result.contains("Heading"), "{result}");
    assert!(result.contains("Content"), "{result}");
}

#[test]
fn anchored_nested_tables_keep_linear_dom_work() {
    let count = |depth| {
        let source = format!(
            "<a href='#target'>Target</a>{}<span id='target'>Value</span>{}",
            "<table><tr><td>".repeat(depth),
            "</td></tr></table>".repeat(depth)
        );
        let dom = Counted {
            tree: Tree::parse(&source),
            visits: Cell::new(0),
        };
        let result = moli_html2md::Converter::new(moli_html2md::Options {
            max_depth: usize::MAX,
            ..Default::default()
        })
        .convert(&dom, dom.tree.root);
        assert!(result.contains("id=\"target\""), "{result}");
        assert_eq!(result.matches("Value").count(), 1);
        dom.visits.get()
    };
    let small = count(32);
    let large = count(128);
    assert!(large <= small * 5, "{small} -> {large}");
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
fn empty_option_labels_keep_their_text_and_choice_boundaries() {
    assert_eq!(
        markdown(
            "<select><option label=''>First choice</option><option label='Public label'>Internal label</option><optgroup label=''><option label=''>Third choice</option></optgroup></select>"
        ),
        "First choice\nPublic label\nThird choice"
    );
}

#[test]
fn empty_default_textareas_do_not_emit_accessible_names() {
    for attributes in [
        "aria-label='Comment'",
        "title='Comment'",
        "aria-label='Comment' role='textbox'",
    ] {
        for content in ["", " \n\t"] {
            let html = format!("<textarea {attributes}>{content}</textarea>");
            assert_eq!(markdown(&html), "", "{html}");
        }
        let html = format!("<textarea {attributes}>Authored text</textarea>");
        assert_eq!(markdown(&html), "Authored text", "{html}");
    }
    assert_eq!(
        markdown("<label>Comment<textarea aria-label='Hint'></textarea></label>"),
        "Comment"
    );
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
fn svg_title_fallback_obeys_the_conversion_depth_limit() {
    let dom = Tree::parse("<svg><title>Chart title</title><path/></svg>");
    let svg = dom.first_child(dom.root).unwrap();
    for (root, svg_depth) in [(svg, 0), (dom.root, 1)] {
        for max_depth in [0, 1, 2, 3, 4, usize::MAX] {
            let converter = Converter::new(Options {
                max_depth,
                ..Default::default()
            });
            let expected = if max_depth > svg_depth + 2 {
                "Chart title"
            } else {
                ""
            };
            assert_eq!(
                converter.convert(&dom, root),
                expected,
                "svg_depth={svg_depth}, max_depth={max_depth}"
            );
        }
    }
    for attribute in ["aria-label", "title"] {
        let dom = Tree::parse(&format!(
            "<svg {attribute}='Authored name'><title>Child title</title></svg>"
        ));
        let svg = dom.first_child(dom.root).unwrap();
        assert_eq!(
            Converter::new(Options {
                max_depth: 1,
                ..Default::default()
            })
            .convert(&dom, svg),
            "Authored name"
        );
    }
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
                format!(
                    "<textarea data-live='{value}' aria-label='Field' title='Hint' role='textbox'>Stale</textarea>"
                )
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

#[test]
fn invalid_and_default_cell_spans_keep_simple_tables_in_markdown() {
    for attr in ["colspan", "rowspan"] {
        for value in ["", "invalid", "-2", "1", " 1 ", "+1", "1tail"] {
            let result = markdown(&format!(
                "<table><tr><th>A</th><th>B</th></tr><tr><td {attr}='{value}'>X</td><td>Y</td></tr></table>"
            ));
            assert!(!result.contains("<table"), "{attr}={value}: {result}");
            assert!(result.contains("X") && result.contains("Y"), "{result}");
        }
    }
    for attr in ["rowspan='0'", "colspan='2'", "rowspan='2'"] {
        let result = markdown(&format!(
            "<table><tr><td {attr}>Merged</td><td>Next</td></tr></table>"
        ));
        assert!(result.contains("<table"), "{attr}: {result}");
    }
}

#[test]
fn html_media_omits_empty_posters_and_redundant_css_wrappers() {
    let result = markdown(
        "<table><tr><td colspan='2'><video src='/movie.mp4' poster=''></video></td></tr></table>",
    );
    assert!(result.contains("movie.mp4"), "{result}");
    assert!(!result.contains("<img"), "{result}");
}

#[test]
fn html_keeps_css_boundaries_without_wrapping_existing_block_tags() {
    let dom = Counted {
        tree: Tree::parse(
            "<table style='display:block'><tr><td colspan='2' style='display:block'><p style='display:block'>Paragraph</p><span style='display:block'>Box</span></td></tr></table>",
        ),
        visits: Cell::new(0),
    };
    let result = convert(&dom, dom.tree.root);
    assert!(!result.starts_with("<div>"), "{result}");
    assert!(!result.contains("<div><p"), "{result}");
    assert!(result.contains("<div>Box</div>"), "{result}");
}

#[test]
fn complex_layouts_expand_but_semantic_data_tables_keep_cell_structure() {
    let result = markdown(
        "<table><tr><td><h2>Article</h2><table><tr><td>First story</td></tr><tr><td>Second story</td></tr></table></td></tr></table>",
    );
    assert!(!result.contains("<table"), "{result}");
    for text in ["## Article", "First story", "Second story"] {
        assert!(result.contains(text), "{result}");
    }
    for source in [
        "<table><tr><th>Name</th><th>Items</th></tr><tr><td>Ada</td><td><ul><li>One</li><li>Two</li></ul></td></tr></table>",
        "<table><caption>Results</caption><tr><td rowspan='2'>A</td><td>B</td></tr><tr><td>C</td></tr></table>",
    ] {
        let result = markdown(source);
        assert!(result.contains("<table"), "{result}");
    }
}

#[test]
fn linked_preformatted_source_keeps_language_lines_and_links() {
    let result = markdown(
        "<pre><code class='language-rust'>let x = <a href='/type'>Type</a>;\n\nlet y = x &amp; 1;\n</code></pre>",
    );
    assert!(result.contains("language-rust"), "{result}");
    assert!(result.contains("\n\nlet y"), "{result}");
    assert!(result.contains("href=\"/type\""), "{result}");
    assert!(!result.contains("&#10;"), "{result}");
}

#[test]
fn component_heading_wrappers_remain_markdown_headings() {
    let result = markdown(
        "<h2><div>Title <span>text</span></div></h2><h3><a href='/story'><div>Story</div></a></h3>",
    );
    assert_eq!(result, "## Title text\n\n### [Story](/story)");
}

#[test]
fn referenced_targets_belong_to_their_captured_blocks() {
    let result = markdown(
        "<a href='#second'>Second</a><a href='#title'>Title</a><a href='#quote'>Quote</a><ul><li>Alpha</li><li id='second'>Beta</li></ul><h2 id='title'>Heading</h2><blockquote id='quote'>Quoted text</blockquote>",
    );
    let html = rendered_html(&result);
    assert!(
        html.contains("<li><span id=\"second\"></span>Beta</li>"),
        "{html}"
    );
    assert!(
        html.contains("<h2><span id=\"title\"></span>Heading</h2>"),
        "{html}"
    );
    assert!(
        html.contains("<blockquote>\n<p><span id=\"quote\"></span>Quoted text</p>"),
        "{html}"
    );
}

#[test]
fn nested_heading_wrappers_have_linear_work_and_keep_every_title() {
    let count = |depth| {
        let html = format!(
            "{}Title{}",
            "<h2><div>".repeat(depth),
            "</div></h2>".repeat(depth)
        );
        let dom = Counted {
            tree: Tree::parse(&html),
            visits: Cell::new(0),
        };
        let result = convert(&dom, dom.tree.root);
        assert_eq!(result.matches("Title").count(), 1, "{result}");
        dom.visits.get()
    };
    let small = count(20);
    let large = count(80);
    assert!(large <= small * 5, "{small} -> {large}");
}

#[test]
fn table_and_code_root_targets_exist_once_in_every_output_shape() {
    for element in [
        "<table id='target'><tr><td>a</td><td>b</td></tr></table>",
        "<table id='target'><tr><td><h2>Article</h2><p>Body</p></td></tr></table>",
        "<table id='target'><caption>Data</caption><tr><td colspan='2'>Body</td></tr></table>",
        "<pre id='target'>source</pre>",
        "<code id='target'>source</code>",
    ] {
        let result = markdown(&format!("<a href='#target'>Go</a>{element}"));
        assert_eq!(result.matches("id=\"target\"").count(), 1, "{result}");
        assert!(result.contains("#target"), "{result}");
    }
    let result = markdown(
        "<a href='#keep'>Go</a><table id='unused'><caption>Data</caption><tr><td id='keep'><pre id='noise'><span id='L2'>Line</span><a name='obsolete'>link</a></pre></td></tr></table>",
    );
    assert_eq!(result.matches("id=\"keep\"").count(), 1, "{result}");
    for unwanted in ["unused", "noise", "L2", "obsolete"] {
        assert!(!result.contains(unwanted), "{result}");
    }
}

#[test]
fn linked_source_roundtrips_punctuation_leading_lines_and_default_language() {
    use moli_html2md::{Converter, Options};
    let source =
        "<pre>\n\nfn new_from_iter&lt;I&gt;(v: &amp;[T]) { *x }\n<a href='/ref'>name</a>\n</pre>";
    let dom = Tree::parse(source);
    let result = Converter::new(Options {
        default_code_language: Some("rust".into()),
        ..Options::default()
    })
    .convert(&dom, dom.root);
    assert!(result.contains("language-rust"), "{result}");
    for entity in ["&#95;", "&#42;", "&#91;"] {
        assert!(!result.contains(entity), "{result}");
    }
    let rendered = Tree::parse(&rendered_html(&result));
    let text = |tree: &Tree| {
        tree.nodes
            .iter()
            .filter_map(|n| n.text.as_deref())
            .collect::<String>()
    };
    assert_eq!(text(&rendered), text(&dom), "{result}");
}

#[test]
fn merged_headerless_data_is_preserved_without_treating_empty_hints_as_headers() {
    let result =
        markdown("<table><tr><td rowspan='2'>Group</td><td>A</td></tr><tr><td>B</td></tr></table>");
    assert!(result.contains("rowspan=\"2\""), "{result}");
    let result = markdown("<table><tr><td scope='' headers=' '><h2>Article</h2></td></tr></table>");
    assert_eq!(result, "## Article");
    let result = markdown(
        "<table><caption>Resources</caption><tr><td colspan='2'><video poster='/cover.jpg'></video><img src='' aria-label='Home'></td></tr></table>",
    );
    assert!(result.contains("/cover.jpg"), "{result}");
    assert!(result.contains("Home"), "{result}");
}

#[test]
fn fallback_targets_and_media_preserve_navigation_without_duplicate_ids() {
    for html in [
        "<a href='#fig'>F</a><table><caption>C</caption><tr><td><img id='fig' src='/a.png' alt='Chart' title='Details'></td></tr></table>",
        "<a href='#note'>N</a><table><caption>C</caption><tr><td><textarea id='note'>x</textarea></td></tr></table>",
        "<a href='#x'>X</a><p id='x'>P</p><table><caption>C</caption><tr><td id='x'>Cell</td></tr></table>",
    ] {
        let output = markdown(html);
        let target = if html.contains("#fig") {
            "fig"
        } else if html.contains("#note") {
            "note"
        } else {
            "x"
        };
        assert_eq!(
            output.matches(&format!("id=\"{target}\"")).count(),
            1,
            "{output}"
        );
        if target == "fig" {
            assert!(output.contains("title=\"Details\""), "{output}");
        }
    }
    let output = markdown(
        "<a href='#one'>1</a><a href='#two'>2</a><table><caption>C</caption><tr><td><a id='one' name='two' href='/x'>linked text</a><span id='unreferenced'>plain</span><video poster=' ' src='/v.mp4'></video></td></tr></table>",
    );
    let html = rendered_html(&output);
    assert_eq!(output.matches("id=\"two\"").count(), 1, "{output}");
    assert!(
        output.find("id=\"two\"").unwrap() < output.find("href=\"/x\"").unwrap(),
        "{output}"
    );

    assert!(
        html.contains("href=\"/x\" id=\"one\">linked text</a>"),
        "{html}"
    );
    assert!(
        !output.contains("<span>plain") && !output.contains("<img"),
        "{output}"
    );
    for span in ["", "colspan='2'"] {
        let output = markdown(&format!(
            "<table><caption>C</caption><tr><td {span}><video poster='/cover.jpg' src='blob:id'><a href='/v.mp4'>Download</a></video></td></tr></table>"
        ));
        let html = rendered_html(&output);
        assert!(
            html.contains("/cover.jpg") && html.contains("href=\"/v.mp4\""),
            "{output}"
        );
    }
}

#[test]
fn computed_boundaries_supplement_structure_without_rewriting_code() {
    for (source, expected) in [
        (
            "<ul><li style='display:block'>A</li><li style='display:block'>B</li></ul>",
            "- A\n- B",
        ),
        (
            "<ul><li><a href='/a' style='display:block'>A</a></li><li><span style='display:block'>B</span></li></ul>",
            "- [A](/a)\n- B",
        ),
        (
            "<select><option style='display:block'>A</option><option style='display:block'>B</option></select>",
            "A\nB",
        ),
    ] {
        let dom = Counted {
            tree: Tree::parse(source),
            visits: Cell::new(0),
        };
        assert_eq!(convert(&dom, dom.tree.root), expected, "{source}");
    }
    for layout in ["display:inline-block", "display:block"] {
        let source = format!(
            "<pre><code>let v = <span style='{layout}'>f</span>(<a href='/doc'>x</a>);</code></pre>"
        );
        let dom = Counted {
            tree: Tree::parse(&source),
            visits: Cell::new(0),
        };
        let result = convert(&dom, dom.tree.root);
        assert!(
            result.contains("let v = f(<a href=\"/doc\">x</a>);"),
            "{result}"
        );
        assert!(!result.contains("<div>"), "{result}");
    }
}

#[test]
fn role_tokens_use_html_spaces_and_media_names_prefer_accessible_labels() {
    for role in ["presentation", "none"] {
        let source = format!(
            "<table role='\u{a0}{role}\u{a0}'><tr><th>H</th></tr><tr><td colspan='2'>X</td></tr></table>"
        );
        assert!(markdown(&source).contains("<table>"));
    }
    for tag in ["audio", "video", "iframe"] {
        let result = markdown(&format!(
            "<{tag} src='/media' title='Tooltip' aria-label='Description'></{tag}>"
        ));
        assert!(result.contains("[Description](/media)"), "{result}");
    }
}

#[test]
fn independent_inline_boxes_request_spaces_through_the_adapter_contract() {
    let dom = Counted {
        tree: Tree::parse(
            "<span style='display:inline-block'>A</span><span style='display:inline-block'>B</span>",
        ),
        visits: Cell::new(0),
    };
    assert_eq!(convert(&dom, dom.tree.root), "A B");
}

#[test]
fn fallback_keeps_code_boxes_and_ignores_column_boxes() {
    for (style, expected) in [
        ("display:inline-block", "run <code>ls</code> now"),
        ("display:block", "run<div><code>ls</code></div>now"),
    ] {
        let source = format!(
            "<table><caption>C</caption><colgroup style='display:block'><col style='display:block'></colgroup><tr><td colspan='2'>run<code style='{style}'>ls</code>now</td></tr></table>"
        );
        let dom = Counted {
            tree: Tree::parse(&source),
            visits: Cell::new(0),
        };
        let result = convert(&dom, dom.tree.root);
        assert!(result.contains(expected), "{result}");
        assert_eq!(
            result.matches("<div>").count(),
            usize::from(style == "display:block"),
            "{result}"
        );
    }
}

#[test]
fn stripped_block_elements_keep_their_computed_boundaries_in_tables() {
    let content =
        "<output style='display:block'>A</output><output style='display:block'>B</output>";
    for source in [
        format!("<div>{content}</div>"),
        format!("<table><tr><td>{content}</td><td>Other</td></tr></table>"),
        format!("<table><tr><td colspan='2'>{content}</td></tr></table>"),
    ] {
        let dom = Counted {
            tree: Tree::parse(&source),
            visits: Cell::new(0),
        };
        let markdown = convert(&dom, dom.tree.root);
        let output = Tree::parse(&rendered_html(&markdown));
        let mut text = String::new();
        for node in &output.nodes {
            if matches!(node.tag.as_deref(), Some("p" | "div" | "td" | "br")) {
                text.push(' ');
            }
            if let Some(value) = &node.text {
                text.push_str(value);
            }
        }
        assert!(
            text.split_whitespace()
                .collect::<Vec<_>>()
                .windows(2)
                .any(|words| words == ["A", "B"]),
            "{source}: {markdown}\n{text}"
        );
    }
}

#[test]
fn content_after_a_nested_list_can_make_the_outer_list_loose() {
    assert_eq!(
        markdown("<ul><li>a<ul><li>b</li></ul>c</li><li>d</li></ul>"),
        "- a\n  - b\n\n  c\n\n- d"
    );
    assert_eq!(
        markdown("<ul><li>a<ul><li>b</li></ul></li><li>d</li></ul>"),
        "- a\n  - b\n- d"
    );
}
