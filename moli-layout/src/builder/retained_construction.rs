//! Validation of source inputs before reusing an existing construction graph.
//!
//! These records describe the source frontier, including nodes that generated
//! no box. The normalized graph remains solely in `RetainedWork::boxes`.
//! Unsupported shapes use the ordinary builder; there is no second anonymous
//! box, table, pseudo-element or form-control construction algorithm here.

use std::{collections::HashMap, fmt::Debug, hash::Hash, sync::Arc, time::Instant};

use super::{BoxBuilder, is_leaf_element, principal_kind, suppresses_element_box};
use crate::{
    LayoutBoxId, LayoutBoxKind, LayoutDisplay, LayoutElementCategory, LayoutElementSemantics,
    LayoutError, LayoutNamespace, LayoutPosition, LayoutSource, LayoutSourceKind,
    LayoutStyleResolver, LayoutWorld, ResolvedLayoutStyle, style::InlineWhiteSpaceCollapse,
    working_set::RetainedWork,
};

#[derive(Clone, Debug, PartialEq, Eq)]
enum ElementShape {
    MissingStyle,
    Suppressed,
    Principal {
        kind: LayoutBoxKind,
        display: LayoutDisplay,
        position: LayoutPosition,
        out_of_flow: bool,
        white_space: InlineWhiteSpaceCollapse,
    },
}

impl ElementShape {
    fn from_inputs(
        semantics: &LayoutElementSemantics,
        style: Option<&ResolvedLayoutStyle>,
        before: bool,
        after: bool,
    ) -> Option<Self> {
        let Some(style) = style else {
            return Some(Self::MissingStyle);
        };
        if suppresses_element_box(semantics, style) {
            // Descendants and pseudos are not consumed while this frontier is
            // suppressed. Its semantics and current suppression are checked
            // again before every reuse, including head and body script nodes.
            return Some(Self::Suppressed);
        }
        let kind = principal_kind(semantics, style);
        if semantics.category != LayoutElementCategory::Generic
            || !matches!(
                style.display(),
                LayoutDisplay::Block | LayoutDisplay::FlowRoot
            )
            || is_leaf_element(semantics, kind, style)
            || style.is_floated()
            || before
            || after
        {
            return None;
        }
        // In this domain attach_children only uses normal flow classification
        // and whitespace suppression. The complete cold graph must additionally
        // contain no anonymous mixed-flow wrappers. Stable absolute/fixed
        // blocks remain children without voting as an in-flow block/inline;
        // their numeric parent and static position are rebuilt each pass.
        Some(Self::Principal {
            kind,
            display: style.display(),
            position: style.position(),
            out_of_flow: style.is_out_of_flow(),
            white_space: style.white_space_collapse(),
        })
    }
}

#[derive(Clone, Debug)]
enum SourceInputKind {
    Element {
        semantics: LayoutElementSemantics,
        shape: ElementShape,
    },
    Text(Option<Arc<str>>),
    Ignored(LayoutSourceKind),
}

impl SourceInputKind {
    fn node_kind(&self) -> LayoutSourceKind {
        match self {
            Self::Element { .. } => LayoutSourceKind::Element,
            Self::Text(_) => LayoutSourceKind::Text,
            Self::Ignored(kind) => *kind,
        }
    }
}

#[derive(Clone, Debug)]
struct SourceInput<N> {
    node: N,
    parent: Option<N>,
    kind: SourceInputKind,
    // None marks a frontier the shared builder did not descend into. Some
    // records the exact canonical flat-child sequence, including ignored and
    // subsequently compacted-away nodes, rather than the produced box list.
    children: Option<Vec<N>>,
}

#[derive(Clone, Debug)]
pub(crate) struct ConstructionInputs<N> {
    root: N,
    root_is_document_element: bool,
    quirks_mode: style::context::QuirksMode,
    sources: Vec<SourceInput<N>>,
    reused_boxes: usize,
}

impl<N> ConstructionInputs<N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub(super) fn new<S: LayoutSource<NodeId = N>>(source: &S) -> Self {
        Self {
            root: source.root(),
            root_is_document_element: source.root_is_document_element(),
            quirks_mode: source.quirks_mode(),
            sources: Vec::new(),
            reused_boxes: 0,
        }
    }

    pub(crate) fn reused_boxes(&self) -> usize {
        self.reused_boxes
    }

    pub(super) fn record_element<S: LayoutSource<NodeId = N>>(
        &mut self,
        source: &S,
        node: N,
        semantics: &LayoutElementSemantics,
        style: Option<&ResolvedLayoutStyle>,
        before: bool,
        after: bool,
    ) -> bool {
        let Some(shape) = ElementShape::from_inputs(semantics, style, before, after) else {
            return false;
        };
        self.sources.push(SourceInput {
            node,
            parent: source.flat_parent(node),
            kind: SourceInputKind::Element {
                semantics: semantics.clone(),
                shape,
            },
            children: None,
        });
        true
    }

    pub(super) fn record_text<S: LayoutSource<NodeId = N>>(
        &mut self,
        source: &S,
        node: N,
        text: Option<&Arc<str>>,
    ) {
        self.sources.push(SourceInput {
            node,
            parent: source.flat_parent(node),
            kind: SourceInputKind::Text(text.cloned()),
            children: None,
        });
    }

    pub(super) fn record_ignored<S: LayoutSource<NodeId = N>>(
        &mut self,
        source: &S,
        node: N,
        kind: LayoutSourceKind,
    ) {
        self.sources.push(SourceInput {
            node,
            parent: source.flat_parent(node),
            kind: SourceInputKind::Ignored(kind),
            children: None,
        });
    }

    pub(super) fn record_children(&mut self, parent: N, children: &[N]) {
        // Eligible nodes have no pseudos, so the element whose child stream
        // is about to be traversed is still the last recorded source.
        let input = self.sources.last_mut().expect("parent source was recorded");
        assert_eq!(input.node, parent);
        input.children = Some(children.to_vec());
    }

    pub(super) fn supports_graph(&self, world: &LayoutWorld<N>) -> bool {
        // A mixture of blocks and meaningful text can require wrappers even
        // when every element has an eligible principal style. Only the actual
        // shared builder's final graph decides whether this pass is admitted.
        world.display_contents_mapping.is_empty()
            && world.boxes.iter().enumerate().all(|(index, layout_box)| {
                layout_box.source.is_some()
                    && layout_box.owner.is_none()
                    && layout_box.pseudo.is_none()
                    && layout_box.anonymous_reason.is_none()
                    && !layout_box.outside_list_marker
                    && matches!(
                        layout_box.kind,
                        LayoutBoxKind::PrincipalBlock
                            | LayoutBoxKind::PrincipalFlowRoot
                            | LayoutBoxKind::Text
                    )
                    && layout_box.structural_parent == layout_box.parent
                    && layout_box
                        .parent
                        .is_none_or(|parent| parent.index() < index)
            })
    }

    /// Charge visible allocations by capacity. Shared source text and
    /// semantics are conservatively charged here as well as at graph owners;
    /// no computed-style or font payload is retained by these records.
    pub(crate) fn estimated_retained_bytes(&self) -> usize {
        fn allocation<T>(values: &Vec<T>) -> usize {
            values.capacity().saturating_mul(std::mem::size_of::<T>())
        }
        let mut bytes = std::mem::size_of::<Self>().saturating_add(allocation(&self.sources));
        for input in &self.sources {
            if let Some(children) = &input.children {
                bytes = bytes.saturating_add(allocation(children));
            }
            match &input.kind {
                SourceInputKind::Element { semantics, .. } => {
                    bytes = bytes.saturating_add(semantics.local_name.len());
                    if let LayoutNamespace::Other(namespace) = &semantics.namespace {
                        bytes = bytes.saturating_add(namespace.len());
                    }
                    if let Some(control) = &semantics.metadata.form_control {
                        bytes = bytes
                            .saturating_add(control.value.len())
                            .saturating_add(control.placeholder.len())
                            .saturating_add(allocation(&control.option_labels));
                        bytes = control
                            .option_labels
                            .iter()
                            .fold(bytes, |bytes, label| bytes.saturating_add(label.len()));
                    }
                }
                SourceInputKind::Text(text) => {
                    bytes = bytes.saturating_add(text.as_deref().map_or(0, str::len));
                }
                SourceInputKind::Ignored(_) => {}
            }
        }
        bytes
    }

    #[cfg(test)]
    pub(crate) fn reserve_source_capacity_for_test(&mut self, additional: usize) {
        self.sources.reserve(additional);
    }
}

pub(super) fn try_restore<S, R>(
    source: &S,
    styles: &mut R,
    previous: &RetainedWork<S::NodeId>,
) -> Result<Option<LayoutWorld<S::NodeId>>, LayoutError>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
{
    let Some(inputs) = previous.construction_inputs() else {
        return Ok(None);
    };
    let started = moli_trace::cpu_profile_enabled().then(Instant::now);
    if source.root() != inputs.root
        || source.root_is_document_element() != inputs.root_is_document_element
        || source.quirks_mode() != inputs.quirks_mode
        || previous.root() != LayoutBoxId::from_index(0)
        || previous.root_is_document_element() != inputs.root_is_document_element
        || previous.quirks_mode() != inputs.quirks_mode
    {
        return Ok(None);
    }
    let mut builder = BoxBuilder::new(source, styles);
    builder.source_root = Some(inputs.root);
    builder.root_is_html = inputs.sources.first().is_some_and(|input| {
        matches!(&input.kind, SourceInputKind::Element { semantics, .. } if semantics.is_html_element("html"))
    });
    let mut current_styles = HashMap::new();
    for input in &inputs.sources {
        if source.node_kind(input.node) != input.kind.node_kind()
            || source.flat_parent(input.node) != input.parent
        {
            return Ok(None);
        }
        match &input.kind {
            SourceInputKind::Element { semantics, shape } => {
                if source.element_semantics(input.node).as_ref() != Some(semantics)
                    || (semantics.replaced.is_none()
                        && source.replaced_metrics(input.node).is_some())
                {
                    return Ok(None);
                }
                let (style, before, after) = match builder.styles.element_styles(input.node)? {
                    Some(styles) => {
                        let (style, before, after) = styles.into_parts();
                        (Some(style), before.is_some(), after.is_some())
                    }
                    None => (None, false, false),
                };
                if let Some(style) = style.as_ref() {
                    builder.note_viewport_body_candidate(input.node, semantics, style);
                }
                if ElementShape::from_inputs(semantics, style.as_ref(), before, after).as_ref()
                    != Some(shape)
                {
                    return Ok(None);
                }
                if matches!(shape, ElementShape::Principal { .. }) {
                    current_styles.insert(
                        input.node,
                        style.expect("a principal source has a current style"),
                    );
                }
            }
            SourceInputKind::Text(text) => {
                if source.element_semantics(input.node).is_some()
                    || source.replaced_metrics(input.node).is_some()
                    || source.text(input.node) != text.as_deref()
                {
                    return Ok(None);
                }
            }
            SourceInputKind::Ignored(_) => {
                if source.element_semantics(input.node).is_some()
                    || source.replaced_metrics(input.node).is_some()
                {
                    return Ok(None);
                }
            }
        }
        if let Some(children) = &input.children
            && !source
                .flat_children(input.node)
                .eq(children.iter().copied())
        {
            return Ok(None);
        }
    }
    let validation_finished = started.map(|_| Instant::now());

    // Every source dependency and every structural decision has now matched.
    // Restore the already normalized graph and freshly sample box inputs with
    // the same helpers used by BoxBuilder. Width/paint changes may replace a
    // ComputedValues owner without requiring a new construction graph.
    let mut world: Option<LayoutWorld<S::NodeId>> = None;
    for (index, old) in previous.construction_boxes().iter().enumerate() {
        let node = old.source.expect("retained construction requires a source");
        let mut layout_box = if old.kind == LayoutBoxKind::Text {
            let parent = old.parent.expect("a retained text source has a parent");
            let inherited_style = &world
                .as_ref()
                .expect("the root precedes its text descendants")
                .boxes[parent.index()]
            .style;
            builder.text_box(
                node,
                inherited_style,
                old.text.clone().expect("a retained text box owns text"),
            )
        } else {
            builder.principal_box(
                node,
                source.label(node),
                old.element_semantics
                    .clone()
                    .expect("a retained principal box has element semantics"),
                old.kind,
                current_styles
                    .remove(&node)
                    .expect("all principal source styles were validated"),
                source.replaced_metrics(node),
            )
        };
        layout_box.structural_parent = old.structural_parent;
        layout_box.parent = old.parent;
        layout_box.children = old.children.clone();
        layout_box.inline_formatting_context = old.inline_formatting_context;
        if let Some(world) = &mut world {
            let id = world.allocate(layout_box);
            assert_eq!(id.index(), index);
            world.map_source(node, id);
        } else {
            let mut root = LayoutWorld::new(layout_box, inputs.root_is_document_element);
            root.quirks_mode = inputs.quirks_mode;
            root.map_source(node, root.root());
            world = Some(root);
        }
    }
    let mut world = world.expect("a retained construction graph has a root");
    builder.finalize_viewport_scroll_policy(&mut world);
    world.css_image_references = builder.css_image_references;
    world.validate_invariants()?;
    let mut current_inputs = inputs.clone();
    current_inputs.reused_boxes = world.len();
    world.construction_inputs = Some(Box::new(current_inputs));
    if let (Some(started), Some(validated)) = (started, validation_finished) {
        tracing::info!(
            target: "moli_cpu_profile",
            stage = "layout_construction_reuse",
            source_validation_us = validated.duration_since(started).as_micros(),
            graph_restore_us = validated.elapsed().as_micros(),
            checked_source_count = inputs.sources.len(),
            reused_construction_boxes = world.len(),
        );
    }
    Ok(Some(world))
}
