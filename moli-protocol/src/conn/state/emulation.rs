#[derive(Debug, Clone, PartialEq)]
pub struct EmulatedDeviceMetrics {
    pub width: u32,
    pub height: u32,
    pub view: Option<moli_page_types::EmulatedView>,
    pub outer_width: u32,
    pub outer_height: u32,
    pub device_scale_factor: f64,
    pub screen_width: u32,
    pub screen_height: u32,
    pub screen_avail_height: u32,
    pub window_x: i32,
    pub window_y: i32,
    pub screen_orientation: moli_page_types::ScreenOrientation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EmulatedNetworkConditions {
    offline: bool,
}

impl EmulatedNetworkConditions {
    pub(crate) fn offline() -> Self {
        Self { offline: true }
    }

    pub(crate) fn navigator_online(&self) -> bool {
        !self.offline
    }
}

pub(crate) type EmulatedViewportSurface = moli_page_types::ViewportSurface;

impl EmulatedDeviceMetrics {
    pub(crate) fn screen_avail_height(&self) -> u32 {
        self.screen_avail_height
    }

    pub(crate) fn device_pixel_ratio(&self) -> f64 {
        if self.device_scale_factor.is_finite() && self.device_scale_factor > 0.0 {
            self.device_scale_factor
        } else {
            1.0
        }
    }

    pub(crate) fn viewport_surface(&self) -> EmulatedViewportSurface {
        EmulatedViewportSurface {
            inner_width: self.width,
            inner_height: self.height,
            outer_width: self.outer_width,
            outer_height: self.outer_height,
            device_pixel_ratio: self.device_pixel_ratio(),
            screen_width: self.screen_width,
            screen_height: self.screen_height,
            screen_avail_width: self.screen_width,
            screen_avail_height: self.screen_avail_height(),
            window_x: self.window_x,
            window_y: self.window_y,
            screen_orientation: self.screen_orientation,
            emulated_view: self.view,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmulatedGeolocationOverride {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: f64,
    pub altitude: Option<f64>,
    pub altitude_accuracy: Option<f64>,
    pub heading: Option<f64>,
    pub speed: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EmulatedGeolocationOverrideState {
    Position(EmulatedGeolocationOverride),
    PositionUnavailable,
}

impl EmulatedGeolocationOverrideState {
    pub(crate) fn position(&self) -> Option<&EmulatedGeolocationOverride> {
        match self {
            Self::Position(position) => Some(position),
            Self::PositionUnavailable => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmulatedMediaOverrides {
    /// CSS environment preference, independent of media query feature overrides.
    pub preferred_text_scale: Option<f32>,
    pub media: Option<String>,
    pub color_scheme: Option<String>,
    pub reduced_motion: Option<String>,
    pub reduced_transparency: Option<String>,
    pub forced_colors: Option<String>,
    pub contrast: Option<String>,
}

impl EmulatedMediaOverrides {
    pub(crate) fn overlaid_on(&self, defaults: &Self) -> Self {
        Self {
            preferred_text_scale: self.preferred_text_scale.or(defaults.preferred_text_scale),
            media: self.media.clone().or_else(|| defaults.media.clone()),
            color_scheme: self
                .color_scheme
                .clone()
                .or_else(|| defaults.color_scheme.clone()),
            reduced_motion: self
                .reduced_motion
                .clone()
                .or_else(|| defaults.reduced_motion.clone()),
            reduced_transparency: self
                .reduced_transparency
                .clone()
                .or_else(|| defaults.reduced_transparency.clone()),
            forced_colors: self
                .forced_colors
                .clone()
                .or_else(|| defaults.forced_colors.clone()),
            contrast: self.contrast.clone().or_else(|| defaults.contrast.clone()),
        }
    }
}

impl From<EmulatedMediaOverrides> for moli_core::page::EmulatedMediaOverrides {
    fn from(value: EmulatedMediaOverrides) -> Self {
        Self {
            preferred_text_scale: value.preferred_text_scale,
            media: value.media,
            color_scheme: value.color_scheme,
            reduced_motion: value.reduced_motion,
            reduced_transparency: value.reduced_transparency,
            forced_colors: value.forced_colors,
            contrast: value.contrast,
        }
    }
}

impl From<&EmulatedMediaOverrides> for moli_core::page::EmulatedMediaOverrides {
    fn from(value: &EmulatedMediaOverrides) -> Self {
        Self {
            preferred_text_scale: value.preferred_text_scale,
            media: value.media.clone(),
            color_scheme: value.color_scheme.clone(),
            reduced_motion: value.reduced_motion.clone(),
            reduced_transparency: value.reduced_transparency.clone(),
            forced_colors: value.forced_colors.clone(),
            contrast: value.contrast.clone(),
        }
    }
}

/// Values currently exposed by the shared Page target.
///
/// A DevTools session keeps its own raw Emulation handler state separately.
/// Commands copy the calling handler's value into this shared surface, just as
/// Chromium's per-session Emulation handlers update one target-wide renderer.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct EffectiveTargetEmulationState {
    pub(crate) default_background_color: Option<[u8; 4]>,
    pub(crate) vision_deficiency: moli_core::page::RendererVisionDeficiency,
    pub(crate) network_conditions: Option<EmulatedNetworkConditions>,
    pub(crate) geolocation_override: Option<EmulatedGeolocationOverrideState>,
    pub(crate) emulated_media: EmulatedMediaOverrides,
    pub(crate) emulated_device_metrics: Option<EmulatedDeviceMetrics>,
    pub(crate) max_touch_points: u32,
    pub(crate) emit_touch_events_for_mouse: bool,
    pub(crate) focus_emulation_enabled: bool,
    pub(crate) script_execution_disabled: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct EffectiveTargetEmulationStateDelta {
    pub(crate) network_conditions: bool,
    pub(crate) geolocation_override: bool,
    pub(crate) emulated_media: bool,
    pub(crate) emulated_device_metrics: bool,
    pub(crate) max_touch_points: bool,
    pub(crate) navigator_queries: bool,
    pub(crate) focus_emulation_enabled: bool,
    pub(crate) script_execution_disabled: bool,
}

impl EffectiveTargetEmulationStateDelta {
    pub(crate) fn surface_changed(self) -> bool {
        self.network_conditions
            || self.geolocation_override
            || self.emulated_device_metrics
            || self.max_touch_points
            || self.navigator_queries
            || self.focus_emulation_enabled
    }
}

impl EffectiveTargetEmulationState {
    /// Applies Chromium's per-handler disable contract to the shared target
    /// surface. Decisions use the disconnecting session's raw values, never a
    /// value last written by another session.
    pub(crate) fn disable_session_handler(
        &mut self,
        raw: &super::devtools_session::DevToolsEmulationSessionState,
    ) -> EffectiveTargetEmulationStateDelta {
        let previous = self.clone();
        if raw.network_conditions.is_some() {
            self.network_conditions = None;
        }
        if raw.geolocation_override.is_some() {
            self.geolocation_override = None;
        }
        // Blink clears media on every Emulation handler disable. Keep the
        // target-wide side effect while retaining each handler's raw copy.
        self.emulated_media = EmulatedMediaOverrides::default();
        self.default_background_color = None;
        self.vision_deficiency = Default::default();
        if raw.emulated_device_metrics.is_some() {
            self.emulated_device_metrics = None;
        }
        if raw.max_touch_points != 0 {
            self.max_touch_points = 0;
        }
        if raw.emit_touch_events_for_mouse {
            self.emit_touch_events_for_mouse = false;
        }
        if raw.focus_emulation_enabled {
            self.focus_emulation_enabled = false;
        }
        // Blink also sends the shared renderer an unconditional script
        // execution reset when any Emulation handler is disabled.
        self.script_execution_disabled = false;
        EffectiveTargetEmulationStateDelta {
            network_conditions: previous.network_conditions != self.network_conditions,
            geolocation_override: previous.geolocation_override != self.geolocation_override,
            emulated_media: previous.emulated_media != self.emulated_media,
            emulated_device_metrics: previous.emulated_device_metrics
                != self.emulated_device_metrics,
            max_touch_points: previous.max_touch_points != self.max_touch_points,
            navigator_queries: false,
            focus_emulation_enabled: previous.focus_emulation_enabled
                != self.focus_emulation_enabled,
            script_execution_disabled: previous.script_execution_disabled
                != self.script_execution_disabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EffectiveTargetEmulationState, EmulatedDeviceMetrics};

    #[test]
    fn device_pixel_ratio_normalizes_non_positive_and_non_finite_values() {
        let mut metrics = EmulatedDeviceMetrics {
            width: 800,
            height: 600,
            view: None,
            outer_width: 800,
            outer_height: 600,
            device_scale_factor: 2.0,
            screen_width: 800,
            screen_height: 600,
            screen_avail_height: 600,

            window_x: 0,
            window_y: 0,
            screen_orientation: Default::default(),
        };
        assert_eq!(metrics.device_pixel_ratio(), 2.0);

        metrics.device_scale_factor = 0.0;
        assert_eq!(metrics.device_pixel_ratio(), 1.0);

        metrics.device_scale_factor = f64::INFINITY;
        assert_eq!(metrics.device_pixel_ratio(), 1.0);

        metrics.device_scale_factor = f64::NAN;
        assert_eq!(metrics.device_pixel_ratio(), 1.0);
    }

    #[test]
    fn handler_disable_uses_raw_state_for_conditional_target_resets() {
        let mut effective = EffectiveTargetEmulationState {
            focus_emulation_enabled: true,
            script_execution_disabled: true,
            emulated_media: super::EmulatedMediaOverrides {
                color_scheme: Some("dark".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        let raw = crate::conn::DevToolsEmulationSessionState::default();

        let delta = effective.disable_session_handler(&raw);

        assert!(
            effective.focus_emulation_enabled,
            "an untouched handler must not clear another session's focus setting"
        );
        assert!(
            effective.emulated_media.color_scheme.is_none(),
            "Blink clears the shared media override on every handler disable"
        );
        assert!(!delta.focus_emulation_enabled);
        assert!(delta.emulated_media);
        assert!(!effective.script_execution_disabled);
        assert!(delta.script_execution_disabled);
    }
}
