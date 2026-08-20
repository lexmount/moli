use std::{fmt, fmt::Debug, hash::Hash};

use crate::{
    LayoutAnonymousReason, LayoutBoxId, LayoutBoxKind, LayoutCapabilityDiagnostic, LayoutDisplay,
    LayoutElementSemantics, LayoutPseudo, LayoutWorld,
};

/// Versioned, allocation-ID-free representation of a constructed box tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedBoxTree {
    /// Schema version for golden and trace consumers.
    pub schema_version: u32,
    /// Normalized root box.
    pub root: NormalizedBoxNode,
}

/// One stable node in a [`NormalizedBoxTree`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedBoxNode {
    /// Child-index path such as `0/2/1`; never a layout allocator ID.
    pub path: String,
    /// Browser-level role assigned during construction.
    pub role: LayoutBoxKind,
    /// Exact computed display classification retained by construction.
    pub display: LayoutDisplay,
    /// Source label for principal and text boxes.
    pub source: Option<String>,
    /// Source label that owns an anonymous or pseudo box.
    pub owner: Option<String>,
    /// Pseudo origin, when this is generated content.
    pub pseudo: Option<LayoutPseudo>,
    /// Pass-owned qualified name and HTML semantic category for element boxes.
    pub element: Option<LayoutElementSemantics>,
    /// Construction reason for a box with no DOM source.
    pub anonymous_reason: Option<LayoutAnonymousReason>,
    /// Stable indication of later layout/paint capabilities still using fallback.
    pub capability_diagnostics: Vec<LayoutCapabilityDiagnostic>,
    /// Formatting context established by this box.
    pub formatting_context: Option<NormalizedFormattingContext>,
    /// Whether the box is removed from normal flow.
    pub out_of_flow: bool,
    /// Owned text payload, when present.
    pub text: Option<String>,
    /// Normalized children in box-tree order.
    pub children: Vec<NormalizedBoxNode>,
}

/// Stable formatting-context classification used by construction goldens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalizedFormattingContext {
    /// Normal block formatting context.
    Block,
    /// Basic inline formatting context.
    Inline,
    /// Flex formatting context.
    Flex,
    /// Grid formatting context; numeric grid layout can still be deferred.
    Grid,
    /// Structural table formatting context.
    Table,
}

impl<N> LayoutWorld<N>
where
    N: Copy + Debug + Eq + Hash,
{
    /// Projects the world into a stable tree suitable for tests and traces.
    pub fn normalized_tree(&self) -> NormalizedBoxTree {
        NormalizedBoxTree {
            schema_version: 3,
            root: self.normalize_box(self.root, "0".to_owned()),
        }
    }

    fn normalize_box(&self, id: LayoutBoxId, path: String) -> NormalizedBoxNode {
        let layout_box = &self.boxes[id.index()];
        let formatting_context = if layout_box.inline_formatting_context {
            Some(NormalizedFormattingContext::Inline)
        } else if layout_box.style.uses_flex_formatting_context() {
            Some(NormalizedFormattingContext::Flex)
        } else if layout_box.style.display().is_grid_container() {
            Some(NormalizedFormattingContext::Grid)
        } else if layout_box.style.display().is_table() {
            Some(NormalizedFormattingContext::Table)
        } else {
            match layout_box.kind {
                LayoutBoxKind::PrincipalFlex | LayoutBoxKind::PrincipalInlineFlex => {
                    Some(NormalizedFormattingContext::Flex)
                }
                LayoutBoxKind::PrincipalGrid | LayoutBoxKind::PrincipalInlineGrid => {
                    Some(NormalizedFormattingContext::Grid)
                }
                LayoutBoxKind::TableWrapper
                | LayoutBoxKind::InlineTableWrapper
                | LayoutBoxKind::TableRowGroup
                | LayoutBoxKind::TableHeaderGroup
                | LayoutBoxKind::TableFooterGroup
                | LayoutBoxKind::TableColumnGroup
                | LayoutBoxKind::TableColumn
                | LayoutBoxKind::TableRow
                | LayoutBoxKind::AnonymousTableWrapper
                | LayoutBoxKind::AnonymousTableRowGroup
                | LayoutBoxKind::AnonymousTableRow => Some(NormalizedFormattingContext::Table),
                LayoutBoxKind::PrincipalBlock
                | LayoutBoxKind::PrincipalFlowRoot
                | LayoutBoxKind::PrincipalInlineBlock
                | LayoutBoxKind::ImageFallback
                | LayoutBoxKind::ListItem
                | LayoutBoxKind::InlineListItem
                | LayoutBoxKind::TableCaption
                | LayoutBoxKind::TableCell
                | LayoutBoxKind::FormControl
                | LayoutBoxKind::AnonymousBlock
                | LayoutBoxKind::AnonymousFlexItem => Some(NormalizedFormattingContext::Block),
                LayoutBoxKind::AnonymousGridItem | LayoutBoxKind::AnonymousTableCell => {
                    Some(NormalizedFormattingContext::Block)
                }
                LayoutBoxKind::PrincipalInline
                | LayoutBoxKind::InlineContinuation
                | LayoutBoxKind::Text
                | LayoutBoxKind::LineBreak
                | LayoutBoxKind::PseudoMarker
                | LayoutBoxKind::PseudoBefore
                | LayoutBoxKind::PseudoAfter
                | LayoutBoxKind::Replaced => None,
            }
        };
        let children = layout_box
            .children
            .iter()
            .copied()
            .enumerate()
            .map(|(index, child)| self.normalize_box(child, format!("{path}/{index}")))
            .collect();

        NormalizedBoxNode {
            path,
            role: layout_box.kind,
            display: layout_box.style.display(),
            source: layout_box
                .source
                .is_some()
                .then(|| layout_box.source_label.clone()),
            owner: layout_box.owner_label.clone(),
            pseudo: layout_box.pseudo,
            element: layout_box.element_semantics.clone(),
            anonymous_reason: layout_box.anonymous_reason,
            capability_diagnostics: layout_box.capability_diagnostics.clone(),
            formatting_context,
            out_of_flow: layout_box.style.is_out_of_flow(),
            text: layout_box.text.as_deref().map(str::to_owned),
            children,
        }
    }
}

impl fmt::Display for NormalizedBoxTree {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "box-tree-schema={}", self.schema_version)?;
        write_normalized_node(formatter, &self.root, 0)
    }
}

fn write_normalized_node(
    formatter: &mut fmt::Formatter<'_>,
    node: &NormalizedBoxNode,
    depth: usize,
) -> fmt::Result {
    for _ in 0..depth {
        formatter.write_str("  ")?;
    }
    formatter.write_str(node.role.debug_name())?;
    write!(formatter, " path={}", node.path)?;
    write!(formatter, " display={}", node.display.debug_name())?;
    if let Some(source) = &node.source {
        write!(formatter, " source={source}")?;
    }
    if let Some(owner) = &node.owner {
        write!(formatter, " owner={owner}")?;
    }
    if let Some(pseudo) = node.pseudo {
        write!(formatter, " pseudo={}", pseudo.label())?;
    }
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
    if let Some(reason) = node.anonymous_reason {
        write!(formatter, " anonymous={}", reason.debug_name())?;
    }
    for diagnostic in &node.capability_diagnostics {
        write!(formatter, " capability={}", diagnostic.code())?;
    }
    if let Some(formatting_context) = node.formatting_context {
        let label = match formatting_context {
            NormalizedFormattingContext::Block => "block",
            NormalizedFormattingContext::Inline => "inline",
            NormalizedFormattingContext::Flex => "flex",
            NormalizedFormattingContext::Grid => "grid",
            NormalizedFormattingContext::Table => "table",
        };
        write!(formatter, " fc={label}")?;
    }
    if node.out_of_flow {
        formatter.write_str(" out-of-flow")?;
    }
    if let Some(text) = &node.text {
        write!(formatter, " text={text:?}")?;
    }
    formatter.write_str("\n")?;
    for child in &node.children {
        write_normalized_node(formatter, child, depth + 1)?;
    }
    Ok(())
}
