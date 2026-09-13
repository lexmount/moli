use crate::conn::{EmulatedDeviceMetrics, viewport_surface_install_script};

pub(super) fn live_device_metrics_override_script(
    metrics: &EmulatedDeviceMetrics,
    remember_original_descriptors: bool,
) -> String {
    viewport_surface_install_script(&metrics.viewport_surface(), remember_original_descriptors)
}

pub(super) const LIVE_DEVICE_METRICS_CLEAR_SCRIPT: &str = r#"
(() => {
  const storeKey = '__moliDeviceMetricsOriginalDescriptors';
  const descriptors = globalThis[storeKey] || {};
  let isTopLevelWindow = true;
  try {
    isTopLevelWindow = globalThis.parent === globalThis;
  } catch (_) {
  }
  const restoreDescriptor = (scope, object, property) => {
    if (!object) {
      return;
    }
    const key = `${scope}.${property}`;
    try {
      if (Object.prototype.hasOwnProperty.call(descriptors, key)) {
        const original = descriptors[key];
        if (original && original.existed && original.descriptor) {
          Object.defineProperty(object, property, original.descriptor);
        } else {
          delete object[property];
        }
      } else {
        delete object[property];
      }
    } catch (_) {
    }
  };
  const windowProperties = ['outerWidth', 'outerHeight', 'devicePixelRatio'];
  if (isTopLevelWindow) {
    windowProperties.unshift('innerWidth', 'innerHeight');
  }
  for (const property of windowProperties) {
    restoreDescriptor('window', globalThis, property);
  }
  for (const property of ['width', 'height', 'availWidth', 'availHeight']) {
    restoreDescriptor('screen', globalThis.screen, property);
  }
  try {
    delete globalThis[storeKey];
  } catch (_) {
  }
})()
"#;

/// CDP zero dimensions remove the layout override for that axis. The actual
/// visible widget is resized only when both axes are supplied; it can differ
/// from the previous emulated layout after a single-axis command.
pub(super) fn metrics_from_cdp(
    params: super::params::SetDeviceMetricsOverrideParams,
    previous: Option<&EmulatedDeviceMetrics>,
    base: &crate::conn::EmulatedViewportSurface,
) -> Result<EmulatedDeviceMetrics, crate::devtools_runtime::DevToolsError> {
    use crate::devtools_runtime::{DevToolsError, DevToolsErrorKind};
    let invalid = || DevToolsError::new(DevToolsErrorKind::InvalidArgument, "InvalidParams");
    let size = |value: i64| {
        if (0..=10_000_000).contains(&value) {
            Ok(value as u32)
        } else {
            Err(invalid())
        }
    };
    let width = size(params.width)?;
    let height = size(params.height)?;
    let screen_width = size(params.screen_width.unwrap_or(0))?;
    let screen_height = size(params.screen_height.unwrap_or(0))?;
    let scale = params.scale.unwrap_or(1.0);
    if !params.device_scale_factor.is_finite()
        || params.device_scale_factor < 0.0
        || !scale.is_finite()
        || scale <= 0.0
        || scale > 10.0
    {
        return Err(invalid());
    }
    let mut visible_width = previous.map_or(base.inner_width, |m| m.visible_width);
    let mut visible_height = previous.map_or(base.inner_height, |m| m.visible_height);
    if width != 0 && height != 0 && !params.dont_set_visible_size.unwrap_or(false) {
        visible_width = width;
        visible_height = height;
    }
    let width = if width == 0 {
        (f64::from(visible_width) / scale).round() as u32
    } else {
        width
    };
    let height = if height == 0 {
        (f64::from(visible_height) / scale).round() as u32
    } else {
        height
    };
    let (screen_width, screen_height) = if screen_width != 0 && screen_height != 0 {
        (screen_width, screen_height)
    } else if params.mobile {
        (width, height)
    } else {
        (base.screen_width, base.screen_height)
    };
    Ok(EmulatedDeviceMetrics {
        width,
        height,
        visible_width,
        visible_height,
        outer_width: if params.mobile {
            width
        } else {
            base.outer_width
        },
        outer_height: if params.mobile {
            height
        } else {
            base.outer_height
        },
        device_scale_factor: if params.device_scale_factor == 0.0 {
            base.device_pixel_ratio
        } else {
            params.device_scale_factor
        },
        screen_width,
        screen_height,
        screen_avail_height: screen_height,
    })
}

#[cfg(test)]
mod tests {
    use super::LIVE_DEVICE_METRICS_CLEAR_SCRIPT;

    #[test]
    fn live_device_metrics_clear_script_uses_plain_helper_store() {
        assert!(!LIVE_DEVICE_METRICS_CLEAR_SCRIPT.contains("Object.create(null)"));
        assert!(LIVE_DEVICE_METRICS_CLEAR_SCRIPT.contains("globalThis.parent === globalThis"));
        assert!(LIVE_DEVICE_METRICS_CLEAR_SCRIPT.contains("if (isTopLevelWindow)"));
    }
}
