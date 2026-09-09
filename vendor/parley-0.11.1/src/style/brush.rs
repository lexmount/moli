// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

/// Trait for types that represent the color of glyphs or decorations.
pub trait Brush: Clone + PartialEq + Default + core::fmt::Debug {}

impl<T: Clone + PartialEq + Default + core::fmt::Debug> Brush for T {}
