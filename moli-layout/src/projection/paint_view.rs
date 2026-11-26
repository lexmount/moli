use std::{borrow::Cow, cell::RefCell, collections::HashMap, fmt::Debug, hash::Hash, ops::Range};

use crate::{
    FrozenLayoutBox, FrozenLayoutTree, InlineSvgImageLookup, LayoutBoxGeometry, LayoutBoxId,
    LayoutClipChainId, LayoutClipNode, LayoutFragment, LayoutFragmentId, LayoutImageResource,
    LayoutReplacedKind, LayoutScrollExtent, LayoutViewport, LayoutWorld, PaintDiagnostic,
    layout_tree::FrozenInlineSvgInputs,
    source::{CssImageLookup, LiveReplacedImageLookup},
    stacking::PaintOrderEvent,
    style::ResolvedLayoutTransform,
};

use super::{OutputProjection, ProjectedCoordinateSpace, RetainedPaintData, ScrollProjection};

#[derive(Default)]
pub(crate) struct BoxPaintMetadata {
    pub(crate) direct_fragment: Option<LayoutFragmentId>,
    pub(super) inline_fragments: Range<usize>,
    pub(crate) content_clip: Option<LayoutClipChainId>,
}

/// The order and fragment references belong to the layout publication. Scrolling
/// only changes the coordinate spaces and scrollbar positions used to paint it.
#[derive(Default)]
pub(crate) struct PaintMetadata {
    pub(crate) boxes: Vec<BoxPaintMetadata>,
    pub(crate) events: Vec<PaintOrderEvent>,
}

enum PaintGeometry<'a, N> {
    LayoutPass {
        boxes: &'a [LayoutBoxGeometry],
        scroll_extents: &'a [LayoutScrollExtent],
        transforms: &'a [ResolvedLayoutTransform],
    },
    Published {
        boxes: &'a [FrozenLayoutBox<N>],
        scroll: &'a ScrollProjection<N>,
    },
}

pub(super) struct InlineSvgPaint<'a, N> {
    pub(super) root: N,
    pub(super) frozen: Option<&'a FrozenInlineSvgInputs<N>>,
    pub(super) lookup: &'a InlineSvgImageLookup<'a, N>,
    pub(super) images: &'a RefCell<HashMap<LayoutBoxId, Option<LayoutImageResource>>>,
}

pub(crate) struct PaintProjection<'a, N>
where
    N: Copy + Debug + Eq + Hash,
{
    pub(crate) world: &'a LayoutWorld<N>,
    pub(crate) viewport: LayoutViewport,
    pub(crate) coordinate_spaces: &'a [ProjectedCoordinateSpace],
    pub(crate) clip_chain: &'a [LayoutClipNode],
    pub(crate) fragments: &'a [LayoutFragment],
    pub(crate) diagnostics: &'a [PaintDiagnostic],
    pub(crate) metadata: &'a PaintMetadata,
    pub(crate) css_images: &'a CssImageLookup<'a>,
    live_replaced_images: Option<&'a LiveReplacedImageLookup<'a, N>>,
    inline_svg: Option<InlineSvgPaint<'a, N>>,
    geometry: PaintGeometry<'a, N>,
}

impl<'a, N: Copy + Debug + Eq + Hash> PaintProjection<'a, N> {
    pub(super) fn from_layout_pass(
        pass: &'a OutputProjection<'_, N>,
        css_images: &'a CssImageLookup<'a>,
        inline_svg: Option<InlineSvgPaint<'a, N>>,
    ) -> Self {
        Self {
            world: pass.world,
            viewport: pass.viewport,
            coordinate_spaces: &pass.coordinate_spaces,
            clip_chain: &pass.clip_chain,
            fragments: &pass.fragments,
            diagnostics: &pass.diagnostics,
            metadata: &pass.paint_metadata,
            css_images,
            live_replaced_images: None,
            inline_svg,
            geometry: PaintGeometry::LayoutPass {
                boxes: &pass.boxes,
                scroll_extents: &pass.scroll_extents,
                transforms: &pass.resolved_transforms,
            },
        }
    }

    pub(super) fn from_publication(
        world: &'a LayoutWorld<N>,
        paint: &'a RetainedPaintData,
        tree: &'a FrozenLayoutTree<N>,
        scroll: &'a ScrollProjection<N>,
        coordinate_spaces: &'a [ProjectedCoordinateSpace],
        css_images: &'a CssImageLookup<'a>,
        live_replaced_images: &'a LiveReplacedImageLookup<'a, N>,
        inline_svg: InlineSvgPaint<'a, N>,
    ) -> Self {
        Self {
            world,
            viewport: tree.viewport,
            coordinate_spaces,
            clip_chain: &tree.clip_chain,
            fragments: &tree.fragments,
            diagnostics: &paint.diagnostics,
            metadata: &paint.paint_metadata,
            css_images,
            live_replaced_images: Some(live_replaced_images),
            inline_svg: Some(inline_svg),
            geometry: PaintGeometry::Published {
                boxes: &tree.boxes,
                scroll,
            },
        }
    }

    pub(crate) fn replaced_image(&self, id: LayoutBoxId) -> Option<Cow<'a, LayoutImageResource>> {
        let layout_box = &self.world.boxes[id.index()];
        if let Some(source) = layout_box.source()
            && let Some(kind) = layout_box
                .element_semantics()
                .and_then(|element| element.replaced)
        {
            if kind == LayoutReplacedKind::Svg {
                let svg = self.inline_svg.as_ref()?;
                return svg
                    .images
                    .borrow_mut()
                    .entry(id)
                    .or_insert_with(|| {
                        (svg.lookup)(
                            svg.root,
                            self.viewport,
                            source,
                            &layout_box.style,
                            crate::pass::svg_viewport_size(layout_box),
                            svg.frozen.and_then(|frozen| frozen.input(source)),
                        )
                    })
                    .clone()
                    .map(Cow::Owned);
            }
            if kind.is_live_paint_image()
                && let Some(lookup) = self.live_replaced_images
            {
                return lookup(source, kind).map(Cow::Owned);
            }
        }
        layout_box.replaced_image.as_ref().map(Cow::Borrowed)
    }

    pub(crate) fn geometry(&self, index: usize) -> &LayoutBoxGeometry {
        match self.geometry {
            PaintGeometry::LayoutPass { boxes, .. } => &boxes[index],
            PaintGeometry::Published { boxes, .. } => &boxes[index].geometry,
        }
    }

    pub(crate) fn scroll_extent(&self, index: usize) -> &LayoutScrollExtent {
        match self.geometry {
            PaintGeometry::LayoutPass { scroll_extents, .. } => &scroll_extents[index],
            PaintGeometry::Published { boxes, .. } => &boxes[index].scroll_extent,
        }
    }

    pub(crate) fn inline_fragments(&self, id: LayoutBoxId) -> &[LayoutFragment] {
        &self.fragments[self.metadata.boxes[id.index()].inline_fragments.clone()]
    }

    pub(crate) fn resolved_transform_has_unsupported_3d(&self, id: LayoutBoxId) -> bool {
        match self.geometry {
            PaintGeometry::LayoutPass { transforms, .. } => transforms[id.index()],
            PaintGeometry::Published { scroll, .. } => scroll.resolved_transform(id.index()),
        }
        .has_unsupported_3d
    }
}
