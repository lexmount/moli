/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Generic types for CSS Easing Functions.
//! https://drafts.csswg.org/css-easing/#timing-functions

use crate::derives::*;

/// A generic easing function.
#[derive(
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToCss,
    ToShmem,
    ToTyped,
    Serialize,
    Deserialize,
)]
#[value_info(ty = "TIMING_FUNCTION")]
#[repr(u8, C)]
pub enum TimingFunction<Integer, Number, LinearStops> {
    /// `linear | ease | ease-in | ease-out | ease-in-out`
    Keyword(TimingKeyword),
    /// `cubic-bezier(<number>, <number>, <number>, <number>)`
    #[allow(missing_docs)]
    #[css(comma, function)]
    #[typed(skip)]
    CubicBezier {
        x1: Number,
        y1: Number,
        x2: Number,
        y2: Number,
    },
    /// `step-start | step-end | steps(<integer>, [ <step-position> ]?)`
    /// `<step-position> = jump-start | jump-end | jump-none | jump-both | start | end`
    #[css(comma, function)]
    #[typed(skip)]
    #[value_info(other_values = "step-start,step-end")]
    Steps(Integer, #[css(skip_if = "is_end")] StepPosition),
    /// linear([<linear-stop>]#)
    /// <linear-stop> = <output> && <linear-stop-length>?
    /// <linear-stop-length> = <percentage>{1, 2}
    #[css(function = "linear")]
    #[typed(skip)]
    LinearFunction(LinearStops),
}

#[allow(missing_docs)]
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    MallocSizeOf,
    Parse,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
    ToTyped,
    Serialize,
    Deserialize,
)]
#[repr(u8)]
pub enum TimingKeyword {
    Linear,
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
}

/// Before flag, defined as per https://drafts.csswg.org/css-easing/#before-flag
/// This flag is never user-specified.
#[allow(missing_docs)]
#[derive(PartialEq)]
#[repr(u8)]
pub enum BeforeFlag {
    Unset,
    Set,
}

#[allow(missing_docs)]
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    MallocSizeOf,
    Parse,
    PartialEq,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
    Serialize,
    Deserialize,
)]
#[repr(u8)]
pub enum StepPosition {
    JumpStart,
    JumpEnd,
    JumpNone,
    JumpBoth,
    Start,
    End,
}

#[inline]
fn is_end(position: &StepPosition) -> bool {
    *position == StepPosition::JumpEnd || *position == StepPosition::End
}

impl<Integer, Number, LinearStops> TimingFunction<Integer, Number, LinearStops> {
    /// `ease`
    #[inline]
    pub fn ease() -> Self {
        TimingFunction::Keyword(TimingKeyword::Ease)
    }

    /// Returns true if it is `ease`.
    #[inline]
    pub fn is_ease(&self) -> bool {
        matches!(*self, TimingFunction::Keyword(TimingKeyword::Ease))
    }
}
