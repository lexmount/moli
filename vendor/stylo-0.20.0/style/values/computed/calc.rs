/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Computed-value calc() leaf types.

use super::{Angle, Length, Number, Percentage, Resolution, Time};
use crate::derives::*;
use crate::values::generics::calc::{
    self, CalcUnits, PositivePercentageBasis, SimplificationResult,
};
use crate::Zero;
use debug_unreachable::debug_unreachable;
use serde::{Deserialize, Serialize};

/// The computed leaf of a calc() expression.
#[derive(
    Clone,
    Debug,
    Deserialize,
    MallocSizeOf,
    PartialEq,
    Serialize,
    ToAnimatedZero,
    ToCss,
    ToResolvedValue,
    ToTyped,
)]
#[allow(missing_docs)]
#[repr(u8)]
pub enum ComputedLeaf {
    Length(Length),
    Percentage(Percentage),
    Number(Number),
    Angle(Angle),
    Time(Time),
    Resolution(Resolution),
}

impl ComputedLeaf {
    pub(super) fn is_zero_length(&self) -> bool {
        match *self {
            Self::Length(ref l) => l.is_zero(),
            Self::Percentage(..)
            | Self::Number(..)
            | Self::Angle(..)
            | Self::Time(..)
            | Self::Resolution(..) => false,
        }
    }
}

impl calc::CalcNodeLeaf for ComputedLeaf {
    fn unit(&self) -> CalcUnits {
        match self {
            Self::Length(_) => CalcUnits::LENGTH,
            Self::Percentage(_) => CalcUnits::PERCENTAGE,
            Self::Number(_) => CalcUnits::empty(),
            Self::Angle(_) => CalcUnits::ANGLE,
            Self::Time(_) => CalcUnits::TIME,
            Self::Resolution(_) => CalcUnits::RESOLUTION,
        }
    }

    fn unitless_value(&self) -> Option<f32> {
        Some(match *self {
            Self::Length(ref l) => l.px(),
            Self::Percentage(ref p) => p.0,
            Self::Number(n) => n,
            Self::Angle(ref a) => a.degrees(),
            Self::Time(ref t) => t.seconds(),
            Self::Resolution(ref r) => r.dppx(),
        })
    }

    fn new_number(value: f32) -> Self {
        Self::Number(value)
    }

    fn as_number(&self) -> Option<f32> {
        match *self {
            Self::Length(_)
            | Self::Percentage(_)
            | Self::Angle(_)
            | Self::Time(_)
            | Self::Resolution(_) => None,
            Self::Number(value) => Some(value),
        }
    }

    fn as_angle_radians(&self) -> Option<f32> {
        match *self {
            Self::Angle(a) => Some(a.radians()),
            _ => None,
        }
    }

    fn new_angle_from_radians(radians: f32) -> Self {
        Self::Angle(Angle::from_radians(radians))
    }

    fn compare(&self, other: &Self, basis: PositivePercentageBasis) -> Option<std::cmp::Ordering> {
        use self::ComputedLeaf::*;
        if std::mem::discriminant(self) != std::mem::discriminant(other) {
            return None;
        }

        if matches!(self, Percentage(..)) && matches!(basis, PositivePercentageBasis::Unknown) {
            return None;
        }

        let Ok(self_negative) = self.is_negative() else {
            return None;
        };
        let Ok(other_negative) = other.is_negative() else {
            return None;
        };
        if self_negative != other_negative {
            return Some(if self_negative {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            });
        }

        match (self, other) {
            (&Length(ref one), &Length(ref other)) => one.partial_cmp(other),
            (&Percentage(ref one), &Percentage(ref other)) => one.partial_cmp(other),
            (&Number(ref one), &Number(ref other)) => one.partial_cmp(other),
            (&Angle(ref one), &Angle(ref other)) => one.partial_cmp(other),
            (&Time(ref one), &Time(ref other)) => one.partial_cmp(other),
            (&Resolution(ref one), &Resolution(ref other)) => one.partial_cmp(other),
            _ => unsafe {
                match *self {
                    Length(..) | Percentage(..) | Number(..) | Angle(..) | Time(..)
                    | Resolution(..) => {},
                }
                debug_unreachable!("Forgot to handle unit in compare()")
            },
        }
    }

    fn try_sum_in_place(&mut self, other: &Self) -> Result<(), ()> {
        use self::ComputedLeaf::*;

        // 0px plus anything else is equal to the right hand side.
        if self.is_zero_length() {
            *self = other.clone();
            return Ok(());
        }

        if other.is_zero_length() {
            return Ok(());
        }

        if std::mem::discriminant(self) != std::mem::discriminant(other) {
            return Err(());
        }

        match (self, other) {
            (&mut Length(ref mut one), &Length(ref other)) => {
                *one += *other;
            },
            (&mut Percentage(ref mut one), &Percentage(ref other)) => {
                one.0 += other.0;
            },
            (&mut Number(ref mut one), &Number(ref other)) => {
                *one += *other;
            },
            (&mut Angle(ref mut one), &Angle(ref other)) => {
                *one += *other;
            },
            (&mut Time(ref mut one), &Time(ref other)) => {
                *one += *other;
            },
            (&mut Resolution(ref mut one), &Resolution(ref other)) => {
                *one += *other;
            },
            _ => unsafe {
                match *other {
                    Length(..) | Percentage(..) | Number(..) | Angle(..) | Time(..)
                    | Resolution(..) => {},
                }
                debug_unreachable!("Forgot to handle unit in try_sum_in_place()")
            },
        }

        Ok(())
    }

    fn try_product_in_place(&mut self, other: &mut Self) -> bool {
        if let Self::Number(ref mut left) = *self {
            if let Self::Number(ref right) = *other {
                // Both sides are numbers, so we can just modify the left side.
                *left *= *right;
                true
            } else {
                // The right side is not a number, so the result should be in the units of the right
                // side.
                if other.map(|v| v * *left).is_ok() {
                    std::mem::swap(self, other);
                    true
                } else {
                    false
                }
            }
        } else if let Self::Number(ref right) = *other {
            // The left side is not a number, but the right side is, so the result is the left
            // side unit.
            self.map(|v| v * *right).is_ok()
        } else {
            // Neither side is a number, so a product is not possible.
            false
        }
    }

    fn try_op<O>(&self, other: &Self, op: O) -> Result<Self, ()>
    where
        O: Fn(f32, f32) -> f32,
    {
        use self::ComputedLeaf::*;
        if std::mem::discriminant(self) != std::mem::discriminant(other) {
            return Err(());
        }
        Ok(match (self, other) {
            (&Length(ref one), &Length(ref other)) => {
                Length(super::Length::new(op(one.px(), other.px())))
            },
            (&Percentage(one), &Percentage(other)) => {
                Self::Percentage(super::Percentage(op(one.0, other.0)))
            },
            (&Number(one), &Number(other)) => Self::Number(op(one, other)),
            (&Angle(ref one), &Angle(ref other)) => Self::Angle(super::Angle::from_degrees(op(
                one.degrees(),
                other.degrees(),
            ))),
            (&Time(ref one), &Time(ref other)) => Self::Time(super::Time::from_seconds(op(
                one.seconds(),
                other.seconds(),
            ))),
            (&Resolution(ref one), &Resolution(ref other)) => {
                Self::Resolution(super::Resolution::from_dppx(op(one.dppx(), other.dppx())))
            },
            _ => unsafe {
                match *self {
                    Length(..) | Percentage(..) | Number(..) | Angle(..) | Time(..)
                    | Resolution(..) => {},
                }
                debug_unreachable!("Forgot to handle unit in try_op()")
            },
        })
    }

    fn map(&mut self, mut op: impl FnMut(f32) -> f32) -> Result<(), ()> {
        Ok(match self {
            Self::Length(value) => {
                *value = Length::new(op(value.px()));
            },
            Self::Percentage(value) => {
                *value = Percentage(op(value.0));
            },
            Self::Number(value) => {
                *value = op(*value);
            },
            Self::Angle(value) => {
                *value = Angle::from_degrees(op(value.degrees()));
            },
            Self::Time(value) => {
                *value = Time::from_seconds(op(value.seconds()));
            },
            Self::Resolution(value) => {
                *value = Resolution::from_dppx(op(value.dppx()));
            },
        })
    }

    fn simplify(&mut self) -> SimplificationResult {
        return SimplificationResult::Unchanged;
    }

    fn sort_key(&self) -> calc::SortKey {
        match *self {
            Self::Length(..) => calc::SortKey::Px,
            Self::Percentage(..) => calc::SortKey::Percentage,
            Self::Number(..) => calc::SortKey::Number,
            Self::Angle(..) => calc::SortKey::Deg,
            Self::Time(..) => calc::SortKey::S,
            Self::Resolution(..) => calc::SortKey::Dppx,
        }
    }
}
