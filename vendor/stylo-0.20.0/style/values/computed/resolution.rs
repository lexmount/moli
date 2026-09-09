/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Resolution values:
//!
//! https://drafts.csswg.org/css-values/#resolution

use crate::derives::*;
use crate::typed_om::{NumericType, NumericValue, ToTyped, TypedValue, UnitValue};
use crate::values::CSSFloat;
use std::{
    fmt::{self, Write},
    ops::AddAssign,
};
use style_traits::{CssString, CssWriter, ToCss};
use thin_vec::ThinVec;

/// A computed `<resolution>`.
#[repr(C)]
#[derive(
    Animate,
    Copy,
    Clone,
    Debug,
    Deserialize,
    MallocSizeOf,
    PartialEq,
    PartialOrd,
    Serialize,
    ToAnimatedZero,
    ToResolvedValue,
    ToShmem,
)]
pub struct Resolution(CSSFloat);

impl Resolution {
    /// Returns this resolution value as dppx.
    #[inline]
    pub fn dppx(&self) -> CSSFloat {
        self.0
    }

    /// Return a computed `resolution` value from a dppx float value.
    #[inline]
    pub fn from_dppx(dppx: CSSFloat) -> Self {
        Resolution(dppx)
    }
}

impl ToCss for Resolution {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: fmt::Write,
    {
        self.dppx().to_css(dest)?;
        dest.write_str("dppx")
    }
}

impl ToTyped for Resolution {
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        dest.push(TypedValue::Numeric(NumericValue::Unit(UnitValue {
            numeric_type: NumericType::resolution(),
            value: self.dppx(),
            unit: CssString::from("dppx"),
        })));
        Ok(())
    }
}

impl AddAssign for Resolution {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0
    }
}
