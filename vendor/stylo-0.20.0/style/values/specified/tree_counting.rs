/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Specified types for CSS tree-counting functions.
//! https://drafts.csswg.org/css-values-5/#tree-counting

use crate::derives::*;
use crate::values::computed::Context;

/// A CSS tree-counting function: `sibling-index()` or `sibling-count()`.
/// See <https://www.w3.org/TR/css-values-5/#tree-counting>.
#[derive(Clone, Copy, Debug, MallocSizeOf, PartialEq, PartialOrd, ToCss, ToShmem, ToTyped)]
#[repr(u8)]
pub enum TreeCountingFunction {
    /// `sibling-count()`: the total number of siblings of the element, including itself.
    #[css(function)]
    SiblingCount,
    /// `sibling-index()`: the 1-based index of the element among its siblings.
    #[css(function)]
    SiblingIndex,
}

impl TreeCountingFunction {
    /// Computes the value of this tree-counting function.
    pub fn to_computed_value(&self, context: &Context) -> u32 {
        match self {
            TreeCountingFunction::SiblingCount => context.query_sibling_count(),
            TreeCountingFunction::SiblingIndex => context.query_sibling_index(),
        }
    }
}
