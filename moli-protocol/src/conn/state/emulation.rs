use serde_json::json;

#[derive(Debug, Clone, PartialEq)]
pub struct EmulatedDeviceMetrics {
    pub width: u32,
    pub height: u32,
    pub device_scale_factor: f64,
    pub screen_width: u32,
    pub screen_height: u32,
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

const DEFAULT_VIEWPORT_WIDTH: u32 = 1920;
const DEFAULT_VIEWPORT_HEIGHT: u32 = 1080;
const DEFAULT_SCREEN_WIDTH: u32 = 1920;
const DEFAULT_SCREEN_HEIGHT: u32 = 1080;
const DEFAULT_SCREEN_AVAIL_HEIGHT: u32 = 1040;

fn screen_avail_height_from_screen_height(screen_height: u32) -> u32 {
    screen_height.min(DEFAULT_SCREEN_AVAIL_HEIGHT)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EmulatedViewportSurface {
    pub inner_width: u32,
    pub inner_height: u32,
    pub outer_width: u32,
    pub outer_height: u32,
    pub device_pixel_ratio: f64,
    pub screen_width: u32,
    pub screen_height: u32,
    pub screen_avail_width: u32,
    pub screen_avail_height: u32,
}

impl Default for EmulatedViewportSurface {
    fn default() -> Self {
        Self {
            inner_width: DEFAULT_VIEWPORT_WIDTH,
            inner_height: DEFAULT_VIEWPORT_HEIGHT,
            outer_width: DEFAULT_VIEWPORT_WIDTH,
            outer_height: DEFAULT_VIEWPORT_HEIGHT,
            device_pixel_ratio: 1.0,
            screen_width: DEFAULT_SCREEN_WIDTH,
            screen_height: DEFAULT_SCREEN_HEIGHT,
            screen_avail_width: DEFAULT_SCREEN_WIDTH,
            screen_avail_height: screen_avail_height_from_screen_height(DEFAULT_SCREEN_HEIGHT),
        }
    }
}

impl EmulatedViewportSurface {
    pub(crate) fn from_metrics(metrics: Option<&EmulatedDeviceMetrics>) -> Self {
        metrics.map_or_else(Self::default, EmulatedDeviceMetrics::viewport_surface)
    }

    pub(crate) fn to_page_viewport_surface(&self) -> moli_core::page::ViewportSurface {
        moli_core::page::ViewportSurface {
            inner_width: self.inner_width,
            inner_height: self.inner_height,
            outer_width: self.outer_width,
            outer_height: self.outer_height,
            device_pixel_ratio: self.device_pixel_ratio,
            screen_width: self.screen_width,
            screen_height: self.screen_height,
            screen_avail_width: self.screen_avail_width,
            screen_avail_height: self.screen_avail_height,
        }
    }

    pub(crate) fn as_json_string(&self) -> String {
        json!({
            "innerWidth": self.inner_width,
            "innerHeight": self.inner_height,
            "outerWidth": self.outer_width,
            "outerHeight": self.outer_height,
            "devicePixelRatio": self.device_pixel_ratio,
            "screenWidth": self.screen_width,
            "screenHeight": self.screen_height,
            "screenAvailWidth": self.screen_avail_width,
            "screenAvailHeight": self.screen_avail_height,
        })
        .to_string()
    }
}

impl EmulatedDeviceMetrics {
    pub(crate) fn screen_avail_height(&self) -> u32 {
        screen_avail_height_from_screen_height(self.screen_height)
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
            outer_width: self.width,
            outer_height: self.height,
            device_pixel_ratio: self.device_pixel_ratio(),
            screen_width: self.screen_width,
            screen_height: self.screen_height,
            screen_avail_width: self.screen_width,
            screen_avail_height: self.screen_avail_height(),
        }
    }
}

pub(crate) fn viewport_surface_install_script(
    surface: &EmulatedViewportSurface,
    remember_original_descriptors: bool,
) -> String {
    let descriptor_setup = if remember_original_descriptors {
        r#"
  const storeKey = '__moliDeviceMetricsOriginalDescriptors';
  const descriptors = globalThis[storeKey] || {};
  try {
    Object.defineProperty(globalThis, storeKey, {
      configurable: true,
      value: descriptors,
    });
  } catch (_) {
  }
  const rememberViewportSurfaceDescriptor = (scope, object, property) => {
    if (!object) {
      return;
    }
    const key = `${scope}.${property}`;
    if (Object.prototype.hasOwnProperty.call(descriptors, key)) {
      return;
    }
    descriptors[key] = {
      existed: Object.prototype.hasOwnProperty.call(object, property),
      descriptor: Object.getOwnPropertyDescriptor(object, property),
    };
  };
"#
    } else {
        r#"
  const rememberViewportSurfaceDescriptor = (_scope, _object, _property) => {};
"#
    };
    format!(
        r#"
(() => {{
  const surface = {surface};
  {descriptor_setup}
  let isTopLevelWindow = true;
  try {{
    isTopLevelWindow = globalThis.parent === globalThis;
  }} catch (_) {{
  }}
  const defineViewportSurfaceGetter = (object, property, value) => {{
    if (!object) {{
      return;
    }}
    try {{
      Object.defineProperty(object, property, {{
        configurable: true,
        get: () => value,
      }});
    }} catch (_) {{
    }}
  }};
  const installViewportSurfaceGetter = (scope, object, property, value) => {{
    rememberViewportSurfaceDescriptor(scope, object, property);
    defineViewportSurfaceGetter(object, property, value);
  }};
  const windowValues = [
    ['innerWidth', surface.innerWidth, true],
    ['innerHeight', surface.innerHeight, true],
    ['outerWidth', surface.outerWidth, false],
    ['outerHeight', surface.outerHeight, false],
    ['devicePixelRatio', surface.devicePixelRatio, false],
  ];
  const screenValues = [
    ['width', surface.screenWidth],
    ['height', surface.screenHeight],
    ['availWidth', surface.screenAvailWidth],
    ['availHeight', surface.screenAvailHeight],
  ];
  for (const [property, value, topLevelOnly] of windowValues) {{
    if (topLevelOnly && !isTopLevelWindow) {{
      continue;
    }}
    installViewportSurfaceGetter('window', globalThis, property, value);
  }}
  for (const [property, value] of screenValues) {{
    installViewportSurfaceGetter('screen', globalThis.screen, property, value);
  }}
}})();
"#,
        surface = surface.as_json_string(),
        descriptor_setup = descriptor_setup,
    )
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmulatedMediaOverrides {
    pub media: Option<String>,
    pub color_scheme: Option<String>,
    pub reduced_motion: Option<String>,
    pub forced_colors: Option<String>,
    pub contrast: Option<String>,
}

impl From<EmulatedMediaOverrides> for moli_core::page::EmulatedMediaOverrides {
    fn from(value: EmulatedMediaOverrides) -> Self {
        Self {
            media: value.media,
            color_scheme: value.color_scheme,
            reduced_motion: value.reduced_motion,
            forced_colors: value.forced_colors,
            contrast: value.contrast,
        }
    }
}

impl From<&EmulatedMediaOverrides> for moli_core::page::EmulatedMediaOverrides {
    fn from(value: &EmulatedMediaOverrides) -> Self {
        Self {
            media: value.media.clone(),
            color_scheme: value.color_scheme.clone(),
            reduced_motion: value.reduced_motion.clone(),
            forced_colors: value.forced_colors.clone(),
            contrast: value.contrast.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EmulatedDeviceMetrics, EmulatedViewportSurface, screen_avail_height_from_screen_height,
        viewport_surface_install_script,
    };

    #[test]
    fn screen_available_height_never_exceeds_zero_screen_height() {
        assert_eq!(screen_avail_height_from_screen_height(0), 0);
    }

    #[test]
    fn device_pixel_ratio_normalizes_non_positive_and_non_finite_values() {
        let mut metrics = EmulatedDeviceMetrics {
            width: 800,
            height: 600,
            device_scale_factor: 2.0,
            screen_width: 800,
            screen_height: 600,
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
    fn viewport_surface_install_script_uses_plain_helper_store() {
        let script = viewport_surface_install_script(&EmulatedViewportSurface::default(), true);

        assert!(!script.contains("Object.create(null)"));
        assert!(script.contains("globalThis.parent === globalThis"));
        assert!(script.contains("['innerWidth', surface.innerWidth, true]"));
        assert!(script.contains("if (topLevelOnly && !isTopLevelWindow)"));
    }
}
