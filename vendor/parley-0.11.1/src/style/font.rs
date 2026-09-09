// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::borrow::Cow;

pub use crate::setting::{FontFeature, FontVariation};
pub use fontique::{FontStyle, FontWeight, FontWidth, GenericFamily};
pub use parlance::{FontFamily, FontFamilyName};

/// Font variation settings that can be supplied as a raw source string or a parsed slice.
#[derive(Clone, PartialEq, Debug)]
pub enum FontVariations<'a> {
    /// Setting source in CSS format.
    Source(Cow<'a, str>),
    /// List of settings.
    List(Cow<'a, [FontVariation]>),
}

impl<'a> FontVariations<'a> {
    /// Creates an empty list of font variations.
    pub const fn empty() -> Self {
        Self::List(Cow::Borrowed(&[]))
    }
}

impl<'a> From<&'a str> for FontVariations<'a> {
    fn from(value: &'a str) -> Self {
        Self::Source(Cow::Borrowed(value))
    }
}

impl<'a> From<&'a [FontVariation]> for FontVariations<'a> {
    fn from(value: &'a [FontVariation]) -> Self {
        Self::List(Cow::Borrowed(value))
    }
}

impl<'a, const N: usize> From<&'a [FontVariation; N]> for FontVariations<'a> {
    fn from(value: &'a [FontVariation; N]) -> Self {
        Self::List(Cow::Borrowed(&value[..]))
    }
}

/// Font feature settings that can be supplied as a raw source string or a parsed slice.
#[derive(Clone, PartialEq, Debug)]
pub enum FontFeatures<'a> {
    /// Setting source in CSS format.
    Source(Cow<'a, str>),
    /// List of settings.
    List(Cow<'a, [FontFeature]>),
}

impl<'a> FontFeatures<'a> {
    /// Creates an empty list of font features.
    pub const fn empty() -> Self {
        Self::List(Cow::Borrowed(&[]))
    }
}

impl<'a> From<&'a str> for FontFeatures<'a> {
    fn from(value: &'a str) -> Self {
        Self::Source(Cow::Borrowed(value))
    }
}

impl<'a> From<&'a [FontFeature]> for FontFeatures<'a> {
    fn from(value: &'a [FontFeature]) -> Self {
        Self::List(Cow::Borrowed(value))
    }
}

impl<'a, const N: usize> From<&'a [FontFeature; N]> for FontFeatures<'a> {
    fn from(value: &'a [FontFeature; N]) -> Self {
        Self::List(Cow::Borrowed(&value[..]))
    }
}
