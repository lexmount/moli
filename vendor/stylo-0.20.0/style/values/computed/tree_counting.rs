/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Computed types for CSS tree-counting functions.
//! https://drafts.csswg.org/css-values-5/#tree-counting

/// Holds the resolved sibling-index() and sibling-count() values for an element.
#[derive(Clone, Copy, Debug)]
pub struct TreeCountingResult {
    /// The 1-based index of the element among its siblings.
    pub sibling_index: u32,
    /// The total number of siblings of the element, including itself.
    pub sibling_count: u32,
}

impl TreeCountingResult {
    /// Creates a new TreeCountingResult with the given index and count.
    pub fn new(sibling_index: u32, sibling_count: u32) -> Self {
        TreeCountingResult {
            sibling_index,
            sibling_count,
        }
    }

    /// Creates a default TreeCountingResult.
    pub fn default() -> Self {
        TreeCountingResult::new(1, 1)
    }
}
