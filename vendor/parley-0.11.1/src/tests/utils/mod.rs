// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use peniko::Color;

pub(crate) mod asserts;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ColorBrush {
    pub(crate) color: Color,
}

impl ColorBrush {
    pub(crate) fn new(color: Color) -> Self {
        let rgba8 = color.to_rgba8();
        Self {
            color: Color::from_rgba8(rgba8.r, rgba8.g, rgba8.b, rgba8.a),
        }
    }
}

impl Default for ColorBrush {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
        }
    }
}
