use style::context::QuirksMode;

use super::{
    FullStyleWorldSnapshot, StyleTreeScopeVersions, StyleViewport, StyleWorldEnvironment,
    StyloStyleEnvironment,
};

pub(super) const DEFAULT_VIEWPORT_WIDTH: f32 = 1024.0;
pub(super) const DEFAULT_VIEWPORT_HEIGHT: f32 = 768.0;

/// Style-affecting state of a retained Document style world.
///
/// Stylesheet contents intentionally do not participate. Persistent active
/// collections and dirty records update those incrementally; hashing complete
/// source vectors here would reintroduce the snapshot model. The Document URL
/// is also intentionally absent: same-Document history mutations change that
/// address without replacing the Document's style world.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct StyleWorldKey {
    pub(super) viewport_width_bits: u32,
    pub(super) viewport_height_bits: u32,
    pub(super) screen_width_bits: u32,
    pub(super) screen_height_bits: u32,
    pub(super) device_pixel_ratio_bits: u32,
    pub(super) environment: StyloStyleEnvironment,
    pub(super) quirks_mode: QuirksMode,
    pub(super) tree_scope_versions: Option<StyleTreeScopeVersions>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StyleWorldKeyMismatchTrace {
    pub(super) viewport_changed: bool,
    pub(super) previous_viewport_width_bits: u32,
    pub(super) next_viewport_width_bits: u32,
    pub(super) previous_viewport_height_bits: u32,
    pub(super) next_viewport_height_bits: u32,
    pub(super) screen_changed: bool,
    pub(super) previous_screen_width_bits: u32,
    pub(super) next_screen_width_bits: u32,
    pub(super) previous_screen_height_bits: u32,
    pub(super) next_screen_height_bits: u32,
    pub(super) environment_changed: bool,
    pub(super) quirks_mode_changed: bool,
    pub(super) tree_scope_versions_changed: bool,
    pub(super) previous_tree_scope_versions: Option<StyleTreeScopeVersions>,
    pub(super) next_tree_scope_versions: Option<StyleTreeScopeVersions>,
}

impl StyleWorldKey {
    #[cfg(test)]
    pub(super) fn new(inputs: &FullStyleWorldSnapshot, viewport: impl Into<StyleViewport>) -> Self {
        Self::build(inputs, viewport.into(), None)
    }

    pub(super) fn new_for_observation(
        inputs: &FullStyleWorldSnapshot,
        viewport: StyleViewport,
        tree_scope_versions: StyleTreeScopeVersions,
    ) -> Self {
        Self::build(inputs, viewport, Some(tree_scope_versions))
    }

    fn build(
        inputs: &FullStyleWorldSnapshot,
        viewport: StyleViewport,
        tree_scope_versions: Option<StyleTreeScopeVersions>,
    ) -> Self {
        let viewport_width_bits = style_dimension_bits(viewport.width, DEFAULT_VIEWPORT_WIDTH);
        let viewport_height_bits = style_dimension_bits(viewport.height, DEFAULT_VIEWPORT_HEIGHT);
        Self {
            viewport_width_bits,
            viewport_height_bits,
            screen_width_bits: style_dimension_bits(
                viewport.screen_width,
                f32::from_bits(viewport_width_bits),
            ),
            screen_height_bits: style_dimension_bits(
                viewport.screen_height,
                f32::from_bits(viewport_height_bits),
            ),
            device_pixel_ratio_bits: style_dimension_bits(viewport.device_pixel_ratio, 1.0),
            environment: inputs.environment,
            quirks_mode: inputs.quirks_mode,
            tree_scope_versions,
        }
    }

    pub(super) fn style_viewport(&self) -> StyleViewport {
        StyleViewport::new(
            Some(f64::from(f32::from_bits(self.viewport_width_bits))),
            Some(f64::from(f32::from_bits(self.viewport_height_bits))),
        )
        .with_screen_size(
            Some(f64::from(f32::from_bits(self.screen_width_bits))),
            Some(f64::from(f32::from_bits(self.screen_height_bits))),
        )
        .with_device_pixel_ratio(Some(f64::from(f32::from_bits(
            self.device_pixel_ratio_bits,
        ))))
    }

    pub(super) fn updated_for_observation(&self, environment: &StyleWorldEnvironment) -> Self {
        let mut next = self.clone();
        let viewport_width_bits =
            style_dimension_bits(environment.viewport.width, DEFAULT_VIEWPORT_WIDTH);
        let viewport_height_bits =
            style_dimension_bits(environment.viewport.height, DEFAULT_VIEWPORT_HEIGHT);
        next.viewport_width_bits = viewport_width_bits;
        next.viewport_height_bits = viewport_height_bits;
        next.screen_width_bits = style_dimension_bits(
            environment.viewport.screen_width,
            f32::from_bits(viewport_width_bits),
        );
        next.screen_height_bits = style_dimension_bits(
            environment.viewport.screen_height,
            f32::from_bits(viewport_height_bits),
        );
        next.device_pixel_ratio_bits =
            style_dimension_bits(environment.viewport.device_pixel_ratio, 1.0);
        next.environment = environment.media;
        next.quirks_mode = environment.quirks_mode;
        next.tree_scope_versions = Some(environment.tree_scope_versions);
        next
    }

    pub(super) fn mismatch_trace(&self, next: &Self) -> StyleWorldKeyMismatchTrace {
        StyleWorldKeyMismatchTrace {
            viewport_changed: self.viewport_width_bits != next.viewport_width_bits
                || self.viewport_height_bits != next.viewport_height_bits,
            previous_viewport_width_bits: self.viewport_width_bits,
            next_viewport_width_bits: next.viewport_width_bits,
            previous_viewport_height_bits: self.viewport_height_bits,
            next_viewport_height_bits: next.viewport_height_bits,
            screen_changed: self.screen_width_bits != next.screen_width_bits
                || self.screen_height_bits != next.screen_height_bits
                || self.device_pixel_ratio_bits != next.device_pixel_ratio_bits,
            previous_screen_width_bits: self.screen_width_bits,
            next_screen_width_bits: next.screen_width_bits,
            previous_screen_height_bits: self.screen_height_bits,
            next_screen_height_bits: next.screen_height_bits,
            environment_changed: self.environment != next.environment,
            quirks_mode_changed: self.quirks_mode != next.quirks_mode,
            tree_scope_versions_changed: self.tree_scope_versions != next.tree_scope_versions,
            previous_tree_scope_versions: self.tree_scope_versions,
            next_tree_scope_versions: next.tree_scope_versions,
        }
    }

    pub(super) fn matches_observation_environment(
        &self,
        viewport: StyleViewport,
        environment: StyloStyleEnvironment,
        quirks_mode: QuirksMode,
        tree_scope_versions: StyleTreeScopeVersions,
    ) -> bool {
        let viewport_width_bits = style_dimension_bits(viewport.width, DEFAULT_VIEWPORT_WIDTH);
        let viewport_height_bits = style_dimension_bits(viewport.height, DEFAULT_VIEWPORT_HEIGHT);
        self.viewport_width_bits == viewport_width_bits
            && self.viewport_height_bits == viewport_height_bits
            && self.screen_width_bits
                == style_dimension_bits(viewport.screen_width, f32::from_bits(viewport_width_bits))
            && self.screen_height_bits
                == style_dimension_bits(
                    viewport.screen_height,
                    f32::from_bits(viewport_height_bits),
                )
            && self.device_pixel_ratio_bits
                == style_dimension_bits(viewport.device_pixel_ratio, 1.0)
            && self.environment == environment
            && self.quirks_mode == quirks_mode
            && self.tree_scope_versions == Some(tree_scope_versions)
    }

    pub(super) fn requires_replacement_for_observation(&self, quirks_mode: QuirksMode) -> bool {
        self.quirks_mode != quirks_mode
    }

    pub(super) fn device_differs_from_observation(
        &self,
        viewport: StyleViewport,
        environment: StyloStyleEnvironment,
    ) -> bool {
        let viewport_width_bits = style_dimension_bits(viewport.width, DEFAULT_VIEWPORT_WIDTH);
        let viewport_height_bits = style_dimension_bits(viewport.height, DEFAULT_VIEWPORT_HEIGHT);
        self.viewport_width_bits != viewport_width_bits
            || self.viewport_height_bits != viewport_height_bits
            || self.screen_width_bits
                != style_dimension_bits(viewport.screen_width, f32::from_bits(viewport_width_bits))
            || self.screen_height_bits
                != style_dimension_bits(
                    viewport.screen_height,
                    f32::from_bits(viewport_height_bits),
                )
            || self.device_pixel_ratio_bits
                != style_dimension_bits(viewport.device_pixel_ratio, 1.0)
            || self.environment != environment
    }
}

impl StyleWorldKeyMismatchTrace {
    pub(super) fn requires_style_system_replacement(&self) -> bool {
        self.quirks_mode_changed
    }
}

fn style_dimension_bits(value: Option<f64>, fallback: f32) -> u32 {
    value
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as f32)
        .unwrap_or(fallback)
        .to_bits()
}
