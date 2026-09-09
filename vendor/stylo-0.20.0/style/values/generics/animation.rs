/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Generic values for properties related to animations and transitions.

use crate::derives::*;
use crate::typed_om::{KeywordValue, ToTyped, TypedValue};
use crate::values::generics::length::GenericLengthPercentageOrAuto;
use crate::values::specified::animation::{
    ScrollAxis, ScrollFunction, TimelineName, TimelineRangeName,
};
use crate::values::specified::length::EqualsPercentage;
use crate::Zero;
use std::fmt::{self, Write};
use style_traits::{CssString, CssWriter, ToCss};
use thin_vec::ThinVec;

/// The `animation-duration` property.
///
/// https://drafts.csswg.org/css-animations-2/#animation-duration
#[derive(
    Clone, Copy, Debug, MallocSizeOf, PartialEq, SpecifiedValueInfo, ToComputedValue, ToShmem,
)]
#[repr(C, u8)]
pub enum GenericAnimationDuration<T> {
    /// The initial value. However, we serialize this as 0s if the preference is disabled.
    Auto,
    /// The time value, <time [0s,∞]>.
    Time(T),
}

pub use self::GenericAnimationDuration as AnimationDuration;

impl<T> AnimationDuration<T> {
    /// Returns the `auto` value.
    pub fn auto() -> Self {
        Self::Auto
    }

    /// Returns true if it is `auto`.
    pub fn is_auto(&self) -> bool {
        matches!(*self, Self::Auto)
    }
}

impl<T: Zero> Zero for AnimationDuration<T> {
    fn zero() -> Self {
        Self::Time(T::zero())
    }

    fn is_zero(&self) -> bool {
        match *self {
            Self::Time(ref t) => t.is_zero(),
            _ => false,
        }
    }
}

impl<T: ToCss + Zero> ToCss for AnimationDuration<T> {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        match *self {
            Self::Auto => {
                if static_prefs::pref!("layout.css.scroll-driven-animations.enabled") {
                    dest.write_str("auto")
                } else {
                    Self::Time(T::zero()).to_css(dest)
                }
            },
            Self::Time(ref t) => t.to_css(dest),
        }
    }
}

// TODO: Switch to ToTyped derive once the pref goes away.
impl<T: ToTyped + Zero> ToTyped for AnimationDuration<T> {
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        match *self {
            Self::Auto => {
                if static_prefs::pref!("layout.css.scroll-driven-animations.enabled") {
                    dest.push(TypedValue::Keyword(KeywordValue(CssString::from("auto"))));
                    Ok(())
                } else {
                    Self::Time(T::zero()).to_typed(dest)
                }
            },
            Self::Time(ref t) => t.to_typed(dest),
        }
    }
}

/// The view() notation.
/// https://drafts.csswg.org/scroll-animations-1/#view-notation
#[derive(
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
)]
#[css(function = "view")]
#[repr(C)]
pub struct GenericViewFunction<LengthPercent> {
    /// The axis of scrolling that drives the progress of the timeline.
    #[css(skip_if = "ScrollAxis::is_default")]
    pub axis: ScrollAxis,
    /// An adjustment of the view progress visibility range.
    #[css(skip_if = "GenericViewTimelineInset::is_auto")]
    #[css(field_bound)]
    pub inset: GenericViewTimelineInset<LengthPercent>,
}

pub use self::GenericViewFunction as ViewFunction;

/// A value for the <single-animation-timeline>.
///
/// https://drafts.csswg.org/css-animations-2/#typedef-single-animation-timeline
#[derive(
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
    ToTyped,
)]
#[repr(C, u8)]
#[typed(todo_derive_fields)]
pub enum GenericAnimationTimeline<LengthPercent> {
    /// Use default timeline. The animation’s timeline is a DocumentTimeline.
    Auto,
    /// The scroll-timeline name or view-timeline-name.
    /// This also includes `none` value by using an empty atom.
    /// https://drafts.csswg.org/scroll-animations-1/#scroll-timeline-name
    /// https://drafts.csswg.org/scroll-animations-1/#view-timeline-name
    Timeline(TimelineName),
    /// The scroll() notation.
    /// https://drafts.csswg.org/scroll-animations-1/#scroll-notation
    Scroll(ScrollFunction),
    /// The view() notation.
    /// https://drafts.csswg.org/scroll-animations-1/#view-notation
    View(#[css(field_bound)] GenericViewFunction<LengthPercent>),
}

pub use self::GenericAnimationTimeline as AnimationTimeline;

impl<LengthPercent> AnimationTimeline<LengthPercent> {
    /// Returns the `auto` value.
    pub fn auto() -> Self {
        Self::Auto
    }

    /// Returns true if it is auto (i.e. the default value).
    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }
}

/// A generic value for the `[ [ auto | <length-percentage> ]{1,2} ]`.
///
/// https://drafts.csswg.org/scroll-animations-1/#view-timeline-inset
#[derive(
    Clone,
    Copy,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C)]
pub struct GenericViewTimelineInset<LengthPercent> {
    /// The start inset in the relevant axis.
    pub start: GenericLengthPercentageOrAuto<LengthPercent>,
    /// The end inset.
    pub end: GenericLengthPercentageOrAuto<LengthPercent>,
}

pub use self::GenericViewTimelineInset as ViewTimelineInset;

impl<LengthPercent> ViewTimelineInset<LengthPercent> {
    /// Returns true if it is auto.
    #[inline]
    fn is_auto(&self) -> bool {
        self.start.is_auto() && self.end.is_auto()
    }
}

impl<LengthPercent> ToCss for ViewTimelineInset<LengthPercent>
where
    LengthPercent: PartialEq + ToCss,
{
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        self.start.to_css(dest)?;
        if self.end != self.start {
            dest.write_char(' ')?;
            self.end.to_css(dest)?;
        }
        Ok(())
    }
}

impl<LengthPercent> ToTyped for ViewTimelineInset<LengthPercent> where
    LengthPercent: PartialEq + ToTyped
{
}

impl<LengthPercent> Default for ViewTimelineInset<LengthPercent> {
    fn default() -> Self {
        Self {
            start: GenericLengthPercentageOrAuto::auto(),
            end: GenericLengthPercentageOrAuto::auto(),
        }
    }
}

/// A value for animation-range-start or animation-range-end.
///
/// https://drafts.csswg.org/scroll-animations-1/#animation-range-start
/// https://drafts.csswg.org/scroll-animations-1/#animation-range-end
#[derive(
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C)]
pub struct GenericAnimationRangeValue<LengthPercent> {
    /// The specific timeline range. If it is None, the animation range only has length-percentage
    /// component.
    pub name: TimelineRangeName,
    /// Used to measure the specific point from the start of the named timeline.
    pub lp: LengthPercent,
}

pub use self::GenericAnimationRangeValue as AnimationRangeValue;

impl<LengthPercent> AnimationRangeValue<LengthPercent> {
    /// Returns the "normal" value
    #[inline]
    pub fn normal(lp: LengthPercent) -> Self {
        Self::new(TimelineRangeName::Normal, lp)
    }

    /// Returns Self as a LengthPercentage.
    #[inline]
    pub fn length_percentage(lp: LengthPercent) -> Self {
        Self::new(TimelineRangeName::None, lp)
    }

    /// Returns Self as a tuple of TimelineRangeName range name and LengthPercentage.
    #[inline]
    pub fn new(name: TimelineRangeName, lp: LengthPercent) -> Self {
        Self { name, lp }
    }

    /// Returns true if it is "normal".
    #[inline]
    pub fn is_normal(&self) -> bool {
        self.name.is_normal()
    }
}

/// A value for animation-range-start.
///
/// https://drafts.csswg.org/scroll-animations-1/#animation-range-start
#[derive(
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
    ToTyped,
)]
#[repr(transparent)]
#[typed(todo_derive_fields)]
pub struct GenericAnimationRangeStart<LengthPercent>(pub GenericAnimationRangeValue<LengthPercent>);

pub use self::GenericAnimationRangeStart as AnimationRangeStart;

fn to_css_with_default<LengthPercent, W>(
    value: &AnimationRangeValue<LengthPercent>,
    dest: &mut CssWriter<W>,
    default: f32,
) -> fmt::Result
where
    LengthPercent: ToCss + EqualsPercentage,
    W: Write,
{
    if matches!(value.name, TimelineRangeName::Normal) {
        return dest.write_str("normal");
    }
    if matches!(value.name, TimelineRangeName::None) {
        return value.lp.to_css(dest);
    }
    // <timeline-range-name> <length-percentage>?
    value.name.to_css(dest)?;
    if !value.lp.equals_percentage(default) {
        dest.write_char(' ')?;
        value.lp.to_css(dest)?;
    }
    Ok(())
}

impl<LengthPercent: ToCss + EqualsPercentage> ToCss for AnimationRangeStart<LengthPercent> {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        to_css_with_default(&self.0, dest, 0.0)
    }
}

/// A value for animation-range-end.
///
/// https://drafts.csswg.org/scroll-animations-1/#animation-range-end
#[derive(
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
    ToTyped,
)]
#[repr(transparent)]
#[typed(todo_derive_fields)]
pub struct GenericAnimationRangeEnd<LengthPercent>(pub GenericAnimationRangeValue<LengthPercent>);

pub use self::GenericAnimationRangeEnd as AnimationRangeEnd;

impl<LengthPercent: ToCss + EqualsPercentage> ToCss for AnimationRangeEnd<LengthPercent> {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        to_css_with_default(&self.0, dest, 1.0)
    }
}
