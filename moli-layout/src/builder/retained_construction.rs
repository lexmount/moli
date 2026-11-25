//! Validation of source inputs before reusing an existing construction graph.
//!
//! These records describe the source frontier, including nodes that generated
//! no box. A supported graph moves between the pass and the working set.
//! Unsupported shapes use the ordinary builder; there is no second anonymous
//! box, table, pseudo-element or form-control construction algorithm here.

use std::{fmt::Debug, hash::Hash, sync::Arc, time::Instant};

mod tail;

use super::{BoxBuilder, principal_kind, suppresses_element_box};
use crate::{
    LayoutBoxId, LayoutBoxKind, LayoutDisplay, LayoutElementCategory, LayoutElementSemantics,
    LayoutError, LayoutNamespace, LayoutPosition, LayoutSource, LayoutSourceKind,
    LayoutStyleResolver, LayoutWorld, ResolvedLayoutStyle, style::InlineWhiteSpaceCollapse,
    working_set::RetainedConstruction,
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
        if !matches!(
            semantics.category,
            LayoutElementCategory::Generic | LayoutElementCategory::Table(_)
        ) || !matches!(
            style.display(),
            LayoutDisplay::Block
                | LayoutDisplay::FlowRoot
                | LayoutDisplay::Grid
                | LayoutDisplay::Table
                | LayoutDisplay::InlineTable
                | LayoutDisplay::TableCaption
                | LayoutDisplay::TableRowGroup
                | LayoutDisplay::TableHeaderGroup
                | LayoutDisplay::TableFooterGroup
                | LayoutDisplay::TableRow
                | LayoutDisplay::TableCell
                | LayoutDisplay::TableColumnGroup
                | LayoutDisplay::TableColumn
        ) || (semantics
            .category
            .child_box_policy(semantics.is_replaced(), style.display())
            == crate::ChildBoxPolicy::None
            && kind != LayoutBoxKind::TableColumn)
            || style.is_floated()
            || before
            || after
        {
            return None;
        }
        // Admit the shared builder's actual source-backed graph, not a second
        // set of flow/Grid/table normalization rules. Anonymous repairs and
        // discarded principal styles are checked against the finished graph.
        // A table column is a source-backed leaf with no private replaced or
        // form-control contents; its unconsumed descendants stay outside the
        // recorded frontier. Numeric parents and table/grid data stay fresh.
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
                            | LayoutBoxKind::PrincipalGrid
                            | LayoutBoxKind::TableWrapper
                            | LayoutBoxKind::InlineTableWrapper
                            | LayoutBoxKind::TableCaption
                            | LayoutBoxKind::TableRowGroup
                            | LayoutBoxKind::TableHeaderGroup
                            | LayoutBoxKind::TableFooterGroup
                            | LayoutBoxKind::TableRow
                            | LayoutBoxKind::TableCell
                            | LayoutBoxKind::TableColumnGroup
                            | LayoutBoxKind::TableColumn
                            | LayoutBoxKind::Text
                    )
                    && layout_box.structural_parent == layout_box.parent
                    && layout_box
                        .parent
                        .is_none_or(|parent| parent.index() < index)
            })
            // Styles are restored in produced principal-box order. Table
            // column groups may discard otherwise eligible element subtrees;
            // those frontiers must not leave unmatched style records here.
            && self.sources.iter().filter_map(|input| match &input.kind {
                SourceInputKind::Element { shape: ElementShape::Principal { .. }, .. } => Some(input.node),
                _ => None,
            }).eq(world.boxes.iter().filter(|layout_box| layout_box.kind != LayoutBoxKind::Text).map(|layout_box| layout_box.source.expect("supported graph has sources")))
    }
}

pub(super) fn try_restore<S, R>(
    source: &S,
    styles: &mut R,
    previous: RetainedConstruction<S::NodeId>,
) -> Result<Option<LayoutWorld<S::NodeId>>, LayoutError>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
{
    let inputs = &previous.inputs;
    let started = moli_trace::cpu_profile_enabled().then(Instant::now);
    if source.root() != inputs.root
        || source.root_is_document_element() != inputs.root_is_document_element
        || source.quirks_mode() != inputs.quirks_mode
    {
        return Ok(None);
    }
    let mut builder = BoxBuilder::new(source, styles);
    builder.source_root = Some(inputs.root);
    builder.root_is_html = inputs.sources.first().is_some_and(|input| {
        matches!(&input.kind, SourceInputKind::Element { semantics, .. } if semantics.is_html_element("html"))
    });
    let principal_count = inputs
        .sources
        .iter()
        .filter(|input| {
            matches!(
                input.kind,
                SourceInputKind::Element {
                    shape: ElementShape::Principal { .. },
                    ..
                }
            )
        })
        .count();
    // The eligible graph is allocated in source preorder and compaction
    // preserves that order. Keep the current styles in the same sequence,
    // avoiding a hash table that repeatedly moves these large owned values.
    let mut current_styles = Vec::with_capacity(principal_count);
    let mut tail_edit = None;
    for (index, input) in inputs.sources.iter().enumerate() {
        if tail_edit
            .as_ref()
            .is_some_and(|edit: &tail::TailEdit<S::NodeId>| edit.removes(input.node))
        {
            continue;
        }
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
                    current_styles.push((
                        input.node,
                        style.expect("a principal source has a current style"),
                    ));
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
        if let Some(children) = &input.children {
            let current = source.flat_children(input.node).collect::<Vec<_>>();
            if current != *children {
                if tail_edit.is_some() {
                    return Ok(None);
                }
                tail_edit = tail::TailEdit::plan(&previous, index, current);
                if tail_edit.is_none() {
                    return Ok(None);
                }
            }
        }
    }
    let validation_finished = started.map(|_| Instant::now());
    let checked_source_count = inputs.sources.len();
    let mut current_styles = current_styles.into_iter();

    let RetainedConstruction {
        boxes,
        source_mapping,
        mut inputs,
    } = previous;
    let reused_boxes = tail_edit
        .as_ref()
        .map_or(boxes.len(), |edit| edit.keep_boxes);
    let mut world =
        LayoutWorld::from_construction(boxes, source_mapping, inputs.root_is_document_element);
    world.quirks_mode = inputs.quirks_mode;

    // Parents precede their text leaves in the admitted construction graph.
    // Refresh the same style and resource inputs as the cold builder.
    for index in 0..reused_boxes {
        let old = &world.boxes[index];
        let node = old
            .source
            .expect("retained construction requires source boxes");
        let style = if old.kind == LayoutBoxKind::Text {
            let parent = old.parent.expect("a retained text source has a parent");
            ResolvedLayoutStyle::text_leaf_from(&world.boxes[parent.index()].style)
        } else {
            let (styled_node, style) = current_styles.next().expect("validated principal style");
            assert_eq!(styled_node, node, "retained principal order changed");
            style
        };
        let layout_box = &mut world.boxes[index];
        layout_box.source_label.clone_from(&source.label(node));
        LayoutWorld::refresh_box_style(layout_box, style);
        if layout_box.kind == LayoutBoxKind::Text {
            layout_box.text_selection = source.text_selection(node);
        } else {
            builder.refresh_principal_inputs(layout_box, None);
        }
    }
    inputs.reused_boxes = reused_boxes;
    if let Some(edit) = tail_edit {
        let Some(updated) = edit.apply(&mut builder, &mut world, inputs)? else {
            return Ok(None);
        };
        inputs = updated;
    }
    builder.finalize_viewport_scroll_policy(&mut world);
    world.css_image_references = builder.css_image_references;
    world.validate_invariants()?;
    world.construction_inputs = Some(inputs);
    if let (Some(started), Some(validated)) = (started, validation_finished) {
        tracing::info!(
            target: "moli_cpu_profile",
            stage = "layout_construction_reuse",
            source_validation_us = validated.duration_since(started).as_micros(),
            graph_restore_us = validated.elapsed().as_micros(),
            checked_source_count,
            reused_construction_boxes = reused_boxes,
        );
    }
    Ok(Some(world))
}
