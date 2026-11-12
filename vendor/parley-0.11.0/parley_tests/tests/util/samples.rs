// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

/// Simple Latin text - baseline for most tests
pub(crate) const LATIN: &str = "The quick brown fox jumps.";

/// Latin with multiple lines for testing line-related properties
pub(crate) const LATIN_MULTILINE: &str = "Line one.\nLine two.\nLine three.";

/// Arabic text for RTL and complex shaping
pub(crate) const ARABIC: &str = "مرحبا بالعالم";

/// Mixed bidirectional text
pub(crate) const MIXED_BIDI: &str = "Hello مرحبا World";

/// Text with common ligatures (fi, fl, ff, ffi, ffl)
pub(crate) const LIGATURES: &str = "fi fl ff ffi ffl office";

/// Text with spaces for word spacing tests
pub(crate) const SPACED: &str = "one two three four five";

pub(crate) const LOREM_IPSUM: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Integer cursus interdum dui, in gravida ligula aliquam in. Vivamus vitae metus pharetra, ultricies metus quis, consectetur augue. Phasellus ac mauris et nisi pretium aliquet sed ac orci. Ut mi ipsum, mollis sed placerat et, bibendum et velit. Nunc vitae ornare leo. Aliquam turpis sem, varius eget neque vel, mattis ultricies ex. Fusce metus mauris, fermentum at porttitor quis, malesuada sed elit. Integer vel eros congue, volutpat nunc in, lobortis ante.";
