use crate::css_style::top_level_comma_separated_component_values;

fn is_css_color_like(value: &str) -> bool {
    matches!(
        value,
        "red" | "blue" | "green" | "black" | "white" | "lightgreen" | "transparent"
    ) || (value.starts_with('#') && (value.len() == 4 || value.len() == 7))
        || (value.starts_with("rgb(") && value.ends_with(')'))
        || (value.starts_with("rgba(") && value.ends_with(')'))
}

pub(in crate::context_bootstrap::css_runtime::supports) fn is_legacy_webkit_background_image_like(
    value: &str,
) -> bool {
    is_legacy_webkit_radial_gradient_like(value) || is_legacy_webkit_gradient_like(value)
}

fn is_legacy_webkit_radial_gradient_like(value: &str) -> bool {
    let Some(inner) = value
        .strip_prefix("-webkit-radial-gradient(")
        .and_then(|value| value.strip_suffix(')'))
    else {
        return false;
    };
    let Some(parts) = top_level_comma_separated_component_values(inner) else {
        return false;
    };
    if parts.len() < 3 {
        return false;
    }
    let radii = parts[1].split_whitespace().collect::<Vec<_>>();
    radii.len() == 2
        && radii
            .iter()
            .all(|radius| css_non_negative_length_or_percentage(radius))
        && parts[2..]
            .iter()
            .all(|color| is_css_color_like(color.trim()))
}

fn is_legacy_webkit_gradient_like(value: &str) -> bool {
    let Some(inner) = value
        .strip_prefix("-webkit-gradient(")
        .and_then(|value| value.strip_suffix(')'))
    else {
        return false;
    };
    let Some(parts) = top_level_comma_separated_component_values(inner) else {
        return false;
    };
    matches!(
        parts.as_slice(),
        [kind, from, to, stop]
            if kind.trim() == "linear"
                && !from.trim().is_empty()
                && !to.trim().is_empty()
                && is_legacy_webkit_gradient_color_stop_like(stop.trim())
    )
}

fn is_legacy_webkit_gradient_color_stop_like(value: &str) -> bool {
    let Some(inner) = value
        .strip_prefix("color-stop(")
        .and_then(|value| value.strip_suffix(')'))
    else {
        return false;
    };
    let Some(parts) = top_level_comma_separated_component_values(inner) else {
        return false;
    };
    matches!(parts.as_slice(), [offset, color] if is_legacy_gradient_stop_offset_like(offset.trim()) && is_css_color_like(color.trim()))
}

fn is_legacy_gradient_stop_offset_like(value: &str) -> bool {
    if let Some(number) = moli_css_parse::parse_number(value) {
        return (0.0..=1.0).contains(&number);
    }
    if let Some(percent) = value
        .strip_suffix('%')
        .and_then(moli_css_parse::parse_number)
    {
        return (0.0..=100.0).contains(&percent);
    }
    value.starts_with("calc(") && value.ends_with(')')
}

fn css_non_negative_length_or_percentage(value: &str) -> bool {
    if value == "0" {
        return true;
    }
    if let Some(percent) = value
        .strip_suffix('%')
        .and_then(moli_css_parse::parse_number)
    {
        return percent >= 0.0;
    }
    for unit in [
        "px", "em", "rem", "in", "cm", "mm", "pt", "pc", "vh", "vw", "vmin", "vmax",
    ] {
        if let Some(number) = value
            .strip_suffix(unit)
            .and_then(moli_css_parse::parse_number)
        {
            return number >= 0.0;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_background_image_supports_legacy_webkit_radial_gradient() {
        assert!(is_legacy_webkit_background_image_like(
            "-webkit-radial-gradient(1px 2px, 3% 4%, red, blue)"
        ));
        assert!(is_legacy_webkit_background_image_like(
            "-webkit-radial-gradient(1px 2px, 0% 4%, red, blue)"
        ));
        assert!(!is_legacy_webkit_background_image_like(
            "-webkit-radial-gradient(1px 2px, 3% 4% red, blue)"
        ));
        assert!(!is_legacy_webkit_background_image_like(
            "-webkit-radial-gradient(1px 2px, -3% 4%, red, blue)"
        ));
    }

    #[test]
    fn css_background_image_supports_legacy_webkit_gradient_color_stop_calc() {
        assert!(is_legacy_webkit_background_image_like(
            "-webkit-gradient(linear, left top, left bottom, color-stop(calc(0.5 + 0.001 * sign(1em - 1px)), blue))"
        ));
        assert!(is_legacy_webkit_background_image_like(
            "-webkit-gradient(linear, left top, left bottom, color-stop(calc(50% + 0.001% * sign(1em - 1px)), blue))"
        ));
        assert!(!is_legacy_webkit_background_image_like(
            "-webkit-gradient(linear, left top, left bottom, color-stop(2, blue))"
        ));
    }
}
