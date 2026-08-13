// SPDX-License-Identifier: MIT OR Apache-2.0

use taffy::{ReplacedSizingContext, ResolvedAspectRatio, Size, SizeContainment, WritingMode};

use crate::{LayoutReplacedKind, ReplacedMetrics};

/// Browser-owned natural metrics for one replaced box.
///
/// This adapter normalizes DOM/resource inputs and HTML default object sizes.
/// CSS preferred/min/max sizing, percentage resolution, box-sizing and ratio
/// transfer remain owned by Taffy's replaced sizing algorithm.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReplacedContext {
    natural_size: Size<f32>,
    preferred_size_hint: Size<Option<f32>>,
    inherent_ratio: Option<f32>,
}

impl ReplacedContext {
    pub(crate) fn for_element(kind: LayoutReplacedKind, metrics: Option<ReplacedMetrics>) -> Self {
        let mut metrics = metrics.unwrap_or_default();
        metrics.intrinsic_width = valid_dimension(metrics.intrinsic_width);
        metrics.intrinsic_height = valid_dimension(metrics.intrinsic_height);
        metrics.attribute_width = valid_dimension(metrics.attribute_width);
        metrics.attribute_height = valid_dimension(metrics.attribute_height);
        metrics.intrinsic_ratio = metrics
            .intrinsic_ratio
            .filter(|ratio| ratio.is_finite() && *ratio > 0.0);
        let default_size = match kind {
            LayoutReplacedKind::FormControl => Size {
                width: 160.0,
                height: 24.0,
            },
            // An HTML image without available natural dimensions represents
            // no content. The 300x150 default object size applies to the
            // remaining replaced categories.
            LayoutReplacedKind::Image => Size::ZERO,
            LayoutReplacedKind::Svg
            | LayoutReplacedKind::Canvas
            | LayoutReplacedKind::Embedded
            | LayoutReplacedKind::Frame
            | LayoutReplacedKind::Media => Size {
                width: 300.0,
                height: 150.0,
            },
        };

        // Canvas width/height attributes define the intrinsic bitmap
        // coordinate space independently: specifying only width does not
        // scale the default 150px height.
        if kind == LayoutReplacedKind::Canvas {
            metrics.intrinsic_width = metrics.attribute_width.or(Some(default_size.width));
            metrics.intrinsic_height = metrics.attribute_height.or(Some(default_size.height));
            metrics.attribute_width = None;
            metrics.attribute_height = None;
        }
        let inherent_ratio = metrics
            .intrinsic_ratio
            .or_else(|| {
                metrics
                    .intrinsic_width
                    .zip(metrics.intrinsic_height)
                    .filter(|(_, height)| *height > 0.0)
                    .map(|(width, height)| width / height)
            })
            .or_else(|| {
                // HTML dimension attributes provide an aspect-ratio hint for
                // image-like replaced elements before decoded pixels exist.
                matches!(kind, LayoutReplacedKind::Image | LayoutReplacedKind::Media)
                    .then(|| {
                        metrics
                            .attribute_width
                            .zip(metrics.attribute_height)
                            .filter(|(_, height)| *height > 0.0)
                            .map(|(width, height)| width / height)
                    })
                    .flatten()
            });
        let natural_size = concrete_object_size(
            metrics.intrinsic_width,
            metrics.intrinsic_height,
            inherent_ratio,
            default_size,
        );
        let preferred_size_hint = Size {
            width: metrics.attribute_width,
            height: metrics.attribute_height,
        };
        // Canvas dimensions define its intrinsic coordinate space even while
        // its pixels remain an unavailable placeholder in Phase 4.
        let inherent_ratio = inherent_ratio.or_else(|| {
            (kind == LayoutReplacedKind::Canvas).then_some(natural_size.width / natural_size.height)
        });
        Self {
            natural_size,
            preferred_size_hint,
            inherent_ratio,
        }
    }

    pub(crate) fn form_control(size: Size<f32>) -> Self {
        Self {
            natural_size: size,
            preferred_size_hint: Size::NONE,
            inherent_ratio: None,
        }
    }

    pub(crate) const fn inherent_ratio(&self) -> Option<f32> {
        self.inherent_ratio
    }

    pub(crate) const fn sizing_context(
        self,
        writing_mode: WritingMode,
        aspect_ratio: ResolvedAspectRatio,
        size_containment: SizeContainment,
    ) -> ReplacedSizingContext {
        ReplacedSizingContext::new(
            writing_mode,
            aspect_ratio,
            size_containment,
            self.natural_size,
            self.preferred_size_hint,
        )
    }
}

fn concrete_object_size(
    width: Option<f32>,
    height: Option<f32>,
    ratio: Option<f32>,
    default_size: Size<f32>,
) -> Size<f32> {
    match (width, height, ratio) {
        (Some(width), Some(height), _) => Size { width, height },
        (Some(width), None, Some(ratio)) => Size {
            width,
            height: width / ratio,
        },
        (Some(width), None, None) => Size {
            width,
            height: default_size.height,
        },
        (None, Some(height), Some(ratio)) => Size {
            width: height * ratio,
            height,
        },
        (None, Some(height), None) => Size {
            width: default_size.width,
            height,
        },
        (None, None, Some(ratio)) if default_size.width > 0.0 && default_size.height > 0.0 => {
            let width_at_default_height = default_size.height * ratio;
            if width_at_default_height <= default_size.width {
                Size {
                    width: width_at_default_height,
                    height: default_size.height,
                }
            } else {
                Size {
                    width: default_size.width,
                    height: default_size.width / ratio,
                }
            }
        }
        (None, None, _) => default_size,
    }
}

fn valid_dimension(value: Option<f32>) -> Option<f32> {
    value.filter(|value| value.is_finite() && *value >= 0.0)
}
