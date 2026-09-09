// SPDX-License-Identifier: MIT OR Apache-2.0

use super::{
    InlineDirection, LayoutDisplay, LayoutOverflowMode, PaintBorderColors, PaintBorderStyle,
    PaintBorderStyles, PaintColor, PreferredAspectRatio, ResolvedLayoutStyle,
};
use crate::LayoutImageFallback;
use taffy::{BoxSizing, Dimension, Float, LengthPercentage, Rect, Size};

/// Used styles for unavailable HTML images. The decisions follow Blink's
/// HTMLImageFallbackHelper; no author computed style or DOM is mutated.
pub(crate) struct ImageFallbackStyles {
    pub(crate) content: LayoutImageFallback,
    pub(crate) container: Option<ResolvedLayoutStyle>,
    pub(crate) show_icon: bool,
}

impl ImageFallbackStyles {
    pub(crate) fn new(
        host: &mut ResolvedLayoutStyle,
        content: LayoutImageFallback,
        mut container: ResolvedLayoutStyle,
    ) -> Self {
        if content.quirks_mode {
            if host.taffy.size.width.is_auto() {
                host.taffy.size.width = host.taffy.size.height;
            } else if host.taffy.size.height.is_auto() {
                host.taffy.size.height = host.taffy.size.width;
            }
        }
        let size = host.taffy.size;
        let has_dimensions = !size.width.is_auto() && !size.height.is_auto();
        let has_ratio_dimensions = host.preferred_aspect_ratio != PreferredAspectRatio::Auto
            && (!size.width.is_auto() || !size.height.is_auto());
        let replaced = (has_dimensions || has_ratio_dimensions)
            && (content.quirks_mode || !content.has_nonempty_alt_attribute);
        let small = [size.width, size.height]
            .into_iter()
            .any(|axis| axis.into_option().is_some_and(|value| value < 18.0));
        let represents_nothing = match content.alt_text.as_deref() {
            Some(text) => text.is_empty(),
            None => !content.has_source,
        };
        let show_icon = if replaced {
            !small
        } else {
            !represents_nothing
        };
        let container = replaced.then(|| {
            container.force_layout_display(LayoutDisplay::FlowRoot);
            container.taffy.size = size;
            container.taffy.max_size = host.taffy.max_size;
            container.overflow_clips = true;
            container.overflow_x = LayoutOverflowMode::Hidden;
            container.overflow_y = LayoutOverflowMode::Hidden;
            container.taffy.overflow = taffy::Point {
                x: taffy::Overflow::Hidden,
                y: taffy::Overflow::Hidden,
            };
            container.pointer_events = false;
            if show_icon {
                let zoom = host.effective_zoom();
                let border = LengthPercentage::length(zoom.trunc());
                let padding = LengthPercentage::length(zoom);
                container.taffy.border = Rect {
                    left: border,
                    right: border,
                    top: border,
                    bottom: border,
                };
                container.taffy.padding = Rect {
                    left: padding,
                    right: padding,
                    top: padding,
                    bottom: padding,
                };
                container.border_colors = PaintBorderColors::all(PaintColor::new(
                    192.0 / 255.0,
                    192.0 / 255.0,
                    192.0 / 255.0,
                    1.0,
                ));
                container.border_styles = PaintBorderStyles::all(PaintBorderStyle::Solid);
                container.taffy.box_sizing = BoxSizing::BorderBox;
            }
            container
        });
        if !replaced && host.display == LayoutDisplay::Inline {
            host.taffy.size = Size {
                width: Dimension::auto(),
                height: Dimension::auto(),
            };
            host.preferred_aspect_ratio = PreferredAspectRatio::Auto;
            host.taffy.aspect_ratio = None;
        }
        // Blink creates a block-flow LayoutObject for fallback content even
        // with an author flex/grid display. Inline hosts remain atomic inline.
        let display = if host.display.is_inline_level() {
            LayoutDisplay::InlineBlock
        } else {
            LayoutDisplay::FlowRoot
        };
        host.force_layout_display(display);
        host.taffy.item_is_replaced = false;
        Self {
            content,
            container,
            show_icon,
        }
    }
}

impl ResolvedLayoutStyle {
    pub(crate) fn make_broken_image_icon(&mut self) {
        self.force_layout_display(LayoutDisplay::FlowRoot);
        let side = Dimension::length(16.0 * self.effective_zoom());
        self.taffy.size = Size {
            width: side,
            height: side,
        };
        self.taffy.float = if self.direction == InlineDirection::Rtl {
            Float::Right
        } else {
            Float::Left
        };
        self.out_of_flow = true;
        self.mark_replaced();
    }
}
