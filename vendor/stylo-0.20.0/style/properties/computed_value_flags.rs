/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Misc information about a given computed style.

use crate::derives::MallocSizeOf;

/// Misc information about a given computed style.
///
/// All flags are currently inherited for text, pseudo elements, and
/// anonymous boxes, see StyleBuilder::for_inheritance and its callsites.
/// If we ever want to add some flags that shouldn't inherit for them,
/// we might want to add a function to handle this.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, MallocSizeOf, PartialEq)]
pub struct ComputedValueFlags(u32);

bitflags! {
    impl ComputedValueFlags: u32 {
        /// Whether the style or any of the ancestors has a text-decoration-line
        /// property that should get propagated to descendants.
        ///
        /// text-decoration-line is a reset property, but gets propagated in the
        /// frame/box tree.
        const HAS_TEXT_DECORATION_LINES = 1 << 0;

        /// Whether line break inside should be suppressed.
        ///
        /// If this flag is set, the line should not be broken inside,
        /// which means inlines act as if nowrap is set, <br> element is
        /// suppressed, and blocks are inlinized.
        ///
        /// This bit is propagated to all children of line participants.
        /// It is currently used by ruby to make its content unbreakable.
        const SHOULD_SUPPRESS_LINEBREAK = 1 << 1;

        /// A flag used to mark text that that has text-combine-upright.
        ///
        /// This is used from Gecko's layout engine.
        const IS_TEXT_COMBINED = 1 << 2;

        /// A flag used to mark styles under a relevant link that is also
        /// visited.
        const IS_RELEVANT_LINK_VISITED = 1 << 3;

        /// A flag used to mark styles which are a ::first-line or under one.
        const IS_IN_FIRST_LINE_SUBTREE = 1 << 4;

        /// A flag used to mark styles which have contain:style or under one.
        const SELF_OR_ANCESTOR_HAS_CONTAIN_STYLE = 1 << 5;

        /// Whether this style's `display` or `content`property depends on our parent style.
        ///
        /// This is important because it may affect our optimizations to avoid
        /// computing the style of pseudo-elements, given whether the
        /// pseudo-element is generated depends on the `display` (or `content`) value.
        const DISPLAY_OR_CONTENT_DEPEND_ON_INHERITED_STYLE = 1 << 6;

        /// Whether the child explicitly inherits any reset property.
        const INHERITS_RESET_STYLE = 1 << 7;

        /// Whether any value on our style is font-metric-dependent on our
        /// primary font.
        const DEPENDS_ON_SELF_FONT_METRICS = 1 << 8;

        /// Whether any value on our style is font-metric-dependent on the
        /// primary font of our parent.
        const DEPENDS_ON_INHERITED_FONT_METRICS = 1 << 9;

        /// Whether this style is the style of the document element.
        const IS_ROOT_ELEMENT_STYLE = 1 << 10;

        /// Whether this element is inside an `opacity: 0` subtree.
        const IS_IN_OPACITY_ZERO_SUBTREE = 1 << 11;

        /// Whether there are author-specified rules for border-* properties
        /// (except border-image-*), background-color, or background-image.
        ///
        /// TODO(emilio): Maybe do include border-image, see:
        ///
        /// https://github.com/w3c/csswg-drafts/issues/4777#issuecomment-604424845
        const HAS_AUTHOR_SPECIFIED_BORDER_BACKGROUND = 1 << 12;

        /// Whether the style depends on viewport units.
        const USES_VIEWPORT_UNITS = 1 << 13;

        /// Whether the style depends on viewport units on container queries.
        ///
        /// This needs to be a separate flag from `USES_VIEWPORT_UNITS` because
        /// it causes us to re-match the style (rather than re-cascascading it,
        /// which is enough for other uses of viewport units).
        const USES_VIEWPORT_UNITS_ON_CONTAINER_QUERIES = 1 << 14;

        /// A flag used to mark styles which have `container-type` of `size` or
        /// `inline-size`, or under one.
        const SELF_OR_ANCESTOR_HAS_SIZE_CONTAINER_TYPE = 1 << 15;

        /// Whether the style uses container query units, in which case the style depends on the
        /// container's size and we can't reuse it across cousins (without double-checking the
        /// container at least).
        const USES_CONTAINER_UNITS = 1 << 16;

        /// Whether there are author-specific rules for text `color`.
        const HAS_AUTHOR_SPECIFIED_TEXT_COLOR = 1 << 17;

        /// Whether this style considered a scope style rule.
        const CONSIDERED_NONTRIVIAL_SCOPED_STYLE = 1 << 18;

        /// Whether this style is that of a `display: contents` element that is either a direct
        /// child of an item container or another `display: contents` element, the style of which
        /// has this flag set, marked in order to cascade beyond them to the descendants of the
        /// the item container that do generate a box.
        const DISPLAY_CONTENTS_IN_ITEM_CONTAINER = 1 << 19;

        /// Whether there are author-specific rules for `text-shadow`.
        const HAS_AUTHOR_SPECIFIED_TEXT_SHADOW = 1 << 20;

        /// Whether this style depends on container style query.
        const DEPENDS_ON_CONTAINER_STYLE_QUERY = 1 << 21;

        /// Whether this style is in an appearance: base subtree
        const IS_IN_APPEARANCE_BASE_SUBTREE = 1 << 22;

        /// Whether grid-auto-flow is author-specified.
        const HAS_AUTHOR_SPECIFIED_GRID_AUTO_FLOW = 1 << 23;

        /// Whether this style has used font relative units. Note that this is different than the
        /// FONT_METRICS bits, which don't include rem / em etc.
        const USES_FONT_RELATIVE_UNITS = 1 << 24;

        /// Whether this style depends on font relative units in container queries.
        const USES_FONT_RELATIVE_UNITS_ON_CONTAINER_QUERIES = 1 << 25;

        /// Whether this style uses `sibling-count()`.
        const USES_SIBLING_COUNT = 1 << 26;

        /// Whether this style uses `sibling-index()`.
        const USES_SIBLING_INDEX = 1 << 27;
    }
}

impl Default for ComputedValueFlags {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

impl ComputedValueFlags {
    /// Flags that are unconditionally propagated to descendants.
    #[inline]
    fn inherited_flags() -> Self {
        Self::IS_RELEVANT_LINK_VISITED
            | Self::IS_IN_FIRST_LINE_SUBTREE
            | Self::HAS_TEXT_DECORATION_LINES
            | Self::IS_IN_OPACITY_ZERO_SUBTREE
            | Self::SELF_OR_ANCESTOR_HAS_CONTAIN_STYLE
            | Self::SELF_OR_ANCESTOR_HAS_SIZE_CONTAINER_TYPE
            | Self::IS_IN_APPEARANCE_BASE_SUBTREE
    }

    /// Flags that may be propagated to descendants.
    #[inline]
    fn maybe_inherited_flags() -> Self {
        Self::inherited_flags()
            | Self::SHOULD_SUPPRESS_LINEBREAK
            | Self::DISPLAY_CONTENTS_IN_ITEM_CONTAINER
    }

    /// Flags that are an input to the cascade.
    #[inline]
    fn cascade_input_flags() -> Self {
        Self::USES_VIEWPORT_UNITS_ON_CONTAINER_QUERIES
            | Self::CONSIDERED_NONTRIVIAL_SCOPED_STYLE
            | Self::DEPENDS_ON_CONTAINER_STYLE_QUERY
            | Self::USES_FONT_RELATIVE_UNITS_ON_CONTAINER_QUERIES
    }

    /// Flags corresponding to usage of tree-counting functions.
    #[inline]
    pub fn tree_counting_function_flags() -> Self {
        Self::USES_SIBLING_COUNT | Self::USES_SIBLING_INDEX
    }

    /// Returns the flags that are always propagated to descendants.
    ///
    /// See StyleAdjuster::set_bits and StyleBuilder.
    #[inline]
    pub fn inherited(self) -> Self {
        self & Self::inherited_flags()
    }

    /// Flags that are conditionally propagated to descendants, just to handle
    /// properly style invalidation.
    #[inline]
    pub fn maybe_inherited(self) -> Self {
        self & Self::maybe_inherited_flags()
    }

    /// Flags that are an input to the cascade.
    #[inline]
    pub fn for_cascade_inputs(self) -> Self {
        self & Self::cascade_input_flags()
    }
}
