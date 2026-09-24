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
pub struct EmulatedNetworkConditions {
    offline: bool,
}

impl EmulatedNetworkConditions {
    pub fn offline() -> Self {
        Self { offline: true }
    }

    pub fn navigator_online(&self) -> bool {
        !self.offline
    }
}

pub type EmulatedViewportSurface = moli_page_types::ViewportSurface;

impl EmulatedDeviceMetrics {
    pub fn screen_avail_height(&self) -> u32 {
        self.screen_avail_height
    }

    pub fn device_pixel_ratio(&self) -> f64 {
        if self.device_scale_factor.is_finite() && self.device_scale_factor > 0.0 {
            self.device_scale_factor
        } else {
            1.0
        }
    }

    pub fn viewport_surface(&self) -> EmulatedViewportSurface {
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
    pub fn position(&self) -> Option<&EmulatedGeolocationOverride> {
        match self {
            Self::Position(position) => Some(position),
            Self::PositionUnavailable => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmulatedMediaOverrides {
    pub media: Option<String>,
    pub color_scheme: Option<String>,
    pub reduced_motion: Option<String>,
    pub reduced_transparency: Option<String>,
    pub forced_colors: Option<String>,
    pub contrast: Option<String>,
}

impl From<EmulatedMediaOverrides> for crate::page::EmulatedMediaOverrides {
    fn from(value: EmulatedMediaOverrides) -> Self {
        Self {
            media: value.media,
            color_scheme: value.color_scheme,
            reduced_motion: value.reduced_motion,
            reduced_transparency: value.reduced_transparency,
            forced_colors: value.forced_colors,
            contrast: value.contrast,
        }
    }
}

impl From<&EmulatedMediaOverrides> for crate::page::EmulatedMediaOverrides {
    fn from(value: &EmulatedMediaOverrides) -> Self {
        Self {
            media: value.media.clone(),
            color_scheme: value.color_scheme.clone(),
            reduced_motion: value.reduced_motion.clone(),
            reduced_transparency: value.reduced_transparency.clone(),
            forced_colors: value.forced_colors.clone(),
            contrast: value.contrast.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EmulatedDeviceMetrics;

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
}
