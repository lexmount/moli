use super::params::{ScrollbarType, SetDeviceMetricsOverrideParams, ViewportMeta};
use crate::conn::EmulatedDeviceMetrics;

/// CDP zero dimensions remove the layout override for that axis. The actual
/// visible widget is resized only when both axes are supplied; it can differ
/// from the previous emulated layout after a single-axis command.
pub(super) fn metrics_from_cdp(
    params: SetDeviceMetricsOverrideParams,
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
    // Mobile emulation also needs viewport-meta processing, autosizing and
    // overlay scrollbars. Reject it instead of publishing only its geometry.
    // Default/disabled options below retain the native desktop behavior.
    for (unsupported, setting) in [
        (params.mobile, "mobile=true"),
        (params.display_feature.is_some(), "displayFeature"),
        (params.device_posture.is_some(), "devicePosture"),
        (
            matches!(params.scrollbar_type, Some(ScrollbarType::Overlay)),
            "scrollbarType=overlay",
        ),
        (
            params.screen_orientation_lock_emulation == Some(true),
            "screenOrientationLockEmulation=true",
        ),
        (
            matches!(params.viewport_meta, Some(ViewportMeta::Enable)),
            "viewportMeta=enable",
        ),
    ] {
        if unsupported {
            return Err(DevToolsError::new(
                DevToolsErrorKind::Unsupported,
                format!("Emulation.setDeviceMetricsOverride does not support {setting}."),
            ));
        }
    }
    let device_scale_factor = if params.device_scale_factor == 0.0 {
        base.device_pixel_ratio
    } else {
        params.device_scale_factor
    };
    let mut visible_size = previous.map_or(base.visible_size(), |metrics| {
        metrics.viewport_surface().visible_size()
    });
    let mut requested_size = (width, height);
    let mut viewport = None;
    if let Some(clip) = params.viewport {
        if ![clip.x, clip.y, clip.width, clip.height, clip.scale]
            .into_iter()
            .all(f64::is_finite)
        {
            return Err(invalid());
        }
        let viewport_scale = clip.scale * device_scale_factor / base.device_pixel_ratio;
        if !viewport_scale.is_finite() {
            return Err(invalid());
        }
        // Chromium accepts zero/negative clip scales. They can leave no visible
        // content, but must not turn a negative extent into an enormous widget.
        let dimension = |value: f64| value.round().clamp(0.0, f64::from(i32::MAX)) as u32;
        requested_size = (
            dimension(clip.width * viewport_scale),
            dimension(clip.height * viewport_scale),
        );
        if clip.x >= 0.0 {
            viewport = Some(moli_page_types::EmulatedViewport {
                x: clip.x,
                y: clip.y,
                scale: viewport_scale,
            });
        }
    }
    if requested_size.0 != 0
        && requested_size.1 != 0
        && !params.dont_set_visible_size.unwrap_or(false)
    {
        visible_size = requested_size;
    }
    let width = if width == 0 {
        (f64::from(visible_size.0) / scale).round() as u32
    } else {
        width
    };
    let height = if height == 0 {
        (f64::from(visible_size.1) / scale).round() as u32
    } else {
        height
    };
    let (screen_width, screen_height) = if screen_width != 0 && screen_height != 0 {
        (screen_width, screen_height)
    } else {
        (base.screen_width, base.screen_height)
    };
    Ok(EmulatedDeviceMetrics {
        width,
        height,
        view: Some(moli_page_types::EmulatedView {
            width: visible_size.0,
            height: visible_size.1,
            scale,
            native_device_pixel_ratio: base.device_pixel_ratio,
            viewport,
        }),
        outer_width: base.outer_width,
        outer_height: base.outer_height,
        device_scale_factor,
        screen_width,
        screen_height,
        screen_avail_height: screen_height,
        window_x,
        window_y,
        screen_orientation,
    })
}
