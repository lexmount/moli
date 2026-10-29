use super::*;

/// CSSOM serializes the coordinated background layers, including initial
/// components. Specified-value shorthand serialization would omit those parts.
pub(super) fn computed_shorthand(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> String {
    let [
        Some(images),
        Some(repeats),
        Some(attachments),
        Some(positions_x),
        Some(positions_y),
        Some(sizes),
        Some(origins),
        Some(clips),
    ] = [
        "background-image",
        "background-repeat",
        "background-attachment",
        "background-position-x",
        "background-position-y",
        "background-size",
        "background-origin",
        "background-clip",
    ]
    .map(|property| {
        top_level_comma_separated_component_values(
            &resolution.computed_property(runtime, handle, property),
        )
        .filter(|values| !values.is_empty())
    })
    else {
        return String::new();
    };
    let color = resolution.computed_property(runtime, handle, "background-color");
    images
        .iter()
        .enumerate()
        .map(|(index, image)| {
            // The image list controls the layer count. Other lists repeat or
            // truncate to that count, and only the final layer carries color.
            let color = if index + 1 == images.len() {
                format!("{color} ")
            } else {
                String::new()
            };
            format!(
                "{color}{image} {} {} {} {} / {} {} {}",
                layer_value(&repeats, index),
                layer_value(&attachments, index),
                layer_value(&positions_x, index),
                layer_value(&positions_y, index),
                layer_value(&sizes, index),
                layer_value(&origins, index),
                layer_value(&clips, index),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn layer_value(values: &[String], index: usize) -> &str {
    &values[index % values.len()]
}
