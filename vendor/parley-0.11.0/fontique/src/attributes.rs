// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Properties for specifying font matching attributes.

use core::fmt;

use parlance::{FontStyle, FontWeight, FontWidth};

/// Primary attributes for font matching: [`FontWidth`], [`FontStyle`] and [`FontWeight`].
///
/// These are used to [configure] a [`Query`].
///
/// [configure]: crate::Query::set_attributes
/// [`Query`]: crate::Query
#[derive(Copy, Clone, PartialEq, Default, Debug)]
pub struct Attributes {
    pub width: FontWidth,
    pub style: FontStyle,
    pub weight: FontWeight,
}

impl Attributes {
    /// Creates new attributes from the given width, style and weight.
    pub fn new(width: FontWidth, style: FontStyle, weight: FontWeight) -> Self {
        Self {
            width,
            style,
            weight,
        }
    }
}

impl fmt::Display for Attributes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "width: {}, style: {}, weight: {}",
            self.width, self.style, self.weight
        )
    }
}
