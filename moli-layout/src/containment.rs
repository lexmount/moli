//! Box-owned CSS containment eligibility and derived layout/paint roles.
//!
//! Stylo owns computed containment bits, but whether those bits apply depends
//! on the exact principal box produced by construction. Keep that used-value
//! decision here so containing-block selection, stacking, geometry projection,
//! hit testing, and paint clipping all consume one Chromium-aligned answer.

use crate::world::{LayoutBox, LayoutBoxKind};

impl<N> LayoutBox<N> {
    /// Mirrors Blink's `LayoutObject::IsBox()` boundary.
    ///
    /// Non-atomic inline content is flattened into its owner IFC and has no
    /// independent CSS box on which transform containing-block semantics can
    /// apply. Table structural boxes are still boxes even though containment
    /// has a narrower eligibility rule for them.
    pub(crate) fn is_css_box(&self) -> bool {
        !self.inline_flattened
            && !matches!(
                self.kind,
                LayoutBoxKind::Text
                    | LayoutBoxKind::LineBreak
                    | LayoutBoxKind::PrincipalInline
                    | LayoutBoxKind::InlineContinuation
            )
    }

    /// Mirrors Blink's `IsEligibleForPaintOrLayoutContainment()` overrides.
    ///
    /// LayoutBox is eligible by default. Internal table sections, columns and
    /// rows explicitly reject paint/layout containment; table wrappers, cells,
    /// captions, replaced content and atomic inlines remain eligible.
    pub(crate) fn is_eligible_for_paint_or_layout_containment(&self) -> bool {
        self.is_css_box()
            && !matches!(
                self.kind,
                LayoutBoxKind::TableRowGroup
                    | LayoutBoxKind::TableHeaderGroup
                    | LayoutBoxKind::TableFooterGroup
                    | LayoutBoxKind::TableColumnGroup
                    | LayoutBoxKind::TableColumn
                    | LayoutBoxKind::TableRow
                    | LayoutBoxKind::AnonymousTableRowGroup
                    | LayoutBoxKind::AnonymousTableRow
            )
    }

    /// Mirrors Blink's `LayoutObject::IsEligibleForSizeContainment()`
    /// boundary. Ordinary CSS boxes opt in, while table layout objects reject
    /// size containment even where paint/layout containment may apply.
    pub(crate) fn is_eligible_for_size_containment(&self) -> bool {
        self.is_css_box()
            && !matches!(
                self.kind,
                LayoutBoxKind::TableWrapper
                    | LayoutBoxKind::InlineTableWrapper
                    | LayoutBoxKind::TableRowGroup
                    | LayoutBoxKind::TableHeaderGroup
                    | LayoutBoxKind::TableFooterGroup
                    | LayoutBoxKind::TableColumnGroup
                    | LayoutBoxKind::TableColumn
                    | LayoutBoxKind::TableRow
                    | LayoutBoxKind::TableCell
                    | LayoutBoxKind::AnonymousTableWrapper
                    | LayoutBoxKind::AnonymousTableRowGroup
                    | LayoutBoxKind::AnonymousTableRow
                    | LayoutBoxKind::AnonymousTableCell
            )
    }

    pub(crate) fn applies_any_size_containment(&self) -> bool {
        let containment = self.used_size_containment();
        containment.axes.width || containment.axes.height
    }

    /// Resolve computed containment to the used node-level layout protocol.
    ///
    /// Stylo owns the effective logical axes and physical fallback values;
    /// this box boundary owns principal-box eligibility. Remembered sizes can
    /// later be selected here without changing Taffy's numeric algorithms.
    pub(crate) fn used_size_containment(&self) -> taffy::SizeContainment {
        if self.is_eligible_for_size_containment() {
            self.style.size_containment()
        } else {
            taffy::SizeContainment::NONE
        }
    }

    pub(crate) fn establishes_positioned_containing_block(&self) -> bool {
        self.style.establishes_positioned_containing_block(
            self.is_css_box(),
            self.is_eligible_for_paint_or_layout_containment(),
        )
    }

    pub(crate) fn establishes_fixed_containing_block(&self) -> bool {
        self.style.establishes_fixed_containing_block(
            self.is_css_box(),
            self.is_eligible_for_paint_or_layout_containment(),
        )
    }

    pub(crate) fn creates_stacking_context(
        &self,
        is_root: bool,
        is_flex_or_grid_item: bool,
    ) -> bool {
        self.style.creates_stacking_context(
            is_root,
            is_flex_or_grid_item,
            self.is_eligible_for_paint_or_layout_containment(),
        )
    }

    pub(crate) fn clips_descendant_paint(&self) -> bool {
        self.style.clips_overflow()
            || (self.is_eligible_for_paint_or_layout_containment()
                && self.style.applies_paint_containment())
    }
}
