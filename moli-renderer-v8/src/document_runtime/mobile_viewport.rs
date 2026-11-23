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
    let mut initial_scale = None;
    let mut minimum_scale = 0.25_f64;
    let mut maximum_scale = 5.0_f64;
    for (name, value) in viewport_directives(content.unwrap_or_default()) {
        match name.to_ascii_lowercase().as_str() {
            "width" => width = device_length(value, device_width, device_height),
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
    let maximum_scale = maximum_scale.max(minimum_scale);
    let wide_viewport = width.unwrap_or(device_width.max(980.0));
    let scale = initial_scale
        .unwrap_or_else(|| (device_width / wide_viewport).min(1.0))
        .clamp(minimum_scale, maximum_scale);
    let scale = if initial_scale.is_some() && width.is_none() {
        let layout_width = (device_width / scale).floor().max(1.0);
        (device_width / layout_width).clamp(minimum_scale, maximum_scale)
    } else {
        scale
    } as f32;
    let visual_width = f64::from(device_width as f32 * scale.recip());
    let visual_height = f64::from(device_height as f32 * scale.recip());
    let layout_width = if initial_scale.is_some() {
        width.unwrap_or(visual_width).max(visual_width)
    } else {
        visual_width
    };
    let layout_height = (layout_width * device_height / device_width).max(visual_height);
    surface.inner_width = layout_width.ceil() as u32;
    surface.inner_height = layout_height.ceil() as u32;
    view.scale = f64::from(scale);
    view.width = visual_width;
    view.height = visual_height;
    surface
}

fn viewport_directives(mut content: &str) -> impl Iterator<Item = (&str, &str)> {
    let separator = |c: char| c.is_ascii_whitespace() || matches!(c, ',' | ';');
    std::iter::from_fn(move || {
        loop {
            content = content.trim_start_matches(separator);
            if content.is_empty() {
                return None;
            }
            let end = content
                .find(|c| c == '=' || separator(c))
                .unwrap_or(content.len());
            let (name, rest) = content.split_at(end);
            content = rest.trim_ascii_start();
            let Some(rest) = content.strip_prefix('=') else {
                continue;
            };
            content = rest.trim_ascii_start();
            let end = content.find(separator).unwrap_or(content.len());
            let (value, remaining) = content.split_at(end);
            content = remaining;
            return Some((name, value));
        }
    })
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
