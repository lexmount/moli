use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{NodeData, RcDom};
use moli_html2md::{Dom, NodeKind};

/// An independent test adapter. Parsing and copying happen only in tests;
/// the production converter sees the same four borrowed queries as NativeDom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree {
    pub nodes: Vec<Node>,
    pub root: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub tag: Option<String>,
    pub text: Option<String>,
    pub attrs: Vec<(String, String)>,
    pub first: Option<usize>,
    pub next: Option<usize>,
}

impl Tree {
    pub fn parse(html: &str) -> Self {
        let parsed = html5ever::parse_document(RcDom::default(), Default::default()).one(format!(
            "<x-turndown id='turndown-root'>{html}</x-turndown>"
        ));
        let mut nodes: Vec<Node> = Vec::new();
        let mut root = 0;
        // RcDom's destructor clears descendants even when handles survive.
        // Keep the parsed document alive until the complete arena is copied.
        let mut stack = vec![(parsed.document.clone(), None::<usize>)];
        let mut last_children: Vec<Option<usize>> = Vec::new();
        while let Some((node, parent)) = stack.pop() {
            let id = nodes.len();
            let (tag, text, attrs) = match &node.data {
                NodeData::Element { name, attrs, .. } => (
                    Some(name.local.to_string()),
                    None,
                    attrs
                        .borrow()
                        .iter()
                        .map(|a| (a.name.local.to_string(), a.value.to_string()))
                        .collect(),
                ),
                NodeData::Text { contents } => {
                    (None, Some(contents.borrow().to_string()), Vec::new())
                }
                _ => (None, None, Vec::new()),
            };
            if attrs
                .iter()
                .any(|(key, value)| key == "id" && value == "turndown-root")
            {
                root = id;
            }
            nodes.push(Node {
                tag,
                text,
                attrs,
                first: None,
                next: None,
            });
            last_children.push(None);
            if let Some(parent) = parent {
                if let Some(previous) = last_children[parent] {
                    nodes[previous].next = Some(id);
                } else {
                    nodes[parent].first = Some(id);
                }
                last_children[parent] = Some(id);
            }
            stack.extend(
                node.children
                    .borrow()
                    .iter()
                    .rev()
                    .map(|child| (child.clone(), Some(id))),
            );
        }
        assert_ne!(root, 0, "fixture wrapper must be present");
        Self { nodes, root }
    }
}

impl Dom for Tree {
    type NodeId = usize;

    fn node_kind(&self, node: usize) -> NodeKind<'_> {
        let node = &self.nodes[node];
        if let Some(tag) = &node.tag {
            NodeKind::Element(tag)
        } else if let Some(text) = &node.text {
            NodeKind::Text(text)
        } else {
            NodeKind::Other
        }
    }

    fn first_child(&self, node: usize) -> Option<usize> {
        self.nodes[node].first
    }
    fn next_sibling(&self, node: usize) -> Option<usize> {
        self.nodes[node].next
    }
    fn attribute(&self, node: usize, name: &str) -> Option<&str> {
        self.nodes[node]
            .attrs
            .iter()
            .find_map(|(key, value)| (key == name).then_some(value.as_str()))
    }
}

pub fn rendered_html(markdown: &str) -> String {
    let parser = pulldown_cmark::Parser::new_ext(
        markdown,
        pulldown_cmark::Options::ENABLE_TABLES | pulldown_cmark::Options::ENABLE_STRIKETHROUGH,
    );
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, parser);
    html
}
