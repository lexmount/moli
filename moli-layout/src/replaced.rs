// SPDX-License-Identifier: MIT OR Apache-2.0

use taffy::{
    ReplacedNaturalSizing, ReplacedSizingContext, ResolvedAspectRatio, Size, SizeContainment,
    WritingMode,
};

use crate::{LayoutReplacedKind, ReplacedMetrics};

/// Browser-owned natural metrics for one replaced box.
///
/// This adapter preserves DOM/resource natural-axis provenance and supplies
/// the HTML category's default object size. CSS natural-size normalization,
/// preferred/min/max sizing, box-sizing and ratio transfer remain owned by
/// Taffy's replaced sizing algorithm.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReplacedContext {
    natural_sizing: ReplacedNaturalSizing,
    inherent_ratio: Option<f32>,
}

impl ReplacedContext {
    pub(crate) fn for_element(kind: LayoutReplacedKind, metrics: Option<ReplacedMetrics>) -> Self {
        let mut metrics = metrics.unwrap_or_default();
        metrics.intrinsic_width = valid_dimension(metrics.intrinsic_width);
        metrics.intrinsic_height = valid_dimension(metrics.intrinsic_height);
        metrics.intrinsic_ratio = metrics
            .intrinsic_ratio
            .filter(|ratio| ratio.is_finite() && *ratio > 0.0);
        let category_default_size = match kind {
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
        let default_object_size = metrics
            .default_object_size
            .filter(|size| {
                size.width.is_finite()
                    && size.width >= 0.0
                    && size.height.is_finite()
                    && size.height >= 0.0
            })
            .map(|size| Size {
                width: size.width,
                height: size.height,
            })
            .unwrap_or(category_default_size);

        let inherent_ratio = metrics.intrinsic_ratio.or_else(|| {
            metrics
                .intrinsic_width
                .zip(metrics.intrinsic_height)
                .filter(|(_, height)| *height > 0.0)
                .map(|(width, height)| width / height)
        });
        let natural_sizing = ReplacedNaturalSizing::new(
            Size {
                width: metrics.intrinsic_width,
                height: metrics.intrinsic_height,
            },
            default_object_size,
        );
        // Canvas dimensions define its intrinsic coordinate space even while
        // its pixels remain an unavailable placeholder in Phase 4.
        let inherent_ratio = inherent_ratio.or_else(|| {
            (kind == LayoutReplacedKind::Canvas && default_object_size.height > 0.0)
                .then(|| default_object_size.width / default_object_size.height)
        });
        Self {
            natural_sizing,
            inherent_ratio,
        }
    }

    pub(crate) fn form_control(size: Size<f32>) -> Self {
        Self {
            natural_sizing: ReplacedNaturalSizing::fixed(size),
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
            self.natural_sizing,
        )
    }
}

fn valid_dimension(value: Option<f32>) -> Option<f32> {
    value.filter(|value| value.is_finite() && *value >= 0.0)
}
