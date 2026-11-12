// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::utils::{
    ColorBrush,
    fonts::{FONT_FAMILY_LIST, create_font_context},
};
use crate::{Alignment, AlignmentOptions, FontFamily, IndentOptions, Layout, LayoutContext};

#[test]
fn clear_resets_to_new() {
    let mut fcx = create_font_context();
    let mut lcx: LayoutContext<ColorBrush> = LayoutContext::new();

    static TEXT: &str = "Some text that will wrap across several lines.";

    let mut builder = lcx.ranged_builder(&mut fcx, TEXT, 1., false);
    builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
    let mut layout = builder.build(TEXT);
    layout.set_text_indent(10., IndentOptions::default());
    layout.break_all_lines(Some(50.));
    layout.align(Alignment::Center, AlignmentOptions::default());
    assert!(layout.lines().len() > 1);
    assert!(layout.width() > 0.);
    assert!(layout.height() > 0.);

    layout.clear();

    assert_eq!(layout.data, Layout::new().data);
}
