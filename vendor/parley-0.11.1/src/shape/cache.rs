// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::FontVariation;
use alloc::boxed::Box;
use hashbrown::Equivalent;

#[derive(PartialEq, Copy, Clone)]
pub(crate) struct ShapeDataKey {
    /// The font collection's blob ID.
    font_blob_id: u64,
    /// The font's index in the font collection.
    font_index: u32,
}

impl ShapeDataKey {
    pub(crate) const fn new(font_blob_id: u64, font_index: u32) -> Self {
        Self {
            font_blob_id,
            font_index,
        }
    }
}

impl Equivalent<Self> for ShapeDataKey {
    #[inline(always)]
    fn equivalent(&self, key: &Self) -> bool {
        self == key
    }
}

impl From<&Self> for ShapeDataKey {
    #[inline(always)]
    fn from(key: &Self) -> Self {
        *key
    }
}

pub(crate) struct ShapeInstanceId {
    /// The font collection's blob ID.
    font_blob_id: u64,
    /// The font's index in the font collection.
    font_index: u32,
    synthesis: fontique::Synthesis,
    variations: Option<Box<[FontVariation]>>,
}

pub(crate) struct ShapeInstanceKey<'a> {
    /// The font collection's blob ID.
    font_blob_id: u64,
    /// The font's index in the font collection.
    font_index: u32,
    synthesis: &'a fontique::Synthesis,
    variations: Option<&'a [FontVariation]>,
}

impl<'a> ShapeInstanceKey<'a> {
    pub(crate) const fn new(
        font_blob_id: u64,
        font_index: u32,
        synthesis: &'a fontique::Synthesis,
        variations: Option<&'a [FontVariation]>,
    ) -> Self {
        Self {
            font_blob_id,
            font_index,
            synthesis,
            variations,
        }
    }
}

impl<'a> Equivalent<ShapeInstanceId> for ShapeInstanceKey<'a> {
    #[inline(always)]
    fn equivalent(&self, key: &ShapeInstanceId) -> bool {
        self.font_blob_id == key.font_blob_id
            && self.font_index == key.font_index
            && *self.synthesis == key.synthesis
            && self.variations == key.variations.as_deref()
    }
}

impl<'a> From<ShapeInstanceKey<'a>> for ShapeInstanceId {
    #[inline(always)]
    fn from(key: ShapeInstanceKey<'a>) -> Self {
        Self {
            font_blob_id: key.font_blob_id,
            font_index: key.font_index,
            synthesis: *key.synthesis,
            variations: key.variations.map(|v| v.to_vec().into()),
        }
    }
}

pub(crate) struct ShapePlanId {
    /// The font collection's blob ID.
    font_blob_id: u64,
    /// The font's index in the font collection.
    font_index: u32,
    synthesis: fontique::Synthesis,
    direction: harfrust::Direction,
    script: harfrust::Script,
    language: Option<harfrust::Language>,
    features: Box<[harfrust::Feature]>,
    variations: Option<Box<[FontVariation]>>,
}

pub(crate) struct ShapePlanKey<'a> {
    /// The font collection's blob ID.
    font_blob_id: u64,
    /// The font's index in the font collection.
    font_index: u32,
    synthesis: &'a fontique::Synthesis,
    direction: harfrust::Direction,
    script: harfrust::Script,
    language: Option<harfrust::Language>,
    features: &'a [harfrust::Feature],
    variations: Option<&'a [FontVariation]>,
}

impl<'a> ShapePlanKey<'a> {
    pub(crate) const fn new(
        font_blob_id: u64,
        font_index: u32,
        synthesis: &'a fontique::Synthesis,
        direction: harfrust::Direction,
        script: harfrust::Script,
        language: Option<harfrust::Language>,
        features: &'a [harfrust::Feature],
        variations: Option<&'a [FontVariation]>,
    ) -> Self {
        Self {
            font_blob_id,
            font_index,
            synthesis,
            direction,
            script,
            language,
            features,
            variations,
        }
    }
}

impl<'a> Equivalent<ShapePlanId> for ShapePlanKey<'a> {
    #[inline(always)]
    fn equivalent(&self, key: &ShapePlanId) -> bool {
        self.font_blob_id == key.font_blob_id
            && self.font_index == key.font_index
            && *self.synthesis == key.synthesis
            && self.direction == key.direction
            && self.script == key.script
            && self.language == key.language
            && self.features.len() == key.features.len()
            && self.variations == key.variations.as_deref()
            && self
                .features
                .iter()
                .zip(key.features.iter())
                .all(|(a, b)| a == b)
    }
}

impl<'a> From<ShapePlanKey<'a>> for ShapePlanId {
    #[inline(always)]
    fn from(key: ShapePlanKey<'a>) -> Self {
        Self {
            font_blob_id: key.font_blob_id,
            font_index: key.font_index,
            synthesis: *key.synthesis,
            direction: key.direction,
            script: key.script,
            language: key.language,
            features: key.features.to_vec().into(),
            variations: key.variations.map(|v| v.to_vec().into()),
        }
    }
}
