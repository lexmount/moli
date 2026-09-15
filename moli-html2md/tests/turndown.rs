mod support;

use std::sync::OnceLock;

use moli_html2md::{Converter, Options};
use serde_json::Value;
use support::{Tree, rendered_html};

fn corpus() -> &'static Value {
    static CORPUS: OnceLock<Value> = OnceLock::new();
    CORPUS.get_or_init(|| {
        serde_json::from_str(include_str!("turndown/cases.json")).expect("valid reference corpus")
    })
}

fn expectations() -> &'static Value {
    static EXPECTATIONS: OnceLock<Value> = OnceLock::new();
    EXPECTATIONS.get_or_init(|| {
        serde_json::from_str(include_str!("turndown/expectations.json"))
            .expect("valid intentional differences")
    })
}

fn check_case(index: usize) {
    let case = &corpus()["cases"][index];
    let input = case["html"].as_str().expect("HTML fixture");
    let expected = case["markdown"].as_str().expect("reference Markdown");
    let dom = Tree::parse(input);
    let before = dom.clone();
    let converter = Converter::new(Options {
        preformatted_code: case["preformatted_code"].as_bool().unwrap_or(false),
        ..Options::default()
    });
    let actual = converter.convert(&dom, dom.root);
    assert_eq!(dom, before, "conversion modified DOM");
    let difference = &expectations()[case["id"].as_str().expect("fixture ID")];
    if let Some(markdown) = difference["markdown"].as_str() {
        assert!(
            !difference["reason"]
                .as_str()
                .expect("documented difference")
                .is_empty()
        );
        assert_ne!(
            rendered_html(markdown),
            rendered_html(expected),
            "review an obsolete difference"
        );
        assert_eq!(actual, markdown, "{}: {}", case["id"], difference["reason"]);
        return;
    }
    let expected_html = rendered_html(expected);
    let actual_html = rendered_html(&actual);
    if actual_html != expected_html {
        println!(
            "DIFF {}",
            serde_json::json!({
                "id": case["id"], "name": case["name"], "input": input,
                "expected": expected, "actual": actual,
                "expected_html": expected_html, "actual_html": actual_html,
            })
        );
    }
    assert_eq!(
        actual_html, expected_html,
        "case {}: {}",
        case["id"], case["name"]
    );
}

macro_rules! case {
    ($name:ident, $index:expr) => {
        #[test]
        fn $name() {
            check_case($index);
        }
    };
}

include!("turndown/cases.rs");

#[test]
fn every_intentional_difference_names_a_unique_existing_case() {
    let mut ids = std::collections::HashSet::new();
    for case in corpus()["cases"].as_array().expect("cases") {
        assert!(ids.insert(case["id"].as_str().expect("fixture ID")));
    }
    for id in expectations().as_object().expect("difference map").keys() {
        assert!(ids.contains(id.as_str()), "unused difference: {id}");
    }
}
