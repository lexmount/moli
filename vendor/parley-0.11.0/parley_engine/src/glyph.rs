// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

/// Glyph with an offset and advance.
#[derive(Copy, Clone, Default, Debug, PartialEq)]
#[expect(missing_docs, reason = "Deferred")]
pub struct Glyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub advance: f32,
}
