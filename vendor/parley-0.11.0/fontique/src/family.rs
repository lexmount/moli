// Copyright 2024 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Model for font families.

use super::{FontStyle, FontWeight, FontWidth, family_name::FamilyName, font::FontInfo};
use crate::AtomicCounter;
use alloc::sync::Arc;
use core::sync::atomic::Ordering;
use smallvec::SmallVec;

/// Unique identifier for a font family.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(transparent)]
pub struct FamilyId(u64);

impl FamilyId {
    /// Creates a new unique identifier.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        /// Make sure this is larger than the largest generic family id.
        static ID_COUNTER: AtomicCounter = AtomicCounter::new(64);
        #[allow(clippy::useless_conversion, reason = "Not useless on 32-bit platforms")]
        Self(ID_COUNTER.fetch_add(1, Ordering::Relaxed).into())
    }

    /// Returns the underlying integer value.
    pub fn to_u64(self) -> u64 {
        self.0
    }
}

/// Named set of fonts that are instances of a core design.
#[derive(Clone, Debug)]
pub struct FamilyInfo(Arc<FamilyInner>);

impl FamilyInfo {
    /// Creates a new font family object with the given name and collection of
    /// fonts.
    pub fn new(name: FamilyName, fonts: impl IntoIterator<Item = FontInfo>) -> Self {
        let fonts: SmallVec<[FontInfo; 4]> = fonts.into_iter().collect();
        let default_font = super::matching::match_font(
            &fonts[..],
            FontWidth::default(),
            FontStyle::default(),
            FontWeight::default(),
            false,
        )
        .unwrap_or(0);
        Self(Arc::new(FamilyInner {
            name,
            default_font,
            fonts,
        }))
    }

    /// Returns the unique identifier for the family.
    pub fn id(&self) -> FamilyId {
        self.0.name.id()
    }

    /// Returns the name of the family.
    pub fn name(&self) -> &str {
        self.0.name.name()
    }

    /// Returns the collection of fonts that are members of the family.
    pub fn fonts(&self) -> &[FontInfo] {
        &self.0.fonts
    }

    /// Returns index of the default font of the family.
    pub fn default_font_index(&self) -> usize {
        self.0.default_font
    }

    /// Returns the default font of the family.
    pub fn default_font(&self) -> Option<&FontInfo> {
        self.0.fonts.get(self.0.default_font)
    }

    /// Returns the index of the best font from the family for the given attributes.
    pub fn match_index(
        &self,
        width: FontWidth,
        style: FontStyle,
        weight: FontWeight,
        synthesize_style: bool,
    ) -> Option<usize> {
        super::matching::match_font(self.fonts(), width, style, weight, synthesize_style)
    }

    /// Selects the best font from the family for the given attributes.
    pub fn match_font(
        &self,
        width: FontWidth,
        style: FontStyle,
        weight: FontWeight,
        synthesize_style: bool,
    ) -> Option<&FontInfo> {
        self.fonts()
            .get(self.match_index(width, style, weight, synthesize_style)?)
    }
}

#[derive(Clone, Debug)]
struct FamilyInner {
    pub(crate) name: FamilyName,
    default_font: usize,
    fonts: SmallVec<[FontInfo; 4]>,
}
