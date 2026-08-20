use std::{collections::HashMap, fmt};

use crate::{LayoutElementSemantics, LayoutError, LayoutSource, LayoutSourceKind};

/// Versioned, allocation-ID-free dump of the exact flat source tree seen by layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedLayoutSourceTree {
    pub schema_version: u32,
    pub root: NormalizedLayoutSourceNode,
}

/// One source node in a deterministic flat-tree dump.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedLayoutSourceNode {
    pub path: String,
    pub source: String,
    pub kind: LayoutSourceKind,
    pub element: Option<LayoutElementSemantics>,
    pub text: Option<String>,
    pub children: Vec<NormalizedLayoutSourceNode>,
}

/// Validates and dumps the DOM-neutral source contract without running box construction.
pub fn normalize_layout_source<S>(source: &S) -> Result<NormalizedLayoutSourceTree, LayoutError>
where
    S: LayoutSource,
{
    let root = source.root();
    let root_label = source.label(root);
    if let Some(parent) = source.flat_parent(root) {
        return Err(LayoutError::source_contract(
            &root_label,
            format!(
                "view root must not have a flat parent, got {}",
                source.label(parent)
            ),
        ));
    }
    if source.node_kind(root) != LayoutSourceKind::Element {
        return Err(LayoutError::source_contract(
            &root_label,
            format!(
                "view root must be an element, got {:?}",
                source.node_kind(root)
            ),
        ));
    }

    let mut state = SourceDumpState {
        source,
        active: Vec::new(),
        seen: HashMap::new(),
    };
    let root = state.visit(root, "0".to_owned(), None)?;
    Ok(NormalizedLayoutSourceTree {
        schema_version: 1,
        root,
    })
}

struct SourceDumpState<'a, S>
where
    S: LayoutSource,
{
    source: &'a S,
    active: Vec<S::NodeId>,
    seen: HashMap<S::NodeId, String>,
}

impl<S> SourceDumpState<'_, S>
where
    S: LayoutSource,
{
    fn visit(
        &mut self,
        node: S::NodeId,
        path: String,
        expected_parent: Option<S::NodeId>,
    ) -> Result<NormalizedLayoutSourceNode, LayoutError> {
        let label = self.source.label(node);
        if self.active.contains(&node) {
            return Err(LayoutError::SourceCycle {
                source_label: label,
            });
        }
        if let Some(first_path) = self.seen.get(&node) {
            return Err(LayoutError::source_contract(
                label,
                format!(
                    "flat-tree node appears more than once; first path was {first_path}, second path is {path}"
                ),
            ));
        }
        if self.source.flat_parent(node) != expected_parent {
            let actual = self
                .source
                .flat_parent(node)
                .map(|parent| self.source.label(parent))
                .unwrap_or_else(|| "<none>".to_owned());
            let expected = expected_parent
                .map(|parent| self.source.label(parent))
                .unwrap_or_else(|| "<none>".to_owned());
            return Err(LayoutError::source_contract(
                label,
                format!("flat parent mismatch; expected {expected}, got {actual}"),
            ));
        }

        let kind = self.source.node_kind(node);
        let element = self.source.element_semantics(node);
        match (kind, element.as_ref()) {
            (LayoutSourceKind::Element, None) => {
                return Err(LayoutError::source_contract(
                    label,
                    "element source has no element semantics",
                ));
            }
            (LayoutSourceKind::Element, Some(element)) if element.local_name.is_empty() => {
                return Err(LayoutError::source_contract(
                    label,
                    "element local name must not be empty",
                ));
            }
            (LayoutSourceKind::Element, Some(element))
                if !element.is_replaced() && self.source.replaced_metrics(node).is_some() =>
            {
                return Err(LayoutError::source_contract(
                    label,
                    "non-replaced element exposed replaced metrics",
                ));
            }
            (
                LayoutSourceKind::Text | LayoutSourceKind::Comment | LayoutSourceKind::Other,
                Some(_),
            ) => {
                return Err(LayoutError::source_contract(
                    label,
                    format!("{kind:?} source exposed element semantics"),
                ));
            }
            (
                LayoutSourceKind::Text | LayoutSourceKind::Comment | LayoutSourceKind::Other,
                None,
            ) if self.source.replaced_metrics(node).is_some() => {
                return Err(LayoutError::source_contract(
                    label,
                    format!("{kind:?} source exposed replaced metrics"),
                ));
            }
            _ => {}
        }

        self.seen.insert(node, path.clone());
        self.active.push(node);
        let child_ids = self.source.flat_children(node).collect::<Vec<_>>();
        let mut children = Vec::with_capacity(child_ids.len());
        for (index, child) in child_ids.into_iter().enumerate() {
            children.push(self.visit(child, format!("{path}/{index}"), Some(node))?);
        }
        self.active.pop();

        Ok(NormalizedLayoutSourceNode {
            path,
            source: self.source.label(node),
            kind,
            element,
            text: (kind == LayoutSourceKind::Text)
                .then(|| self.source.text(node).map(str::to_owned))
                .flatten(),
            children,
        })
    }
}

impl fmt::Display for NormalizedLayoutSourceTree {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "source-tree-schema={}", self.schema_version)?;
        write_source_node(formatter, &self.root, 0)
    }
}

fn write_source_node(
    formatter: &mut fmt::Formatter<'_>,
    node: &NormalizedLayoutSourceNode,
    depth: usize,
) -> fmt::Result {
    for _ in 0..depth {
        formatter.write_str("  ")?;
    }
    write!(
        formatter,
        "{:?} path={} source={}",
        node.kind, node.path, node.source
    )?;
    if let Some(element) = &node.element {
        write!(
            formatter,
            " element={}:{} category={}",
            element.namespace.debug_name(),
            element.local_name,
            element.category.debug_name()
        )?;
        if let Some(replaced) = element.replaced_kind() {
            write!(formatter, " replaced={}", replaced.debug_name())?;
        } else if let Some(fallback) = element.image_fallback() {
            write!(
                formatter,
                " content=image-fallback alternative={:?}",
                fallback.alternative_text()
            )?;
        }
    }
    if let Some(text) = &node.text {
        write!(formatter, " text={text:?}")?;
    }
    formatter.write_str("\n")?;
    for child in &node.children {
        write_source_node(formatter, child, depth + 1)?;
    }
    Ok(())
}
