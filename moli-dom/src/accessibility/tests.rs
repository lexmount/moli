use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

use super::*;
use crate::{
    NodeId,
    native::{DomHost, NativeDom},
};

#[derive(Default)]
pub(super) struct FixtureStyles(std::collections::HashMap<NodeId, AccessibilityStyle>);

impl FixtureStyles {
    pub(super) fn visible(document: &crate::native::DomHost) -> Self {
        Self(
            document
                .nodes()
                .iter()
                .filter(|node| node.is_element())
                .map(|node| {
                    (
                        node.id(),
                        AccessibilityStyle {
                            display_none: false,
                            visibility_visible: true,
                            hides_contents: false,
                            block_level: false,
                        },
                    )
                })
                .collect(),
        )
    }

    pub(super) fn insert(&mut self, node: NodeId, style: AccessibilityStyle) {
        self.0.insert(node, style);
    }
}

impl AccessibilityStyleSource for FixtureStyles {
    fn element_style(&mut self, node: NodeId) -> Option<AccessibilityStyle> {
        self.0.get(&node).copied()
    }
}

fn backend_node_id(node: NodeId) -> u32 {
    u32::try_from(node.index() + 1).expect("fixture backend id")
}

fn tree(document: &DomHost) -> Vec<Value> {
    request(
        document,
        &mut FixtureStyles::visible(document),
        document.document_node_id(),
        AccessibilityRequest::Tree { max_depth: None },
    )
    .expect("fixture AX tree")
}

fn request(
    document: &DomHost,
    styles: &mut dyn AccessibilityStyleSource,
    node: NodeId,
    request: AccessibilityRequest,
) -> Option<Vec<Value>> {
    accessibility_payloads_for_document(
        document,
        &mut AccessibilityInput::new(styles, AccessibilityFrameState::Active),
        node,
        request,
        &mut |node| Some(backend_node_id(node)),
    )
}

fn document_with_host() -> (DomHost, NodeId) {
    let mut document = DomHost::from_dom(NativeDom::new_html(
        url::Url::parse("https://example.test/").expect("valid document URL"),
    ));
    let host = append_element(&mut document, NodeId::new(0), "div");
    (document, host)
}

fn append_element(document: &mut DomHost, parent: NodeId, tag: &str) -> NodeId {
    let element = document.create_element(tag);
    assert!(document.append_child(parent, element));
    element
}

fn append_text(document: &mut DomHost, parent: NodeId, text: &str) {
    let text = document.create_text_node(text);
    assert!(document.append_child(parent, text));
}

fn node_by_id(nodes: &[Value], node_id: NodeId) -> &Value {
    let backend_id = backend_node_id(node_id);
    nodes
        .iter()
        .find(|node| node["backendDOMNodeId"] == backend_id)
        .unwrap_or_else(|| panic!("missing AX node for {node_id:?}: {nodes:?}"))
}

fn button_names(nodes: &[Value]) -> Vec<&str> {
    nodes
        .iter()
        .filter(|node| node["role"]["value"] == "button")
        .map(|node| node["name"]["value"].as_str().expect("button name"))
        .collect()
}

#[test]
fn ax_requires_styles_for_connected_elements() {
    let (document, _) = document_with_host();
    assert!(
        request(
            &document,
            &mut FixtureStyles::default(),
            document.document_node_id(),
            AccessibilityRequest::Tree { max_depth: None },
        )
        .is_none(),
        "missing computed style must not become a visible default"
    );
}

#[test]
fn ax_hidden_reference_names_cross_transparent_shadow_roots() {
    for mode in ["open", "closed"] {
        let (mut document, host) = document_with_host();
        assert!(document.set_attribute(host, "id", "label"));
        append_text(&mut document, host, "assigned");
        let root = document
            .attach_shadow_root(host, mode)
            .expect("shadow root");
        append_text(&mut document, root, "Shadow ");
        append_element(&mut document, root, "slot");
        let document_id = document.document_node_id();
        let button = append_element(&mut document, document_id, "button");
        assert!(document.set_attribute(button, "aria-labelledby", "label"));
        let mut styles = FixtureStyles::visible(&document);
        styles.insert(
            host,
            AccessibilityStyle {
                display_none: true,
                visibility_visible: true,
                hides_contents: false,
                block_level: true,
            },
        );
        let nodes = request(
            &document,
            &mut styles,
            document_id,
            AccessibilityRequest::Tree { max_depth: None },
        )
        .expect("AX tree");
        assert_eq!(
            node_by_id(&nodes, button)["name"]["value"],
            "Shadow assigned"
        );
        assert!(
            !nodes
                .iter()
                .any(|node| node["backendDOMNodeId"] == backend_node_id(host))
        );
    }
}

#[test]
fn ax_tree_queries_shadow_roots_from_hosts() {
    for mode in ["open", "closed"] {
        let (mut document, host) = document_with_host();
        let root = document
            .attach_shadow_root(host, mode)
            .expect("shadow root");
        let button = append_element(&mut document, root, "button");
        append_text(&mut document, button, "Shadow action");
        let document_id = document.document_node_id();
        let outside = append_element(&mut document, document_id, "button");
        append_text(&mut document, outside, "Outside action");

        let nodes = request(
            &document,
            &mut FixtureStyles::visible(&document),
            root,
            AccessibilityRequest::Tree { max_depth: None },
        )
        .expect("fixture AX tree");
        assert_eq!(nodes[0]["backendDOMNodeId"], backend_node_id(host));
        assert_eq!(button_names(&nodes), ["Shadow action"]);
        assert_eq!(
            node_by_id(&nodes, button)["nodeId"],
            node_by_id(&tree(&document), button)["nodeId"]
        );
        assert!(!nodes.iter().any(|node| {
            node["backendDOMNodeId"] == backend_node_id(root)
                || node["backendDOMNodeId"] == backend_node_id(outside)
        }));
    }
}

#[test]
fn ax_tree_names_shadow_hosts_from_composed_contents() {
    for mode in ["open", "closed"] {
        let (mut document, host) = document_with_host();
        assert!(document.set_attribute(host, "role", "button"));
        append_text(&mut document, host, "host name");
        let root = document
            .attach_shadow_root(host, mode)
            .expect("shadow root");
        append_text(&mut document, root, "Shadow ");
        append_element(&mut document, root, "slot");

        assert_eq!(
            node_by_id(&tree(&document), host)["name"]["value"],
            "Shadow host name"
        );
    }
}

#[test]
fn ax_tree_includes_nested_shadow_controls_and_host_ancestors() {
    for mode in ["open", "closed"] {
        let (mut document, host) = document_with_host();
        let root = document
            .attach_shadow_root(host, mode)
            .expect("shadow root");
        let nested_host = append_element(&mut document, root, "div");
        let nested_root = document
            .attach_shadow_root(nested_host, mode)
            .expect("nested shadow root");
        let button = append_element(&mut document, nested_root, "button");
        append_text(&mut document, button, "Shadow action");

        let nodes = tree(&document);
        let button_node = node_by_id(&nodes, button);
        assert_eq!(button_node["role"]["value"], "button");
        assert_eq!(button_node["name"]["value"], "Shadow action");
        assert_eq!(button_node["ignored"], false);
        assert_eq!(
            button_node["parentId"],
            node_by_id(&nodes, nested_host)["nodeId"]
        );
        assert_eq!(
            node_by_id(&nodes, nested_host)["parentId"],
            node_by_id(&nodes, host)["nodeId"]
        );

        let ancestors = request(
            &document,
            &mut FixtureStyles::visible(&document),
            button,
            AccessibilityRequest::Ancestors,
        )
        .expect("fixture AX ancestors");
        let backend_ids = ancestors
            .iter()
            .map(|node| node["backendDOMNodeId"].clone())
            .collect::<Vec<_>>();
        assert_eq!(
            backend_ids,
            [button, nested_host, host, document.document_node_id()]
                .map(|id| json!(backend_node_id(id)))
        );
    }
}

#[test]
fn ax_tree_uses_slot_order_and_replaces_fallback_after_assignment_changes() {
    let (mut document, host) = document_with_host();
    let first = append_element(&mut document, host, "button");
    assert!(document.set_attribute(first, "slot", "second"));
    append_text(&mut document, first, "First");
    let second = append_element(&mut document, host, "button");
    assert!(document.set_attribute(second, "slot", "first"));
    append_text(&mut document, second, "Second");
    let unassigned = append_element(&mut document, host, "button");
    append_text(&mut document, unassigned, "Unassigned");

    let root = document
        .attach_shadow_root(host, "open")
        .expect("shadow root");
    let first_slot = append_element(&mut document, root, "slot");
    assert!(document.set_attribute(first_slot, "name", "first"));
    let fallback = append_element(&mut document, first_slot, "button");
    append_text(&mut document, fallback, "Fallback");
    let second_slot = append_element(&mut document, root, "slot");
    assert!(document.set_attribute(second_slot, "name", "second"));

    let nodes = tree(&document);
    assert_eq!(button_names(&nodes), ["Second", "First"]);
    assert_eq!(
        node_by_id(&nodes, second)["parentId"],
        node_by_id(&nodes, first_slot)["nodeId"]
    );

    assert!(document.set_attribute(second, "slot", "missing"));
    let nodes = tree(&document);
    assert_eq!(button_names(&nodes), ["Fallback", "First"]);
}

#[test]
fn ax_tree_names_shadow_controls_from_assigned_text_and_scoped_labels() {
    let (mut document, host) = document_with_host();
    let assigned = append_element(&mut document, host, "span");
    append_text(&mut document, assigned, "Assigned name");
    let document_id = document.document_node_id();
    let outside_label = append_element(&mut document, document_id, "span");
    assert!(document.set_attribute(outside_label, "id", "label"));
    append_text(&mut document, outside_label, "Outside label");

    let root = document
        .attach_shadow_root(host, "open")
        .expect("shadow root");
    let button = append_element(&mut document, root, "button");
    let slot = append_element(&mut document, button, "slot");
    append_text(&mut document, slot, "Fallback name");
    let label = append_element(&mut document, root, "span");
    assert!(document.set_attribute(label, "id", "label"));
    append_text(&mut document, label, "Shadow label");
    let labelled_button = append_element(&mut document, root, "button");
    assert!(document.set_attribute(labelled_button, "aria-labelledby", "label"));

    let nodes = tree(&document);
    assert_eq!(node_by_id(&nodes, button)["name"]["value"], "Assigned name");
    assert_eq!(
        node_by_id(&nodes, labelled_button)["name"]["value"],
        "Shadow label"
    );
}

#[test]
fn ax_tree_inherits_aria_hidden_from_shadow_host() {
    let (mut document, host) = document_with_host();
    assert!(document.set_attribute(host, "aria-hidden", "true"));
    let root = document
        .attach_shadow_root(host, "open")
        .expect("shadow root");
    let button = append_element(&mut document, root, "button");
    append_text(&mut document, button, "Hidden action");

    let nodes = tree(&document);
    let button_node = node_by_id(&nodes, button);
    assert_eq!(button_node["ignored"], true);
    assert_eq!(
        button_node["ignoredReasons"][0]["name"],
        "ariaHiddenSubtree"
    );
    assert_eq!(
        button_node["ignoredReasons"][0]["value"]["relatedNodes"][0]["backendDOMNodeId"],
        backend_node_id(host)
    );
}

struct RecordingStyles {
    facts: FixtureStyles,
    missing: HashSet<NodeId>,
    reads: Vec<NodeId>,
}

impl RecordingStyles {
    fn new(document: &DomHost) -> Self {
        Self {
            facts: FixtureStyles::visible(document),
            missing: HashSet::new(),
            reads: Vec::new(),
        }
    }

    fn assert_read_once(&self) {
        assert_eq!(
            self.reads.len(),
            self.reads.iter().collect::<HashSet<_>>().len(),
            "each required element style must be read once: {:?}",
            self.reads
        );
    }
}

impl AccessibilityStyleSource for RecordingStyles {
    fn element_style(&mut self, node: NodeId) -> Option<AccessibilityStyle> {
        self.reads.push(node);
        if self.missing.contains(&node) {
            None
        } else {
            self.facts.element_style(node)
        }
    }
}

#[test]
fn local_requests_only_read_their_dependencies_as_unrelated_subtrees_grow() {
    for size in [0, 2048] {
        let (mut document, host) = document_with_host();
        let target = append_element(&mut document, host, "button");
        document.set_attribute(target, "aria-label", "Target");
        let child = append_element(&mut document, target, "span");
        append_text(&mut document, child, "Contents");
        let unrelated = append_element(&mut document, host, "div");
        let untouched = (0..size)
            .map(|_| {
                let node = append_element(&mut document, unrelated, "span");
                append_text(&mut document, node, "Unrelated");
                node
            })
            .collect::<HashSet<_>>();

        for (query, reads_sibling) in [
            (AccessibilityRequest::Node, false),
            (
                AccessibilityRequest::Partial {
                    fetch_relatives: false,
                },
                false,
            ),
            (
                AccessibilityRequest::Partial {
                    fetch_relatives: true,
                },
                true,
            ),
            (AccessibilityRequest::Ancestors, true),
            (AccessibilityRequest::Tree { max_depth: None }, false),
        ] {
            let mut styles = RecordingStyles::new(&document);
            styles.missing = untouched.clone();
            if !reads_sibling {
                styles.missing.insert(unrelated);
            }
            let mut refs = HashMap::new();
            let nodes = accessibility_payloads_for_document(
                &document,
                &mut AccessibilityInput::new(&mut styles, AccessibilityFrameState::Active),
                target,
                query,
                &mut |node| Some(*refs.entry(node).or_insert(backend_node_id(node))),
            )
            .expect("unrelated missing styles must not affect a local AX request");
            assert_eq!(node_by_id(&nodes, target)["name"]["value"], "Target");
            let mut expected = HashSet::from([host, target, child]);
            if reads_sibling {
                expected.insert(unrelated);
            }
            assert_eq!(
                styles.reads.iter().copied().collect::<HashSet<_>>(),
                expected,
                "{query:?}, unrelated size {size}"
            );
            styles.assert_read_once();
            assert!(
                refs.keys().all(|id| !untouched.contains(id)),
                "unrelated descendants must not receive backend refs"
            );
        }
    }
}

#[test]
fn local_names_can_read_hidden_shadow_labels_outside_the_projected_subtree() {
    for mode in ["open", "closed"] {
        let (mut document, host) = document_with_host();
        let target = append_element(&mut document, host, "button");
        document.set_attribute(target, "aria-labelledby", "external-label");
        let root = document.document_node_id();
        let label = append_element(&mut document, root, "div");
        document.set_attribute(label, "id", "external-label");
        let assigned = append_element(&mut document, label, "span");
        append_text(&mut document, assigned, "assigned");
        let shadow = document
            .attach_shadow_root(label, mode)
            .expect("shadow root");
        append_text(&mut document, shadow, "Shadow");
        let slot = append_element(&mut document, shadow, "slot");
        let unrelated = append_element(&mut document, root, "div");
        for query in [
            AccessibilityRequest::Node,
            AccessibilityRequest::Partial {
                fetch_relatives: false,
            },
            AccessibilityRequest::Tree { max_depth: None },
        ] {
            let mut styles = RecordingStyles::new(&document);
            let hidden = AccessibilityStyle {
                display_none: true,
                visibility_visible: true,
                hides_contents: false,
                block_level: true,
            };
            styles.facts.insert(label, hidden);
            styles.missing.insert(unrelated);
            let nodes =
                request(&document, &mut styles, target, query).expect("local named AX node");
            assert_eq!(
                node_by_id(&nodes, target)["name"]["value"],
                "Shadow assigned"
            );
            assert!(
                nodes
                    .iter()
                    .all(|node| node["backendDOMNodeId"] != backend_node_id(label))
            );
            assert_eq!(
                styles.reads.iter().copied().collect::<HashSet<_>>(),
                HashSet::from([host, target, label, assigned, slot])
            );
            styles.assert_read_once();

            styles.missing.insert(label);
            assert!(
                request(&document, &mut styles, target, query).is_none(),
                "a required external label style must not become an empty name"
            );
        }
    }
}

#[test]
fn required_content_and_child_id_styles_fail_the_whole_request() {
    let (mut document, host) = document_with_host();
    let target = append_element(&mut document, host, "button");
    let child = append_element(&mut document, target, "span");
    append_text(&mut document, child, "Action");
    for explicit_name in [false, true] {
        if explicit_name {
            document.set_attribute(target, "aria-label", "Explicit");
        }
        let mut styles = RecordingStyles::new(&document);
        styles.missing.insert(child);
        assert!(
            request(&document, &mut styles, target, AccessibilityRequest::Node).is_none(),
            "required name contents and childIds must propagate missing styles"
        );
    }
}

#[test]
fn descendant_text_alternatives_read_only_required_external_styles() {
    let (mut document, host) = document_with_host();
    let target = append_element(&mut document, host, "button");
    let child = append_element(&mut document, target, "span");
    document.set_attribute(child, "aria-labelledby", "external");
    let root = document.document_node_id();
    let label = append_element(&mut document, root, "span");
    document.set_attribute(label, "id", "external");
    document.set_attribute(label, "aria-label", "Copy");
    let unrelated = append_element(&mut document, root, "div");
    for query in [
        AccessibilityRequest::Node,
        AccessibilityRequest::Partial {
            fetch_relatives: false,
        },
    ] {
        let mut styles = RecordingStyles::new(&document);
        styles.facts.insert(
            label,
            AccessibilityStyle {
                display_none: true,
                visibility_visible: true,
                hides_contents: false,
                block_level: false,
            },
        );
        styles.missing.insert(unrelated);
        let nodes = request(&document, &mut styles, target, query).expect("local named AX node");
        assert_eq!(node_by_id(&nodes, target)["name"]["value"], "Copy");
        assert_eq!(
            styles.reads.iter().copied().collect::<HashSet<_>>(),
            HashSet::from([host, target, child, label])
        );
        styles.assert_read_once();

        styles.missing.insert(label);
        assert!(
            request(&document, &mut styles, target, query).is_none(),
            "required descendant reference styles must not become an empty name"
        );
    }
}

#[test]
fn full_tree_does_not_read_styles_below_a_not_rendered_branch() {
    let (mut document, host) = document_with_host();
    let hidden = append_element(&mut document, host, "div");
    let child = append_element(&mut document, hidden, "button");
    append_text(&mut document, child, "Hidden action");
    let mut styles = RecordingStyles::new(&document);
    styles.facts.insert(
        hidden,
        AccessibilityStyle {
            display_none: true,
            visibility_visible: true,
            hides_contents: false,
            block_level: true,
        },
    );
    styles.missing.insert(child);
    let nodes = request(
        &document,
        &mut styles,
        document.document_node_id(),
        AccessibilityRequest::Tree { max_depth: None },
    )
    .expect("hidden descendants need no AX styles");
    assert!(button_names(&nodes).is_empty());
    assert_eq!(
        styles.reads.iter().copied().collect::<HashSet<_>>(),
        HashSet::from([host, hidden])
    );
    styles.assert_read_once();
}
