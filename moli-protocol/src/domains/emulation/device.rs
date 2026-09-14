use crate::conn::EmulatedDeviceMetrics;

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
    for (position, limit) in [
        (params.position_x.unwrap_or(0), screen_width),
        (params.position_y.unwrap_or(0), screen_height),
    ] {
        if position < 0 || position > i64::from(limit) {
            return Err(invalid());
        }
    }
    let (window_x, window_y) = match (params.position_x, params.position_y) {
        (Some(x), Some(y)) => (x as i32, y as i32),
        _ if params.mobile => (0, 0),
        _ => (base.window_x, base.window_y),
    };
    let screen_orientation = if let Some(orientation) = params.screen_orientation {
        let kind = match orientation.r#type.as_ref() {
            "portraitPrimary" => "portrait-primary",
            "portraitSecondary" => "portrait-secondary",
            "landscapePrimary" => "landscape-primary",
            "landscapeSecondary" => "landscape-secondary",
            _ => return Err(invalid()),
        };
        if !(0..360).contains(&orientation.angle) {
            return Err(invalid());
        }
        moli_page_types::ScreenOrientation {
            kind,
            angle: orientation.angle as u16,
        }
    } else {
        base.screen_orientation
    };
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
        window_x,
        window_y,
        screen_orientation,
    })
}
