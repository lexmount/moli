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
