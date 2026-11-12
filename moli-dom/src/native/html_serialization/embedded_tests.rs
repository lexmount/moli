use super::*;
use crate::native::{DomHost, ShadowRootInit};
use std::cell::Cell;

#[derive(Default)]
struct Documents {
    entries: Vec<(NativeNodeId, NativeNodeId, String)>,
    lookups: Cell<usize>,
}
impl HtmlEmbeddedDocumentProvider for Documents {
    fn sole_owner(&self) -> Option<NativeNodeId> {
        (self.entries.len() == 1).then(|| self.entries[0].0)
    }

    fn embedded_document(&self, owner: NativeNodeId) -> Option<HtmlEmbeddedDocument<'_>> {
        self.lookups.set(self.lookups.get() + 1);
        self.entries
            .iter()
            .find(|entry| entry.0 == owner)
            .map(|entry| HtmlEmbeddedDocument {
                document: entry.1,
                url: Cow::Borrowed(&entry.2),
            })
    }
}
fn host() -> DomHost {
    DomHost::from_dom(NativeDom::new_html(
        url::Url::parse("https://export.test/").unwrap(),
    ))
}
fn child(host: &mut DomHost, parent: NativeNodeId, name: &str) -> NativeNodeId {
    let node = host.create_element(name);
    assert!(host.append_child(parent, node));
    node
}
fn text(host: &mut DomHost, parent: NativeNodeId, value: &str) {
    let node = host.create_text_node(value);
    assert!(host.append_child(parent, node));
}
fn escape(value: &str) -> String {
    let mut output = String::new();
    escape_html_attribute_into_string(value, &mut output);
    output
}
fn export(
    host: &DomHost,
    docs: &Documents,
    limit: Option<usize>,
) -> Result<String, HtmlSerializationError> {
    host.get_html_for_export(host.document_handle(), &|_| true, None, Some(docs), limit)
}

#[test]
fn single_owner_skips_unrelated_nodes_without_skipping_live_content() {
    let mut host = host();
    let root = host.document_handle();
    for _ in 0..1000 {
        let p = child(&mut host, root, "p");
        text(&mut host, p, "ordinary");
    }
    let owner = child(&mut host, root, "iframe");
    let document = host.create_detached_html_document();
    let p = child(&mut host, document, "p");
    text(&mut host, p, "live & child");
    let docs = Documents {
        entries: vec![(owner, document, "about:blank".into())],
        ..Default::default()
    };
    let output = export(&host, &docs, None).unwrap();
    assert_eq!(output.matches("<p>ordinary</p>").count(), 1000);
    assert!(output.contains("srcdoc=\"&lt;p&gt;live &amp;amp; child&lt;/p&gt;\""));
    assert_eq!(docs.lookups.get(), 1);
}

#[test]
fn embedded_documents_escape_every_token_and_resume_attributes_and_siblings() {
    let mut host = host();
    let root = host.document_handle();
    let outer = child(&mut host, root, "iframe");
    for (name, value) in [
        ("data-before", "&\""),
        ("srcdoc", "STALE"),
        ("data-after", "tail"),
        ("data-moli-frame-url", "OLD"),
    ] {
        assert!(host.set_attribute(outer, name, value));
    }
    let middle_doc = host.create_detached_html_document();
    let inner = child(&mut host, middle_doc, "iframe");
    // Exercise both absent srcdoc and an existing URL before generated srcdoc.
    assert!(host.set_attribute(inner, "data-moli-frame-url", "OLD"));
    let leaf = host.create_detached_html_document();
    let script = child(&mut host, leaf, "script");
    text(&mut host, script, "const v = '<&>\"雪\u{a0}';");
    let comment = host.create_comment("<&>\"雪\u{a0}");
    assert!(host.append_child(leaf, comment));
    let node = child(&mut host, leaf, "x-card");
    assert!(host.set_attribute(node, "title", "<&>\"雪\u{a0}"));
    text(&mut host, node, "<&>\"雪\u{a0}");
    let shadow = host
        .attach_shadow_root_with_init(node, ShadowRootInit::new("closed"))
        .unwrap();
    text(&mut host, shadow, "SHADOW <&>\"");
    let after_inner = child(&mut host, middle_doc, "b");
    text(&mut host, after_inner, "after inner <&>\"");
    let after_outer = child(&mut host, root, "p");
    text(&mut host, after_outer, "after outer <&>\"");
    let docs = Documents {
        entries: vec![
            (outer, middle_doc, "https://frame.test/?a=1&b=2".into()),
            (inner, leaf, "about:srcdoc".into()),
        ],
        ..Default::default()
    };
    // An independent, bottom-up oracle: normal child markup, then normal HTML
    // attribute escaping once for each srcdoc boundary. No shared writer state.
    let leaf_html = host
        .get_html_for_export(leaf, &|_| true, None, None, None)
        .unwrap();
    let middle_html = format!(
        "<iframe data-moli-frame-url=\"about:srcdoc\" srcdoc=\"{}\"></iframe><b>after inner &lt;&amp;&gt;\"</b>",
        escape(&leaf_html)
    );
    let expected = format!(
        "<iframe data-before=\"&amp;&quot;\" srcdoc=\"{}\" data-after=\"tail\" data-moli-frame-url=\"https://frame.test/?a=1&amp;b=2\"></iframe><p>after outer &lt;&amp;&gt;\"</p>",
        escape(&middle_html)
    );
    let before = host.dom().serialize_document();
    assert_eq!(export(&host, &docs, None).unwrap(), expected);
    assert_eq!(host.dom().serialize_document(), before);
    assert!(before.contains("STALE") && before.contains("OLD"));
    // Limits apply to the final, twice-escaped bytes, not the raw child size.
    for limit in [0, leaf_html.len(), expected.len() - 1] {
        assert_eq!(
            export(&host, &docs, Some(limit)),
            Err(HtmlSerializationError::OutputLimitExceeded { max_bytes: limit })
        );
    }
    assert_eq!(
        export(&host, &docs, Some(expected.len())).unwrap(),
        expected
    );
    assert_eq!(export(&host, &docs, None).unwrap(), expected);
}

#[test]
fn filtered_ancestors_do_not_query_frames_or_build_shadow_output() {
    let mut host = host();
    let root = host.document_handle();
    let nav = child(&mut host, root, "nav");
    let owner = child(&mut host, nav, "iframe");
    let docs = Documents {
        entries: vec![(owner, root, "cycle must never be visited".into())],
        ..Default::default()
    };
    struct Skip;
    impl HtmlSerializationPolicy for Skip {
        fn include_node(&self, node: &Node) -> bool {
            node.as_element().is_none_or(|e| e.local_name() != "nav")
        }
    }
    assert_eq!(
        host.get_html_for_export(root, &|_| true, Some(&Skip), Some(&docs), None)
            .unwrap(),
        ""
    );
    assert_eq!(docs.lookups.get(), 0);
    // An unavailable Document keeps the author's original src/srcdoc verbatim.
    assert!(host.set_attribute(owner, "srcdoc", "<b>original & text</b>"));
    let empty = Documents::default();
    assert_eq!(
        export(&host, &empty, None).unwrap(),
        host.dom().serialize_document()
    );
}

#[test]
fn embedded_document_cycles_and_invalid_documents_are_errors_without_partial_output() {
    let mut host = host();
    let root = host.document_handle();
    let owner = child(&mut host, root, "iframe");
    let document = host.create_detached_html_document();
    let nested = child(&mut host, document, "iframe");
    for (target, expected) in [
        (
            root,
            HtmlSerializationError::CyclicEmbeddedDocument { document: root },
        ),
        (
            document,
            HtmlSerializationError::CyclicEmbeddedDocument { document },
        ),
        (owner, HtmlSerializationError::InvalidNode { node: owner }),
    ] {
        let docs = Documents {
            entries: vec![
                (owner, document, "outer".into()),
                (nested, target, "inner".into()),
            ],
            ..Default::default()
        };
        assert_eq!(export(&host, &docs, None), Err(expected));
        assert_eq!(
            export(&host, &Documents::default(), None).unwrap(),
            "<iframe></iframe>"
        );
    }
}

#[test]
fn deep_embedded_documents_use_an_explicit_stack_and_restore_each_sibling() {
    let mut host = host();
    let mut docs = Documents::default();
    let mut current = host.document_handle();
    let mut documents = Vec::new();
    for index in 0..128 {
        let owner = child(&mut host, current, "iframe");
        let document = host.create_detached_html_document();
        docs.entries
            .push((owner, document, format!("https://frame.test/{index}")));
        let sibling = child(&mut host, current, "b");
        text(&mut host, sibling, "&\"");
        documents.push(current);
        current = document;
    }
    let leaf = child(&mut host, current, "p");
    text(&mut host, leaf, "leaf <&>\"");
    let mut expected = "<p>leaf &lt;&amp;&gt;\"</p>".to_owned();
    for index in (0..documents.len()).rev() {
        expected = format!(
            "<iframe srcdoc=\"{}\" data-moli-frame-url=\"https://frame.test/{index}\"></iframe><b>&amp;\"</b>",
            escape(&expected)
        );
    }
    assert_eq!(export(&host, &docs, None).unwrap(), expected);
}

#[test]
fn embedded_attribute_resumption_applies_the_same_policy_as_ordinary_elements() {
    let mut host = host();
    let root = host.document_handle();
    let owner = child(&mut host, root, "iframe");
    for (name, value) in [
        ("title", "old"),
        ("srcdoc", "stale"),
        ("onload", "remove"),
        ("data-after", "kept"),
    ] {
        assert!(host.set_attribute(owner, name, value));
    }
    let document = host.create_detached_html_document();
    let p = child(&mut host, document, "p");
    text(&mut host, p, "live");
    let docs = Documents {
        entries: vec![(owner, document, "about:blank".into())],
        ..Default::default()
    };
    struct Policy(NativeNodeId, [Attribute; 2], bool);
    impl HtmlSerializationPolicy for Policy {
        fn has_filters(&self) -> bool {
            self.2
        }
        fn include_attribute(&self, _: NativeNodeId, a: &Attribute) -> bool {
            !a.name_matches("onload")
        }
        fn attribute_overrides(&self, node: NativeNodeId) -> &[Attribute] {
            if node == self.0 { &self.1 } else { &[] }
        }
    }
    let mut policy = Policy(
        owner,
        [
            Attribute::new("title".into(), String::new(), None, "new & value".into()),
            Attribute::new("data-extra".into(), String::new(), None, "last".into()),
        ],
        true,
    );
    let output = host
        .get_html_for_export(root, &|_| true, Some(&policy), Some(&docs), None)
        .unwrap();
    assert!(
        output.starts_with("<iframe title=\"new &amp; value\" srcdoc=\""),
        "{output}"
    );
    assert!(
        output.ends_with(
            " data-after=\"kept\" data-extra=\"last\" data-moli-frame-url=\"about:blank\"></iframe>"
        ),
        "{output}"
    );
    assert!(!output.contains("remove") && !output.contains("stale"));
    policy.2 = false;
    let output = host
        .get_html_for_export(root, &|_| true, Some(&policy), Some(&docs), None)
        .unwrap();
    assert!(output.contains("onload=\"remove\""));
    assert!(output.contains("title=\"new &amp; value\""));
    assert!(output.contains("data-extra=\"last\""));
    assert!(!output.contains("stale"));
}

#[test]
fn namespaced_frame_attributes_do_not_replace_the_null_namespace_export_values() {
    for author_attributes in [false, true] {
        let mut host = host();
        let root = host.document_handle();
        let owner = child(&mut host, root, "iframe");
        for (namespace, prefix, label) in [
            ("http://www.w3.org/XML/1998/namespace", None, "xml"),
            ("urn:frame-metadata", Some("p"), "custom"),
        ] {
            for name in ["srcdoc", "data-moli-frame-url"] {
                assert!(host.set_attribute_ns(
                    owner,
                    Some(namespace),
                    prefix,
                    name,
                    &format!("{label} & value"),
                ));
            }
        }
        if author_attributes {
            assert!(host.set_attribute_ns(owner, None, None, "srcdoc", "stale"));
            assert!(host.set_attribute_ns(owner, None, None, "data-moli-frame-url", "stale"));
        }
        assert!(host.set_attribute(owner, "data-after", "tail"));
        let document = host.create_detached_html_document();
        let b = child(&mut host, document, "b");
        text(&mut host, b, "live & child");
        let docs = Documents {
            entries: vec![(owner, document, "https://frame.test/?a=1&b=2".into())],
            ..Default::default()
        };
        let metadata = concat!(
            " xml:srcdoc=\"xml &amp; value\"",
            " xml:data-moli-frame-url=\"xml &amp; value\"",
            " p:srcdoc=\"custom &amp; value\"",
            " p:data-moli-frame-url=\"custom &amp; value\"",
        );
        let embedded = format!(
            " srcdoc=\"{}\" data-moli-frame-url=\"https://frame.test/?a=1&amp;b=2\"",
            escape("<b>live &amp; child</b>"),
        );
        let expected = if author_attributes {
            format!("<iframe{metadata}{embedded} data-after=\"tail\"></iframe>")
        } else {
            format!("<iframe{metadata} data-after=\"tail\"{embedded}></iframe>")
        };
        let before = host.dom().serialize_document();
        assert_eq!(export(&host, &docs, None).unwrap(), expected);
        assert_eq!(host.dom().serialize_document(), before);
        assert_eq!(
            export(&host, &docs, Some(expected.len() - 1)),
            Err(HtmlSerializationError::OutputLimitExceeded {
                max_bytes: expected.len() - 1,
            }),
        );
        assert_eq!(
            export(&host, &docs, Some(expected.len())).unwrap(),
            expected
        );
    }
}
