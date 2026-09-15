//! Open review findings, separate from regressions asserting desired behavior.
//! These tests run normally and fail if the recorded behavior changes. When a
//! gap is fixed, replace its current-output assertion with the desired result
//! and update turndown/README.md; do not refresh the snapshot automatically.

mod support;

use moli_html2md::convert;
use support::{Tree, rendered_html};

fn markdown(html: &str) -> String {
    let dom = Tree::parse(html);
    let before = dom.clone();
    let result = convert(&dom, dom.root);
    assert_eq!(dom, before);
    result
}

fn track_list_spacing(html: &str) {
    let result = markdown(html);
    assert_eq!(result, "- before\n  - inner\n- after");
    assert_eq!(
        rendered_html(&result),
        "<ul>\n<li>before\n<ul>\n<li>inner</li>\n</ul>\n</li>\n<li>after</li>\n</ul>\n"
    );
    // Output from the pinned Turndown 7.2.4 reference. Whether these wrappers
    // should make the outer list loose remains a compatibility decision.
    let turndown = "-   before\n    \n    -   inner\n    \n-   after";
    assert_eq!(
        rendered_html(turndown),
        "<ul>\n<li>\n<p>before</p>\n<ul>\n<li>inner</li>\n</ul>\n</li>\n<li>\n<p>after</p>\n</li>\n</ul>\n"
    );
    assert_ne!(rendered_html(&result), rendered_html(turndown));
}

#[test]
fn track_nested_list_spacing_through_a_transparent_wrapper() {
    track_list_spacing("<ul><li>before<ins><ul><li>inner</li></ul></ins></li><li>after</li></ul>");
}

#[test]
fn track_nested_list_spacing_before_a_trailing_empty_element() {
    track_list_spacing(
        "<ul><li>before<ul><li>inner</li></ul><span></span></li><li>after</li></ul>",
    );
}

#[test]
fn track_multiline_image_alt_becoming_a_heading() {
    let result = markdown("<img src='/x' alt='first\n# heading'>");
    // Turndown emits the same Markdown. Matching it does not preserve the image.
    assert_eq!(result, "![first\n# heading](/x)");
    let actual_html = rendered_html(&result);
    assert_eq!(actual_html, "<p>![first</p>\n<h1>heading](/x)</h1>\n");
    let desired_html = "<p><img src=\"/x\" alt=\"first # heading\" /></p>\n";
    assert_ne!(
        actual_html, desired_html,
        "review the resolved image-alt gap"
    );
}

#[test]
fn track_multiline_link_title_becoming_a_heading() {
    let result = markdown("<a href='/' title='first\n# heading'>link</a>");
    // Turndown has this loss too; the link and its title must survive rendering.
    assert_eq!(result, "[link](/ \"first\n# heading\")");
    let actual_html = rendered_html(&result);
    assert_eq!(
        actual_html,
        "<p>[link](/ \"first</p>\n<h1>heading\")</h1>\n"
    );
    let desired_html = "<p><a href=\"/\" title=\"first\n# heading\">link</a></p>\n";
    assert_ne!(
        actual_html, desired_html,
        "review the resolved link-title gap"
    );
}

#[test]
fn track_multiple_nested_spans_at_the_end_of_a_paragraph() {
    // This predates the closing-edge fix: with no following character there
    // is no punctuation fallback, but sibling delimiters can still mispair.
    let result = markdown("<em><strong>x</strong>b<strong>c</strong></em>");
    assert_eq!(result, "***x**b**c***");
    let actual_html = rendered_html(&result);
    assert_eq!(
        actual_html,
        "<p><em><strong>x</strong>b</em><em>c</em>**</p>\n"
    );
    let desired_html = "<p><em><strong>x</strong>b<strong>c</strong></em></p>\n";
    assert_ne!(
        actual_html, desired_html,
        "review the resolved sibling-span gap"
    );
}

#[test]
fn track_mixed_emphasis_markers_at_an_intraword_opening() {
    // The opening check sees the text 'x', although the nested styles emit
    // punctuation first. This is separate from closing a same-marker run.
    let result = markdown("before<em><del><strong>x</strong></del></em>");
    assert_eq!(result, "before*~~**x**~~*");
    let actual_html = rendered_html(&result);
    assert_eq!(
        actual_html,
        "<p>before*<del><strong>x</strong></del>*</p>\n"
    );
    let desired_html = "<p>before<em><del><strong>x</strong></del></em></p>\n";
    assert_ne!(
        actual_html, desired_html,
        "review the resolved mixed-marker gap"
    );
}
