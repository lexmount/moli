/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Specified types for CSS borders.

use crate::typed_om::{ToTyped, TypedValue};
use crate::values::generics::rect::Rect;
use crate::values::specified::length::NonNegativeLengthOrNumber;
use thin_vec::ThinVec;

/// A specified rectangle made of four `<length-or-number>` values.
pub type NonNegativeLengthOrNumberRect = Rect<NonNegativeLengthOrNumber>;

impl ToTyped for NonNegativeLengthOrNumberRect {
    // Note: The specification does not currently define how border image
    // outset should be reified into Typed OM. The current behavior follows
    // existing WPT coverage (border-image-outset.html). Syncing spec with
    // UA/WPT behavior tracked in
    // https://github.com/w3c/csswg-drafts/issues/13907
    fn to_typed(&self, dest: &mut ThinVec<TypedValue>) -> Result<(), ()> {
        if !self.all_sides_equal() {
            return Err(());
        }

        self.0.to_typed(dest)
    }
}
