/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Generic types for CSS handling of specified and computed values of
//! [`position`](https://drafts.csswg.org/css-backgrounds-3/#position)

use cssparser::Parser;
use std::fmt::Write;

use style_derive::Animate;
use style_traits::CssWriter;
use style_traits::ParseError;
use style_traits::SpecifiedValueInfo;
use style_traits::ToCss;

use crate::derives::*;
use crate::logical_geometry::PhysicalSide;
use crate::parser::{Parse, ParserContext};
use crate::rule_tree::CascadeLevel;
use crate::values::animated::ToAnimatedZero;
use crate::values::computed::position::TryTacticAdjustment;
use crate::values::generics::box_::PositionProperty;
use crate::values::generics::length::GenericAnchorSizeFunction;
use crate::values::generics::ratio::Ratio;
use crate::values::generics::Optional;
use crate::values::DashedIdent;

use crate::values::computed::Context;
use crate::values::computed::ToComputedValue;

/// Trait to check if the value of a potentially-tree-scoped type T
/// is actually tree-scoped. e.g. `none` value of `anchor-scope` should
/// not be tree-scoped.
pub trait IsTreeScoped {
    /// Returns true if the current value should be considered tree-scoped.
    /// Default implementation assumes that the value is always tree-scoped.
    fn is_tree_scoped(&self) -> bool {
        true
    }
}

/// A generic type for representing a value scoped to a specific cascade level
/// in the shadow tree hierarchy.
#[repr(C)]
#[derive(
    Clone,
    Copy,
    Debug,
    MallocSizeOf,
    SpecifiedValueInfo,
    ToAnimatedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
    ToTyped,
    Serialize,
    Deserialize,
)]
pub struct TreeScoped<T> {
    /// The scoped value.
    pub value: T,
    /// The cascade level in the shadow tree hierarchy.
    #[css(skip)]
    pub scope: CascadeLevel,
}

impl<T: IsTreeScoped + PartialEq> PartialEq for TreeScoped<T> {
    fn eq(&self, other: &Self) -> bool {
        let tree_scoped = self.value.is_tree_scoped();
        if tree_scoped != other.value.is_tree_scoped() {
            // Trivially different.
            return false;
        }
        let scopes_equal = self.scope == other.scope;
        if !scopes_equal && tree_scoped {
            // Scope difference matters if the name is actually tree-scoped.
            return false;
        }
        // Ok, do the actual value comparison.
        self.value == other.value
    }
}

impl<T> TreeScoped<T> {
    /// Creates a new `TreeScoped` value.
    pub fn new(value: T, scope: CascadeLevel) -> Self {
        Self { value, scope }
    }

    /// Creates a new `TreeScoped` value with the default cascade level
    /// (same tree author normal).
    pub fn with_default_level(value: T) -> Self {
        Self {
            value,
            scope: CascadeLevel::same_tree_author_normal(),
        }
    }
}

impl<T> Parse for TreeScoped<T>
where
    T: Parse,
{
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        Ok(TreeScoped {
            value: T::parse(context, input)?,
            scope: CascadeLevel::same_tree_author_normal(),
        })
    }
}

impl<T> ToComputedValue for TreeScoped<T>
where
    T: ToComputedValue + IsTreeScoped,
    T::ComputedValue: IsTreeScoped,
{
    type ComputedValue = TreeScoped<T::ComputedValue>;
    fn to_computed_value(&self, context: &Context) -> Self::ComputedValue {
        TreeScoped {
            value: self.value.to_computed_value(context),
            scope: if context.current_scope().is_tree() {
                context.current_scope()
            } else {
                self.scope.clone()
            },
        }
    }

    fn from_computed_value(computed: &Self::ComputedValue) -> Self {
        Self {
            value: ToComputedValue::from_computed_value(&computed.value),
            scope: computed.scope.clone(),
        }
    }
}

/// A generic type for representing a CSS [position](https://drafts.csswg.org/css-values/#position).
#[derive(
    Animate,
    Clone,
    ComputeSquaredDistance,
    Copy,
    Debug,
    Deserialize,
    MallocSizeOf,
    PartialEq,
    Serialize,
    SpecifiedValueInfo,
    ToAnimatedValue,
    ToAnimatedZero,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
    ToTyped,
)]
#[repr(C)]
pub struct GenericPosition<H, V> {
    /// The horizontal component of position.
    pub horizontal: H,
    /// The vertical component of position.
    pub vertical: V,
}

impl<H, V> PositionComponent for Position<H, V>
where
    H: PositionComponent,
    V: PositionComponent,
{
    #[inline]
    fn is_center(&self) -> bool {
        self.horizontal.is_center() && self.vertical.is_center()
    }
}

pub use self::GenericPosition as Position;

impl<H, V> Position<H, V> {
    /// Returns a new position.
    pub fn new(horizontal: H, vertical: V) -> Self {
        Self {
            horizontal,
            vertical,
        }
    }
}

/// Implements a method that checks if the position is centered.
pub trait PositionComponent {
    /// Returns if the position component is 50% or center.
    /// For pixel lengths, it always returns false.
    fn is_center(&self) -> bool;
}

/// A generic type for representing an `Auto | <position>`.
/// This is used by <offset-anchor> for now.
/// https://drafts.fxtf.org/motion-1/#offset-anchor-property
#[derive(
    Animate,
    Clone,
    ComputeSquaredDistance,
    Copy,
    Debug,
    Deserialize,
    MallocSizeOf,
    Parse,
    PartialEq,
    Serialize,
    SpecifiedValueInfo,
    ToAnimatedZero,
    ToAnimatedValue,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
    ToTyped,
)]
#[repr(C, u8)]
pub enum GenericPositionOrAuto<Pos> {
    /// The <position> value.
    Position(Pos),
    /// The keyword `auto`.
    Auto,
}

pub use self::GenericPositionOrAuto as PositionOrAuto;

impl<Pos> PositionOrAuto<Pos> {
    /// Return `auto`.
    #[inline]
    pub fn auto() -> Self {
        PositionOrAuto::Auto
    }

    /// Return true if it is 'auto'.
    #[inline]
    pub fn is_auto(&self) -> bool {
        matches!(self, PositionOrAuto::Auto)
    }
}

/// A generic value for the `z-index` property.
#[derive(
    Animate,
    Clone,
    ComputeSquaredDistance,
    Copy,
    Debug,
    MallocSizeOf,
    PartialEq,
    Parse,
    SpecifiedValueInfo,
    ToAnimatedValue,
    ToAnimatedZero,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
    ToTyped,
)]
#[repr(C, u8)]
pub enum GenericZIndex<I> {
    /// An integer value.
    Integer(I),
    /// The keyword `auto`.
    Auto,
}

pub use self::GenericZIndex as ZIndex;

impl<Integer> ZIndex<Integer> {
    /// Returns `auto`
    #[inline]
    pub fn auto() -> Self {
        ZIndex::Auto
    }

    /// Returns whether `self` is `auto`.
    #[inline]
    pub fn is_auto(self) -> bool {
        matches!(self, ZIndex::Auto)
    }

    /// Returns the integer value if it is an integer, or `auto`.
    #[inline]
    pub fn integer_or(self, auto: Integer) -> Integer {
        match self {
            ZIndex::Integer(n) => n,
            ZIndex::Auto => auto,
        }
    }
}

/// Ratio or None.
#[derive(
    Animate,
    Clone,
    ComputeSquaredDistance,
    Copy,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToAnimatedValue,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C, u8)]
pub enum PreferredRatio<N> {
    /// Without specified ratio
    #[css(skip)]
    None,
    /// With specified ratio
    Ratio(
        #[animation(field_bound)]
        #[css(field_bound)]
        #[distance(field_bound)]
        Ratio<N>,
    ),
}

/// A generic value for the `aspect-ratio` property, the value is `auto || <ratio>`.
#[derive(
    Animate,
    Clone,
    ComputeSquaredDistance,
    Copy,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToAnimatedValue,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
    ToTyped,
)]
#[repr(C)]
#[typed(todo_derive_fields)]
pub struct GenericAspectRatio<N> {
    /// Specifiy auto or not.
    #[animation(constant)]
    #[css(represents_keyword)]
    pub auto: bool,
    /// The preferred aspect-ratio value.
    #[animation(field_bound)]
    #[css(field_bound)]
    #[distance(field_bound)]
    pub ratio: PreferredRatio<N>,
}

pub use self::GenericAspectRatio as AspectRatio;

impl<N> AspectRatio<N> {
    /// Returns `auto`
    #[inline]
    pub fn auto() -> Self {
        AspectRatio {
            auto: true,
            ratio: PreferredRatio::None,
        }
    }
}

impl<N> ToAnimatedZero for AspectRatio<N> {
    #[inline]
    fn to_animated_zero(&self) -> Result<Self, ()> {
        Err(())
    }
}

/// Specified type for `inset` properties, which allows
/// the use of the `anchor()` function.
/// Note(dshin): `LengthPercentageOrAuto` is not used here because
/// having `LengthPercentageOrAuto` and `AnchorFunction` in the enum
/// pays the price of the discriminator for `LengthPercentage | Auto`
/// as well as `LengthPercentageOrAuto | AnchorFunction`. This increases
/// the size of the style struct, which would not be great.
/// On the other hand, we trade for code duplication, so... :(
#[derive(
    Animate,
    Clone,
    ComputeSquaredDistance,
    Debug,
    MallocSizeOf,
    PartialEq,
    ToCss,
    ToShmem,
    ToAnimatedValue,
    ToAnimatedZero,
    ToComputedValue,
    ToResolvedValue,
    ToTyped,
)]
#[repr(C)]
pub enum GenericInset<P, LP> {
    /// A `<length-percentage>` value.
    LengthPercentage(LP),
    /// An `auto` value.
    Auto,
    /// Inset defined by the anchor element.
    ///
    /// <https://drafts.csswg.org/css-anchor-position-1/#anchor-pos>
    AnchorFunction(Box<GenericAnchorFunction<P, Self>>),
    /// Inset defined by the size of the anchor element.
    ///
    /// <https://drafts.csswg.org/css-anchor-position-1/#anchor-pos>
    AnchorSizeFunction(Box<GenericAnchorSizeFunction<Self>>),
    /// A `<length-percentage>` value, guaranteed to contain `calc()`,
    /// which then is guaranteed to contain `anchor()` or `anchor-size()`.
    AnchorContainingCalcFunction(LP),
}

impl<P, LP> SpecifiedValueInfo for GenericInset<P, LP>
where
    LP: SpecifiedValueInfo,
{
    fn collect_completion_keywords(f: style_traits::KeywordsCollectFn) {
        LP::collect_completion_keywords(f);
        f(&["auto"]);
        if static_prefs::pref!("layout.css.anchor-positioning.enabled") {
            f(&["anchor", "anchor-size"]);
        }
    }
}

impl<P, LP> GenericInset<P, LP> {
    /// `auto` value.
    #[inline]
    pub fn auto() -> Self {
        Self::Auto
    }

    /// Return true if it is 'auto'.
    #[inline]
    #[cfg(feature = "servo")]
    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }
}

pub use self::GenericInset as Inset;

/// Anchor function used by inset properties. This resolves
/// to length at computed time.
///
/// https://drafts.csswg.org/css-anchor-position-1/#funcdef-anchor
#[derive(
    Animate,
    Clone,
    ComputeSquaredDistance,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToShmem,
    ToAnimatedValue,
    ToAnimatedZero,
    ToComputedValue,
    ToResolvedValue,
    Serialize,
    Deserialize,
    ToTyped,
)]
#[repr(C)]
#[typed(todo_derive_fields)]
pub struct GenericAnchorFunction<Percentage, Fallback> {
    /// Anchor name of the element to anchor to.
    /// If omitted, selects the implicit anchor element.
    /// The shadow cascade order of the tree-scoped anchor name
    /// associates the name with the host of the originating stylesheet.
    #[animation(constant)]
    pub target_element: TreeScoped<DashedIdent>,
    /// Where relative to the target anchor element to position
    /// the anchored element to.
    pub side: GenericAnchorSide<Percentage>,
    /// Value to use in case the anchor function is invalid.
    pub fallback: Optional<Fallback>,
}

impl<Percentage, Fallback> ToCss for GenericAnchorFunction<Percentage, Fallback>
where
    Percentage: ToCss,
    Fallback: ToCss,
{
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> std::fmt::Result
    where
        W: Write,
    {
        dest.write_str("anchor(")?;
        if !self.target_element.value.is_empty() {
            self.target_element.to_css(dest)?;
            dest.write_str(" ")?;
        }
        self.side.to_css(dest)?;
        if let Some(f) = self.fallback.as_ref() {
            // This comma isn't really `derive()`-able, unfortunately.
            dest.write_str(", ")?;
            f.to_css(dest)?;
        }
        dest.write_str(")")
    }
}

impl<Percentage, Fallback> GenericAnchorFunction<Percentage, Fallback> {
    /// Is the anchor valid for given property?
    pub fn valid_for(&self, side: PhysicalSide, position_property: PositionProperty) -> bool {
        position_property.is_absolutely_positioned() && self.side.valid_for(side)
    }
}

/// Keyword values for the anchor positioning function.
#[derive(
    Animate,
    Clone,
    ComputeSquaredDistance,
    Copy,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToCss,
    ToShmem,
    Parse,
    ToAnimatedValue,
    ToAnimatedZero,
    ToComputedValue,
    ToResolvedValue,
    Serialize,
    Deserialize,
)]
#[repr(u8)]
pub enum AnchorSideKeyword {
    /// Inside relative (i.e. Same side) to the inset property it's used in.
    Inside,
    /// Same as above, but outside (i.e. Opposite side).
    Outside,
    /// Top of the anchor element.
    Top,
    /// Left of the anchor element.
    Left,
    /// Right of the anchor element.
    Right,
    /// Bottom of the anchor element.
    Bottom,
    /// Refers to the start side of the anchor element for the same axis of the inset
    /// property it's used in, resolved against the positioned element's containing
    /// block's writing mode.
    Start,
    /// Same as above, but for the end side.
    End,
    /// Same as `start`, resolved against the positioned element's writing mode.
    SelfStart,
    /// Same as above, but for the end side.
    SelfEnd,
    /// Halfway between `start` and `end` sides.
    Center,
}

impl AnchorSideKeyword {
    fn from_physical_side(side: PhysicalSide) -> Self {
        match side {
            PhysicalSide::Top => Self::Top,
            PhysicalSide::Right => Self::Right,
            PhysicalSide::Bottom => Self::Bottom,
            PhysicalSide::Left => Self::Left,
        }
    }

    fn physical_side(self) -> Option<PhysicalSide> {
        Some(match self {
            Self::Top => PhysicalSide::Top,
            Self::Right => PhysicalSide::Right,
            Self::Bottom => PhysicalSide::Bottom,
            Self::Left => PhysicalSide::Left,
            _ => return None,
        })
    }
}

impl TryTacticAdjustment for AnchorSideKeyword {
    fn try_tactic_adjustment(&mut self, old_side: PhysicalSide, new_side: PhysicalSide) {
        if !old_side.parallel_to(new_side) {
            let Some(s) = self.physical_side() else {
                return;
            };
            *self = Self::from_physical_side(if s == new_side {
                old_side
            } else if s == old_side {
                new_side
            } else if s == new_side.opposite_side() {
                old_side.opposite_side()
            } else {
                debug_assert_eq!(s, old_side.opposite_side());
                new_side.opposite_side()
            });
            return;
        }

        *self = match self {
            Self::Center | Self::Inside | Self::Outside => *self,
            Self::SelfStart => Self::SelfEnd,
            Self::SelfEnd => Self::SelfStart,
            Self::Start => Self::End,
            Self::End => Self::Start,
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

impl AnchorSideKeyword {
    fn valid_for(&self, side: PhysicalSide) -> bool {
        match self {
            Self::Left | Self::Right => matches!(side, PhysicalSide::Left | PhysicalSide::Right),
            Self::Top | Self::Bottom => matches!(side, PhysicalSide::Top | PhysicalSide::Bottom),
            Self::Inside
            | Self::Outside
            | Self::Start
            | Self::End
            | Self::SelfStart
            | Self::SelfEnd
            | Self::Center => true,
        }
    }
}

/// Anchor side for the anchor positioning function.
#[derive(
    Animate,
    Clone,
    ComputeSquaredDistance,
    Copy,
    Debug,
    MallocSizeOf,
    PartialEq,
    Parse,
    SpecifiedValueInfo,
    ToCss,
    ToShmem,
    ToAnimatedValue,
    ToAnimatedZero,
    ToComputedValue,
    ToResolvedValue,
    Serialize,
    Deserialize,
)]
#[repr(C)]
pub enum GenericAnchorSide<P> {
    /// A keyword value for the anchor side.
    Keyword(AnchorSideKeyword),
    /// Percentage value between the `start` and `end` sides.
    Percentage(P),
}

impl<P> GenericAnchorSide<P> {
    /// Is this anchor side valid for a given side?
    pub fn valid_for(&self, side: PhysicalSide) -> bool {
        match self {
            Self::Keyword(k) => k.valid_for(side),
            Self::Percentage(_) => true,
        }
    }
}
