/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Computed types for CSS borders.

use crate::typed_om::{ToTyped, TypedValue};
use crate::values::computed::length::NonNegativeLengthOrNumber;
use crate::values::generics::rect::Rect;
use thin_vec::ThinVec;

/// A specified rectangle made of four `<length-or-number>` values.
pub type NonNegativeLengthOrNumberRect = Rect<NonNegativeLengthOrNumber>;

impl ToTyped for NonNegativeLengthOrNumberRect {
    fn to_typed(&self, _dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        return Err(());
    }
}
