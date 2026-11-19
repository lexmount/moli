use cssparser::color::clamp_unit_f32;
use style::color::{AbsoluteColor, ColorFlags, ColorSpace};
use style_traits::ToCss;

#[derive(Clone, Copy)]
pub(super) enum CanvasColorStyle {
    Fill,
    Stroke,
}

pub(super) fn serialize_canvas_color(mut color: AbsoluteColor) -> String {
    let missing_components = color.flags.intersects(
        ColorFlags::C0_IS_NONE
            | ColorFlags::C1_IS_NONE
            | ColorFlags::C2_IS_NONE
            | ColorFlags::ALPHA_IS_NONE,
    );
    if color.flags.contains(ColorFlags::IS_LEGACY_SRGB) && !missing_components {
        let [red, green, blue, alpha] = canvas_srgb_components(color).map(clamp_unit_f32);
        if color.alpha == f32::from(alpha) / 255.0 {
            return moli_canvas::serialize_srgb_color([red, green, blue, alpha]);
        }
        return format!("rgba({red}, {green}, {blue}, {})", color.alpha);
    }

    let alpha = color.alpha();
    // Use Stylo for the color components and serialize modern alpha at its
    // required six-decimal precision.
    color.alpha = 1.0;
    color
        .flags
        .remove(ColorFlags::IS_LEGACY_SRGB | ColorFlags::ALPHA_IS_NONE);
    let mut css =
        if missing_components && matches!(color.color_space, ColorSpace::Hsl | ColorSpace::Hwb) {
            let name = match color.color_space {
                ColorSpace::Hsl => "hsl",
                _ => "hwb",
            };
            let hue = color
                .c0()
                .map_or_else(|| "none".to_owned(), |hue| hue.to_css_string());
            let percentage = |component: Option<f32>| {
                component.map_or_else(
                    || "none".to_owned(),
                    |value| {
                        // Stylo stores the components here on a 0–100 scale.
                        // Remove noise from the conversion back to percentages.
                        let rounded = (f64::from(value) * 100_000.0).round() / 100_000.0;
                        format!("{rounded}%")
                    },
                )
            };
            format!(
                "{name}({hue} {} {})",
                percentage(color.c1()),
                percentage(color.c2())
            )
        } else {
            color.to_css_string()
        };

    if alpha != Some(1.0) {
        css.pop();
        css.push_str(" / ");
        if let Some(alpha) = alpha {
            let rounded = (f64::from(alpha) * 1_000_000.0).round() / 1_000_000.0;
            css.push_str(&rounded.to_string());
        } else {
            css.push_str("none");
        }
        css.push(')');
    }
    css
}

pub(super) fn canvas_srgb_components(color: AbsoluteColor) -> [f32; 4] {
    color
        .to_color_space(ColorSpace::Srgb)
        .raw_components()
        .map(|component| component.clamp(0.0, 1.0))
}
