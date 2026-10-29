//! Open compatibility findings; corrected behavior lives in regression tests.

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
