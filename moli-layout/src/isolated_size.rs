//! Size-only layout for independent positioned text formatting contexts.
//!
//! These results never replace a published document tree. A caller must keep
//! document invalidations pending for subsequent full-geometry consumers.

use std::{collections::HashSet, fmt::Debug, hash::Hash};

use crate::{
    DocumentLayoutServices, LayoutDisplay, LayoutElementCategory, LayoutElementSemantics,
    LayoutError, LayoutNamespace, LayoutPassMetrics, LayoutPassRequest, LayoutPosition, LayoutSize,
    LayoutSource, LayoutSourceKind, LayoutStyleResolver, LayoutUsedSize, ResolvedLayoutStyle,
    build_layout_pass,
};

const MAX_SOURCES: usize = 64;
const MAX_TEXT_BYTES: usize = 8192;

/// Bounded sizes for one independent layout region, without live styles,
/// working trees, positions, scroll state, or paint inputs.
pub struct IsolatedSizeLayout<N> {
    root: N,
    sizes: Vec<(N, LayoutSize, Option<LayoutUsedSize>)>,
    metrics: LayoutPassMetrics,
}

impl<N: Copy + Eq> IsolatedSizeLayout<N> {
    pub fn root(&self) -> N {
        self.root
    }

    pub fn offset_size(&self, source: N) -> Option<LayoutSize> {
        self.sizes
            .iter()
            .find_map(|(node, size, _)| (*node == source).then_some(*size))
    }

    pub fn used_size(&self, source: N) -> Option<Option<LayoutUsedSize>> {
        self.sizes
            .iter()
            .find_map(|(node, _, size)| (*node == source).then_some(*size))
    }

    pub fn metrics(&self) -> LayoutPassMetrics {
        self.metrics
    }
}

/// Measures a small text subtree whose positioned containing block is the
/// initial viewport. Unsupported dependencies retain the full-layout path.
pub fn try_build_isolated_sizes<S, R>(
    source: &S,
    styles: &mut R,
    services: &mut DocumentLayoutServices,
    target: S::NodeId,
    request: LayoutPassRequest,
) -> Result<Option<IsolatedSizeLayout<S::NodeId>>, LayoutError>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
{
    if !request.environment().scrollbars_hidden
        || request.viewport.mobile_frame_size().is_some()
        || source.quirks_mode() != style::context::QuirksMode::NoQuirks
        || source.node_kind(target) != LayoutSourceKind::Element
    {
        return Ok(None);
    }

    let mut ancestors = Vec::new();
    let mut current = Some(target);
    while let Some(node) = current {
        if ancestors.len() == MAX_SOURCES || ancestors.contains(&node) {
            return Ok(None);
        }
        ancestors.push(node);
        current = source.flat_parent(node);
    }
    if ancestors.last() != Some(&source.root()) {
        return Ok(None);
    }

    let mut root = None;
    let mut path = Vec::new();
    for node in ancestors.into_iter().rev() {
        path.push(node);
        if source.element_semantics(node).is_none_or(|semantics| {
            semantics.namespace != LayoutNamespace::Html
                || semantics.category != LayoutElementCategory::Generic
                || semantics.is_replaced()
        }) {
            return Ok(None);
        }
        let Some(bundle) = styles.element_styles(node)? else {
            return Ok(None);
        };
        let (style, before, after) = bundle.into_parts();
        if !plain_text_style(&style)
            || before.is_some()
            || after.is_some()
            || !style.has_normal_generated_content()
        {
            return Ok(None);
        }
        if matches!(
            style.position(),
            LayoutPosition::Absolute | LayoutPosition::Fixed
        ) {
            // Clipping preceding siblings changes static positions. Both axes
            // need a positioned origin: subpixel rounding subtracts rounded
            // edges, so even a vertical position can change the reported size.
            if !matches!(
                style.display(),
                LayoutDisplay::Block | LayoutDisplay::FlowRoot
            ) || (style.taffy.inset.left.is_auto() && style.taffy.inset.right.is_auto())
                || (style.taffy.inset.top.is_auto() && style.taffy.inset.bottom.is_auto())
            {
                return Ok(None);
            }
            root = Some(node);
            break;
        }
        if style.establishes_positioned_containing_block(node == source.root(), true, true)
            || style.is_out_of_flow()
            || style.viewport_scrollbar_gutter() != crate::LayoutScrollbarGutter::Auto
        {
            return Ok(None);
        }
    }
    let Some(root) = root else {
        return Ok(None);
    };

    let mut pending = vec![root];
    let mut seen = HashSet::new();
    let mut elements = Vec::new();
    let mut text_bytes = 0usize;
    while let Some(node) = pending.pop() {
        if seen.len() == MAX_SOURCES || !seen.insert(node) {
            return Ok(None);
        }
        match source.node_kind(node) {
            LayoutSourceKind::Element => {
                let Some(semantics) = source.element_semantics(node) else {
                    return Ok(None);
                };
                if semantics.namespace != LayoutNamespace::Html
                    || semantics.is_replaced()
                    || semantics.metadata.form_control.is_some()
                    || semantics.metadata.list.is_some()
                    || semantics.metadata.table.is_some()
                {
                    return Ok(None);
                }
                let Some(bundle) = styles.element_styles(node)? else {
                    return Ok(None);
                };
                let (style, before, after) = bundle.into_parts();
                if !plain_text_style(&style)
                    || before.is_some()
                    || after.is_some()
                    || !style.has_normal_generated_content()
                    || style.changes_counters()
                    || (node != root
                        && (style.is_out_of_flow()
                            || !matches!(
                                style.position(),
                                LayoutPosition::Static | LayoutPosition::Relative
                            )))
                {
                    return Ok(None);
                }
                elements.push(node);
            }
            LayoutSourceKind::Text => {
                text_bytes = text_bytes.saturating_add(source.text(node).map_or(0, str::len));
                if text_bytes > MAX_TEXT_BYTES {
                    return Ok(None);
                }
            }
            LayoutSourceKind::Comment => {}
            _ => return Ok(None),
        }
        for child in source.flat_children(node) {
            if pending.len() + seen.len() == MAX_SOURCES {
                return Ok(None);
            }
            pending.push(child);
        }
    }

    // Keep the real viewport root and its ancestor chain. Making the region
    // itself a layout root would give auto/percentage dimensions viewport-root
    // semantics instead of ordinary absolute-positioned box semantics.
    let view = SizeSource { source, path };
    let pass = build_layout_pass(&view, styles, services, request)?;
    pass.validate_retention_budget()?;
    let sizes = elements
        .into_iter()
        .map(|node| {
            let size = pass
                .tree
                .element_metrics_for_source(node)
                .map_or(LayoutSize::ZERO, |metrics| metrics.offset_size);
            (node, size, pass.tree.used_size_for_source(node))
        })
        .collect();
    Ok(Some(IsolatedSizeLayout {
        root,
        sizes,
        metrics: pass.metrics,
    }))
}

fn plain_text_style(style: &ResolvedLayoutStyle) -> bool {
    style.supports_isolated_text_measurement()
}

struct SizeSource<'a, S: LayoutSource> {
    source: &'a S,
    path: Vec<S::NodeId>,
}

impl<S: LayoutSource> LayoutSource for SizeSource<'_, S>
where
    S::NodeId: Copy + Debug + Eq + Hash,
{
    type NodeId = S::NodeId;
    type ChildIter<'a>
        = std::vec::IntoIter<S::NodeId>
    where
        Self: 'a;

    fn root(&self) -> Self::NodeId {
        self.source.root()
    }
    fn root_is_document_element(&self) -> bool {
        self.source.root_is_document_element()
    }
    fn quirks_mode(&self) -> style::context::QuirksMode {
        self.source.quirks_mode()
    }
    fn flat_parent(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        self.source.flat_parent(node)
    }
    fn flat_children(&self, node: Self::NodeId) -> Self::ChildIter<'_> {
        if let Some(index) = self.path.iter().position(|ancestor| *ancestor == node)
            && let Some(child) = self.path.get(index + 1)
        {
            return vec![*child].into_iter();
        }
        self.source
            .flat_children(node)
            .collect::<Vec<_>>()
            .into_iter()
    }
    fn node_kind(&self, node: Self::NodeId) -> LayoutSourceKind {
        self.source.node_kind(node)
    }
    fn element_semantics(&self, node: Self::NodeId) -> Option<LayoutElementSemantics> {
        self.source.element_semantics(node)
    }
    fn text(&self, node: Self::NodeId) -> Option<&str> {
        self.source.text(node)
    }
    fn label(&self, node: Self::NodeId) -> String {
        self.source.label(node)
    }
}
