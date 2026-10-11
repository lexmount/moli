// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Text shaping utilities.

pub(crate) mod atom;
mod cache;
mod cluster;
mod data;
pub(crate) mod shaped_text;
pub(crate) mod shaper;

pub use cluster::{Char, CharCluster, Coverage, SourceRange, Whitespace};
pub use data::{Character, CharacterFlags, ShapedCluster};

pub(crate) use data::ShapedClusterFlags;
