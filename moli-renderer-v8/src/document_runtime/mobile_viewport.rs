use moli_page_types::ViewportSurface;

pub(super) fn resolve(mut surface: ViewportSurface, content: Option<&str>) -> ViewportSurface {
    let Some(view) = surface
        .emulated_view
        .as_mut()
        .and_then(|view| view.mobile_viewport.as_mut())
    else {
        return surface;
    };
    let device_width = f64::from(surface.inner_width.max(1));
    let device_height = f64::from(surface.inner_height.max(1));
    let mut width = None;
    let mut height = None;
    let mut initial_scale = None;
    let mut minimum_scale = 0.1_f64;
    let mut maximum_scale = 10.0_f64;
    for directive in content.unwrap_or_default().split([',', ';']) {
        let Some((name, value)) = directive.split_once('=') else {
            continue;
        };
        match name.trim_ascii().to_ascii_lowercase().as_str() {
            "width" => width = device_length(value, device_width, device_height),
            "height" => height = device_length(value, device_width, device_height),
            "initial-scale" => initial_scale = scale_value(value),
            "minimum-scale" => {
                if let Some(scale) = scale_value(value) {
                    minimum_scale = scale;
                }
            }
            "maximum-scale" => {
                if let Some(scale) = scale_value(value) {
                    maximum_scale = scale;
                }
            }
            _ => {}
        }
    }
    let layout_width = width
        .or_else(|| initial_scale.map(|scale| device_width / scale))
        .unwrap_or(device_width.max(980.0));
    let page_scale = initial_scale
        .unwrap_or_else(|| (device_width / layout_width).min(1.0))
        .clamp(minimum_scale, maximum_scale.max(minimum_scale));
    let visual_width = device_width / page_scale;
    let visual_height = device_height / page_scale;
    surface.inner_width = layout_width.max(visual_width).ceil() as u32;
    surface.inner_height = height.unwrap_or_default().max(visual_height).ceil() as u32;
    view.scale = page_scale;
    view.width = visual_width;
    view.height = visual_height;
    surface
}

fn device_length(value: &str, device_width: f64, device_height: f64) -> Option<f64> {
    let value = value.trim_ascii();
    if value.eq_ignore_ascii_case("device-width") {
        Some(device_width)
    } else if value.eq_ignore_ascii_case("device-height") {
        Some(device_height)
    } else {
        positive_value(value).map(|length| length.clamp(1.0, 10_000.0))
    }
}

fn scale_value(value: &str) -> Option<f64> {
    positive_value(value.trim_ascii()).map(|scale| scale.clamp(0.1, 10.0))
}

fn positive_value(value: &str) -> Option<f64> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
}
