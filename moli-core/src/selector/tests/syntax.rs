use super::{QueryEngine, SelectorErrorKind, find_by_id, parse_document};
use crate::dom::native::DomHost;

#[test]
fn rejects_invalid_selector_syntax_edges() {
    let engine = QueryEngine;

    for selector in [
        "",
        "div,",
        ",div",
        "div,,span",
        "[attr!=value]",
        "p >",
        "[data-test=",
    ] {
        let result =
            engine.query_selector_all(&parse_document("<html><body></body></html>"), selector);
        assert!(
            result.is_err(),
            "selector {selector:?} should fail but returned Ok"
        );
        assert_eq!(
            result.unwrap_err().kind(),
            SelectorErrorKind::Syntax,
            "{selector}"
        );
    }
}

#[test]
fn duplicate_id_compounds_are_valid_selector_syntax() {
    let engine = QueryEngine;
    let document = parse_document("<html><body><div id='a'></div></body></html>");

    let repeated_same_id = engine
        .query_selector_all(&document, "div#a#a")
        .expect("duplicate id selectors are valid CSS selector syntax");
    assert_eq!(repeated_same_id, vec![find_by_id(&document, "a")]);

    let conflicting_ids = engine
        .query_selector_all(&document, "div#a#b")
        .expect("conflicting id selectors are valid but do not match");
    assert!(conflicting_ids.is_empty());
}

#[test]
fn rejects_more_upstream_invalid_selector_shapes() {
    let document = parse_document(
        "<!doctype html><html><body><div id='container'><p>Test</p></div></body></html>",
    );
    let engine = QueryEngine;

    for selector in [
        ":has()",
        ":not()",
        ":lang()",
        ":nth-child(foo)",
        ":nth-child(-)",
        ":nth-child(+)",
        ":unknown",
        ":not-a-real-pseudo",
        ":fake(test)",
        "p +",
        "p ~",
    ] {
        let error = engine
            .query_selector_all(&document, selector)
            .expect_err("selector should be rejected");
        assert_eq!(error.kind(), SelectorErrorKind::Syntax, "{selector}");
    }
}

// ---------------------------------------------------------------------------
// Selector syntax rejection – additional edge cases beyond the basics
// ---------------------------------------------------------------------------

#[test]
fn rejects_additional_invalid_syntax() {
    let engine = QueryEngine;
    let doc = parse_document("<html><body></body></html>");

    for selector in [
        // Unclosed paren
        ":not(",
        // Consecutive combinators
        "div > > span",
        // Unknown pseudo-class
        ":foobar",
        // :lang() with no argument
        ":lang()",
        // Empty :not()
        ":not()",
        // Unsupported attribute operator
        "[attr!=value]",
    ] {
        let result = engine.query_selector_all(&doc, selector);
        assert!(
            result.is_err(),
            "selector {selector:?} should fail but returned Ok"
        );
        assert_eq!(
            result.unwrap_err().kind(),
            SelectorErrorKind::Syntax,
            "wrong error kind for {selector:?}"
        );
    }
}

#[test]
fn known_terminal_pseudo_elements_are_valid_empty_dom_api_results() {
    let engine = QueryEngine;
    let doc = parse_document("<html><body><p id='target'></p></body></html>");

    for selector in ["::before", "p::after"] {
        let result = engine
            .query_selector_all(&doc, selector)
            .expect("known pseudo-element selector should be valid");
        assert!(
            result.is_empty(),
            "selector {selector:?} should produce an empty DOM API result"
        );
    }
}

#[test]
fn matches_and_closest_reject_empty_selector_syntax() {
    let document = parse_document(
        r#"<!doctype html><html><body><div id="target" class="box"></div></body></html>"#,
    );
    let host = DomHost::from_dom(document.clone());
    let engine = QueryEngine;
    let target = find_by_id(&document, "target");

    for result in [
        engine.matches(&document, target, ""),
        engine.matches_with_scope(&document, target, "", target),
    ] {
        let error = result.expect_err("empty selector should fail");
        assert_eq!(error.kind(), SelectorErrorKind::Syntax);
    }

    for result in [
        engine.matches_host(&host, target, ""),
        engine.matches_with_scope_host(&host, target, "", target),
    ] {
        let error = result.expect_err("empty selector should fail");
        assert_eq!(error.kind(), SelectorErrorKind::Syntax);
    }

    for result in [
        engine.closest(&document, target, ""),
        engine.closest_host(&host, target, ""),
    ] {
        let error = result.expect_err("empty selector should fail");
        assert_eq!(error.kind(), SelectorErrorKind::Syntax);
    }
}
