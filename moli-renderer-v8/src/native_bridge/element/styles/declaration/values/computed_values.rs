use super::*;

pub(super) fn resolve_moli_computed_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> String {
    if property == "accent-color" && value.eq_ignore_ascii_case("auto") {
        return "auto".to_owned();
    }
    if color_property_is_resolved_color(property) {
        return normalize_computed_color(value);
    }
    if matches!(property, "box-shadow" | "text-shadow") {
        let current_color = resolution.computed_property(runtime, handle, "color");
        return normalize_computed_color_functions(value, Some(&current_color));
    }
    if property == "background-image" {
        return resolve_computed_background_image(runtime, handle, value);
    }
    if matches!(property, "transform" | "-webkit-transform")
        && let Some(transform) = computed_transform_matrix_value(value)
    {
        return transform;
    }
    if property == "animation-duration" {
        return resolve_computed_animation_duration(runtime, handle, value);
    }
    if property == "zoom"
        && let Some(zoom) =
            resolve_computed_zoom_with_resolution(runtime, handle, value, resolution)
    {
        return zoom;
    }
    if property == "font-family" {
        return normalize_cssom_font_family_value(value).unwrap_or_else(|| value.to_owned());
    }
    if matches!(property, "grid-template-columns" | "grid-template-rows")
        && let Some(tracks) = resolved_grid_template_tracks(runtime, handle, property)
    {
        return tracks;
    }
    if matches!(property, "width" | "height" | "inline-size" | "block-size")
        && let Some(size) = match resolution.observation {
            Some(observation) => observation.used_size(handle),
            None => used_size_from_layout_snapshot(runtime, handle),
        }
    {
        let value = match property {
            "width" => size.width,
            "height" => size.height,
            "inline-size" => size.inline_size,
            "block-size" => size.block_size,
            _ => unreachable!(),
        };
        return format_non_negative_used_css_px(f64::from(value));
    }
    if property == "width"
        && let Some(width) =
            resolve_computed_width_with_inline_fallback(runtime, handle, value, context, resolution)
    {
        return width;
    }
    if property == "height"
        && let Some(height) = resolve_computed_height_with_inline_fallback(
            runtime, handle, value, context, resolution,
        )
    {
        return height;
    }
    if property == "line-height"
        && let Some(line_height) =
            resolve_computed_line_height_with_resolution(runtime, handle, value, resolution)
    {
        return line_height;
    }
    // Horizontal used-value resolution recursively reads the containing block
    // and the element's own width from this exact retained observation.
    if matches!(property, "margin-left" | "margin-right")
        && let Some(margin) = resolve_computed_horizontal_auto_margin(
            runtime, handle, property, value, context, resolution,
        )
    {
        return margin;
    }
    if matches!(property, "margin-left" | "margin-right")
        && let Some(margin) = resolve_computed_horizontal_margin_with_inline_fallback(
            runtime, handle, property, value, context, resolution,
        )
    {
        return margin;
    }
    if matches!(property, "left" | "right" | "top" | "bottom")
        && let Some(inset) = resolve_computed_inset(runtime, handle, property, value, resolution)
    {
        return inset;
    }
    if matches!(property, "min-width" | "min-height") && (value.is_empty() || value == "auto") {
        return resolve_computed_auto_min_size(runtime, handle, resolution);
    }
    value.to_owned()
}

fn resolved_grid_template_tracks(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    if !runtime.layout_policy().uses_real_layout() {
        return None;
    }
    let grid = observable_used_grid_tracks(runtime, handle).ok()??;
    let tracks = match property {
        "grid-template-columns" => &grid.columns,
        "grid-template-rows" => &grid.rows,
        _ => return None,
    };

    serialize_used_grid_track_list(tracks)
}

fn serialize_used_grid_track_list(
    tracks: &moli_layout::LayoutResolvedGridTrackList,
) -> Option<String> {
    if tracks.track_count() != tracks.used_track_sizes.len() {
        return None;
    }
    if tracks.used_track_sizes.is_empty() {
        return Some("none".to_owned());
    }
    if tracks.explicit_line_names.len() != tracks.explicit_track_count.saturating_add(1) {
        return None;
    }
    let mut components = Vec::with_capacity(
        tracks.used_track_sizes.len().saturating_add(
            tracks
                .explicit_line_names
                .iter()
                .filter(|names| !names.is_empty())
                .count(),
        ),
    );
    let mut size_index = 0usize;
    for _ in 0..tracks.negative_implicit_track_count {
        components.push(format_non_negative_used_css_px(f64::from(
            *tracks.used_track_sizes.get(size_index)?,
        )));
        size_index += 1;
    }
    for (track_index, names) in tracks.explicit_line_names.iter().enumerate() {
        if !names.is_empty() {
            let mut serialized = String::from("[");
            for (index, name) in names.iter().enumerate() {
                if index > 0 {
                    serialized.push(' ');
                }
                serialize_identifier(name.as_ref(), &mut serialized)
                    .expect("serializing an identifier into String cannot fail");
            }
            serialized.push(']');
            components.push(serialized);
        }
        if track_index < tracks.explicit_track_count {
            components.push(format_non_negative_used_css_px(f64::from(
                *tracks.used_track_sizes.get(size_index)?,
            )));
            size_index += 1;
        }
    }
    for _ in 0..tracks.positive_implicit_track_count {
        components.push(format_non_negative_used_css_px(f64::from(
            *tracks.used_track_sizes.get(size_index)?,
        )));
        size_index += 1;
    }
    (size_index == tracks.used_track_sizes.len()).then(|| components.join(" "))
}

pub(super) fn computed_axis_position_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property_prefix: &str,
    context: StyleComputationContext,
) -> Option<String> {
    let axis_value = |property| {
        normalized_stylo_computed_style_value(runtime, handle, property, context)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| computed_style_default_value(runtime, handle, property))
    };
    let horizontal_property = format!("{property_prefix}position-x");
    let vertical_property = format!("{property_prefix}position-y");
    let horizontal = axis_value(&horizontal_property);
    let vertical = axis_value(&vertical_property);
    let horizontal_layers =
        top_level_comma_separated_component_values(&horizontal).unwrap_or_else(|| vec![horizontal]);
    let vertical_layers =
        top_level_comma_separated_component_values(&vertical).unwrap_or_else(|| vec![vertical]);
    if horizontal_layers.len() != vertical_layers.len() {
        return None;
    }
    Some(
        horizontal_layers
            .iter()
            .zip(vertical_layers.iter())
            .map(|(horizontal, vertical)| format!("{} {}", horizontal.trim(), vertical.trim()))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

pub(super) fn resolve_computed_animation_duration(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
) -> String {
    if !value
        .split(',')
        .any(|component| component.trim().eq_ignore_ascii_case("auto"))
    {
        return value.to_owned();
    }
    let initial_timeline = animation_timeline_is_initial_auto(runtime, handle);
    value
        .split(',')
        .map(|component| {
            let component = component.trim();
            if initial_timeline && component.eq_ignore_ascii_case("auto") {
                "0s"
            } else {
                component
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn resolve_computed_zoom_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("normal") {
        return Some("1".to_owned());
    }
    let zoom = resolve_css_zoom_numeric(
        value,
        css_numeric_context_with_viewport_and_resolution(
            runtime,
            handle,
            resolution.computation.viewport(),
            resolution,
        ),
    )?;
    if zoom < 0.0 {
        return None;
    }
    if zoom == 0.0 {
        return Some("1".to_owned());
    }
    Some(format_css_number(zoom))
}

fn resolve_css_zoom_numeric(
    value: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<f64> {
    moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::LengthPercentage {
            basis: 1.0,
            unitless: moli_css_parse::UnitlessLength::Any,
        },
        context,
    )
    .and_then(moli_css_parse::CssNumericValue::px_length)
    .or_else(|| {
        moli_css_parse::resolve_css_numeric(value, moli_css_parse::CssNumericKind::Number, context)
            .and_then(moli_css_parse::CssNumericValue::number)
    })
}

fn animation_timeline_is_initial_auto(runtime: &JsContextHost, handle: DomHandle) -> bool {
    let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "animation-timeline")
    else {
        return true;
    };
    let timelines = top_level_comma_separated_component_values(&entry.value)
        .unwrap_or_else(|| vec![entry.value]);
    matches!(
        timelines.as_slice(),
        [timeline] if timeline.trim().eq_ignore_ascii_case("auto")
    )
}

pub(in crate::native_bridge::element::styles) fn style_property_value_for_pseudo_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    pseudo_element: &str,
    property: &str,
    context: StyleComputationContext,
) -> String {
    if !computed_style_applies(runtime, handle) {
        return String::new();
    }
    let property = canonical_style_property_name(property);
    let value = normalized_stylo_computed_pseudo_style_value(
        runtime,
        handle,
        pseudo_element,
        &property,
        context,
    );
    match property.as_str() {
        "left" | "right" | "top" | "bottom" => value.unwrap_or_default(),
        "background-color" => value
            .map(|value| normalize_computed_color(&value))
            .unwrap_or_else(|| computed_style_default_value(runtime, handle, &property)),
        "accent-color" => value
            .map(|value| {
                if value.eq_ignore_ascii_case("auto") {
                    "auto".to_owned()
                } else {
                    normalize_computed_color(&value)
                }
            })
            .unwrap_or_else(|| computed_style_default_value(runtime, handle, &property)),
        "color" | "caret-color" | "outline-color" => value
            .map(|value| normalize_computed_color(&value))
            .unwrap_or_else(|| {
                inherited_computed_style_value(runtime, handle, &property, "rgb(0, 0, 0)")
            }),
        "width" => value
            .filter(|value| !value.is_empty())
            .map(|value| resolve_computed_pseudo_width(runtime, handle, &value))
            .unwrap_or_else(|| "auto".to_owned()),
        "height" | "min-width" | "min-height" => value
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "auto".to_owned()),
        "text-decoration-thickness" | "text-underline-offset" => value
            .filter(|value| !value.is_empty())
            .and_then(|value| {
                resolve_computed_text_decoration_length(runtime, handle, &property, &value)
            })
            .unwrap_or_else(|| computed_style_default_value(runtime, handle, &property)),
        "position" => value
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "static".to_owned()),
        "display" => value
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default_pseudo_display(runtime, handle, pseudo_element)),
        "content" => match value.as_deref() {
            Some("normal") | None => "none".to_owned(),
            Some(value) => value.to_owned(),
        },
        _ if color_property_is_resolved_color(&property) => value
            .map(|value| normalize_computed_color(&value))
            .unwrap_or_else(|| normalize_computed_color("currentcolor")),
        _ => value.unwrap_or_default(),
    }
}

fn resolve_computed_text_decoration_length(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
) -> Option<String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("auto")
        || (property == "text-decoration-thickness" && value.eq_ignore_ascii_case("from-font"))
    {
        return Some(value.to_ascii_lowercase());
    }
    resolve_computed_font_relative_length(runtime, handle, value)
        .or_else(|| parse_css_px(value).map(format_css_px))
}

fn resolve_computed_pseudo_width(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
) -> String {
    if !value.contains('%') {
        return value.to_owned();
    }
    let Some(basis) = computed_pseudo_width_basis(runtime, handle) else {
        return value.to_owned();
    };
    moli_css_parse::resolve_length_percentage(
        value,
        basis,
        moli_css_parse::UnitlessLength::ZeroOnly,
    )
    .map(format_css_px)
    .unwrap_or_else(|| value.to_owned())
}

fn computed_pseudo_width_basis(runtime: &JsContextHost, handle: DomHandle) -> Option<f64> {
    let width = style_property_value(runtime, handle, StyleMode::Computed, "width");
    if let Some(px) = width.strip_suffix("px")
        && let Some(width) = moli_css_parse::parse_number(px)
    {
        return Some(width);
    }
    let parent = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)?;
    runtime.dom_host().node(parent).and_then(Node::as_element)?;
    computed_pseudo_width_basis(runtime, parent)
}

fn default_pseudo_display(
    runtime: &JsContextHost,
    handle: DomHandle,
    pseudo_element: &str,
) -> String {
    if matches!(pseudo_element, "before" | "after") {
        let parent_display = style_property_value(runtime, handle, StyleMode::Computed, "display");
        if matches!(
            parent_display.as_str(),
            "flex" | "inline-flex" | "grid" | "inline-grid"
        ) {
            return "block".to_owned();
        }
    }
    "inline".to_owned()
}

pub(super) fn border_width_property_index(property: &str) -> Option<usize> {
    Some(match property {
        "border-width" => 4,
        "border-top-width" => 0,
        "border-right-width" => 1,
        "border-bottom-width" => 2,
        "border-left-width" => 3,
        _ => return None,
    })
}

pub(super) fn border_width_longhands() -> &'static [&'static str] {
    &[
        "border-top-width",
        "border-right-width",
        "border-bottom-width",
        "border-left-width",
    ]
}

pub(super) fn border_width_from_shorthand(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    border_width_property_index(property)?;
    inline_style_entry_for_inline_style(runtime, handle, "border")
        .and_then(|entry| border_shorthand_width(&entry.value))
}

pub(super) fn border_component_from_component_shorthand(
    runtime: &JsContextHost,
    handle: DomHandle,
    shorthand: &str,
    index: usize,
) -> Option<String> {
    inline_style_entry_for_inline_style(runtime, handle, shorthand)
        .and_then(|entry| box_shorthand_component(&entry.value, index))
}

pub(super) fn border_component_style_entry_from_component_shorthand(
    runtime: &JsContextHost,
    handle: DomHandle,
    shorthand: &str,
    longhand: &str,
    index: usize,
) -> Option<StyleEntry> {
    let entry = inline_style_entry_for_inline_style(runtime, handle, shorthand)?;
    let value = box_shorthand_component(&entry.value, index)?;
    Some(StyleEntry {
        name: longhand.to_owned(),
        value,
        priority: entry.priority,
    })
}

pub(super) fn border_color_property(property: &str) -> bool {
    matches!(
        property,
        "border-color"
            | "border-top-color"
            | "border-right-color"
            | "border-bottom-color"
            | "border-left-color"
            | "border-block-start-color"
            | "border-block-end-color"
            | "border-inline-start-color"
            | "border-inline-end-color"
    )
}

pub(super) fn border_color_property_index(property: &str) -> Option<usize> {
    Some(match property {
        "border-top-color" => 0,
        "border-right-color" => 1,
        "border-bottom-color" => 2,
        "border-left-color" => 3,
        _ => return None,
    })
}

pub(super) fn border_color_longhands() -> &'static [&'static str] {
    &[
        "border-top-color",
        "border-right-color",
        "border-bottom-color",
        "border-left-color",
    ]
}

pub(super) fn border_color_from_shorthand(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    inline_style_entry_for_inline_style(runtime, handle, "border")
        .and_then(|entry| border_shorthand_color(&entry.value))
}

pub(super) fn border_style_property(property: &str) -> bool {
    matches!(
        property,
        "border-style"
            | "border-top-style"
            | "border-right-style"
            | "border-bottom-style"
            | "border-left-style"
    )
}

pub(super) fn border_style_property_index(property: &str) -> Option<usize> {
    Some(match property {
        "border-top-style" => 0,
        "border-right-style" => 1,
        "border-bottom-style" => 2,
        "border-left-style" => 3,
        _ => return None,
    })
}

pub(super) fn border_style_longhands() -> &'static [&'static str] {
    &[
        "border-top-style",
        "border-right-style",
        "border-bottom-style",
        "border-left-style",
    ]
}

pub(super) fn border_style_from_shorthand(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    inline_style_entry_for_inline_style(runtime, handle, "border")
        .and_then(|entry| border_shorthand_style(&entry.value))
}

pub(super) fn color_property_is_resolved_color(property: &str) -> bool {
    property == "color"
        || property == "accent-color"
        || property == "background-color"
        || property == "caret-color"
        || property == "outline-color"
        || property == "text-decoration-color"
        || property == "text-emphasis-color"
        || property == "-webkit-text-fill-color"
        || property == "-webkit-text-stroke-color"
        || border_color_property(property)
}

fn resolve_computed_inset(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if let Some(length) = resolve_computed_font_relative_length(runtime, handle, value) {
        return Some(length);
    }
    if let Some(length) =
        resolve_computed_inset_length_percentage(runtime, handle, property, value, resolution)
    {
        return Some(length);
    }
    resolve_computed_auto_inset(runtime, handle, property, value, resolution)
}

fn resolve_computed_auto_min_size(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> String {
    if display_none_ancestor(runtime, handle, resolution) {
        return "0px".to_owned();
    }
    let aspect_ratio = resolution.raw_property(runtime, handle, "aspect-ratio");
    if !aspect_ratio.is_empty() && aspect_ratio != "auto" {
        return "auto".to_owned();
    }
    if let Some(parent) = flat_tree_element_parent(runtime, handle) {
        let parent_display = resolution.computed_property(runtime, parent, "display");
        if matches!(
            parent_display.as_str(),
            "flex" | "inline-flex" | "grid" | "inline-grid"
        ) {
            return "auto".to_owned();
        }
    }
    "0px".to_owned()
}

fn display_none_ancestor(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> bool {
    let mut current = Some(handle);
    while let Some(candidate) = current {
        if resolution.computed_property(runtime, candidate, "display") == "none" {
            return true;
        }
        current = flat_tree_element_parent(runtime, candidate);
    }
    false
}

fn resolve_computed_font_relative_length(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    let (raw_number, font_size) = if let Some(number) = value.strip_suffix("rem") {
        (
            number,
            runtime
                .dom_host()
                .document_element_handle()
                .and_then(|document_element| computed_font_size_px(runtime, document_element))
                .unwrap_or(16.0),
        )
    } else if let Some(number) = value.strip_suffix("em") {
        (
            number,
            computed_font_size_px(runtime, handle).unwrap_or(16.0),
        )
    } else {
        return None;
    };
    let multiplier = moli_css_parse::parse_number(raw_number)?;
    Some(format_css_px(multiplier * font_size))
}

fn resolve_computed_inset_length_percentage(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    let value = value.trim();
    if !value.contains('%') && !value.to_ascii_lowercase().starts_with("calc(") {
        return None;
    }
    let position = computed_position_with_resolution(runtime, handle, resolution);
    let basis = match position.as_str() {
        "relative" | "sticky" | "absolute" | "fixed" => {
            computed_inset_containing_block_size(runtime, handle, &position, property)?
        }
        _ => return None,
    };
    let resolved = moli_css_parse::resolve_length_percentage(
        value,
        basis,
        moli_css_parse::UnitlessLength::ZeroOnly,
    )?;
    Some(format_css_px(resolved))
}

fn computed_position(runtime: &JsContextHost, handle: DomHandle) -> String {
    let read = ComputedStyleRead::new(runtime, handle);
    computed_position_with_resolution(runtime, handle, read.resolution_context())
}

fn computed_position_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> String {
    let position = resolution.computed_property(runtime, handle, "position");
    if position.is_empty() {
        "static".to_owned()
    } else {
        position
    }
}

pub(super) fn logical_inset_style_entry(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<StyleEntry> {
    let logical = physical_inset_logical_source(runtime, handle, property)?;
    inline_style_entry_for_inline_style(runtime, handle, logical)
}

pub(super) fn inset_shorthand_style_entry(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<StyleEntry> {
    let shorthand_index = match property {
        "top" => 0,
        "right" => 1,
        "bottom" => 2,
        "left" => 3,
        _ => return None,
    };
    let mut entry = inline_style_entry_for_inline_style(runtime, handle, "inset")?;
    entry.name = property.to_owned();
    entry.value = box_shorthand_component(&entry.value, shorthand_index)?;
    Some(entry)
}

fn physical_inset_logical_source(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<&'static str> {
    if !matches!(property, "left" | "right") {
        return None;
    }
    let writing_mode = raw_stylo_computed_style_value(runtime, handle, "writing-mode");
    if !writing_mode.is_empty() && writing_mode != "horizontal-tb" {
        return None;
    }
    let direction = computed_direction(runtime, handle);
    match (property, direction.as_str()) {
        ("left", "rtl") | ("right", "ltr") => Some("inset-inline-end"),
        ("left", _) | ("right", _) => Some("inset-inline-start"),
        _ => None,
    }
}

fn computed_direction(runtime: &JsContextHost, handle: DomHandle) -> String {
    let read = ComputedStyleRead::new(runtime, handle);
    computed_direction_with_resolution(runtime, handle, read.resolution_context())
}

pub(super) fn computed_direction_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> String {
    if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "direction") {
        let value = entry.value.to_ascii_lowercase();
        if matches!(value.as_str(), "ltr" | "rtl") {
            return value;
        }
    }
    let direction = resolution.raw_property(runtime, handle, "direction");
    let direction = direction.to_ascii_lowercase();
    if matches!(direction.as_str(), "ltr" | "rtl") {
        return direction;
    }
    html_directionality(runtime.dom_host(), handle)
        .as_str()
        .to_owned()
}

pub(super) fn raw_stylo_computed_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> String {
    ComputedStyleRead::new(runtime, handle)
        .raw_primary_property(property)
        .unwrap_or_default()
}

pub(super) fn raw_stylo_computed_style_value_with_inputs(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    inputs: &FullStyleWorldSnapshot,
    context: StyleComputationContext,
) -> String {
    let read_document = context.resolved_read_document(runtime, handle);
    runtime
        .computed_style_snapshot_from_stylo_after_style_update(
            handle,
            inputs,
            read_document,
            context.viewport,
        )
        .and_then(|style| style.property_value(property))
        .unwrap_or_default()
}

pub(super) fn raw_stylo_computed_style_value_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    context: StyleComputationContext,
) -> Option<String> {
    ComputedStyleRead::new_with_context(runtime, handle, context).raw_primary_property(property)
}

fn computed_inset_containing_block_size(
    runtime: &JsContextHost,
    handle: DomHandle,
    position: &str,
    property: &str,
) -> Option<f64> {
    let containing_block = computed_inset_containing_block(runtime, handle, position)?;
    let rect = computed_style_geometry_rect(runtime, containing_block)?;
    let vertical = matches!(property, "top" | "bottom");
    let mut size = if vertical { rect.height } else { rect.width };
    if position == "absolute"
        && !vertical
        && let Some(inline_width) = inline_text_containing_block_width(runtime, containing_block)
        && inline_width > size
    {
        size = inline_width;
    }
    if matches!(position, "relative" | "sticky") {
        size -= computed_box_component_px(
            runtime,
            containing_block,
            if vertical {
                "padding-top"
            } else {
                "padding-left"
            },
            "padding",
            if vertical { 0 } else { 3 },
        );
        size -= computed_box_component_px(
            runtime,
            containing_block,
            if vertical {
                "padding-bottom"
            } else {
                "padding-right"
            },
            "padding",
            if vertical { 2 } else { 1 },
        );
    }
    Some(size)
}

fn inline_text_containing_block_width(
    runtime: &JsContextHost,
    containing_block: DomHandle,
) -> Option<f64> {
    let display = style_property_value(runtime, containing_block, StyleMode::Computed, "display");
    if display != "inline" {
        return None;
    }
    let text = runtime.dom_host().text_content(containing_block)?;
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    let font_size = computed_font_size_px(runtime, containing_block).unwrap_or(16.0);
    Some((collapsed.chars().count() as f64 + 1.0) * font_size)
}

fn computed_inset_containing_block(
    runtime: &JsContextHost,
    handle: DomHandle,
    position: &str,
) -> Option<DomHandle> {
    match position {
        "absolute" => flat_ancestor_chain(runtime, handle)
            .into_iter()
            .find(|ancestor| {
                !matches!(
                    computed_position(runtime, *ancestor).as_str(),
                    "static" | ""
                )
            }),
        "fixed" => flat_ancestor_chain(runtime, handle)
            .into_iter()
            .find(|ancestor| {
                let transform =
                    style_property_value(runtime, *ancestor, StyleMode::Computed, "transform");
                !transform.is_empty() && transform != "none"
            }),
        "sticky" => flat_ancestor_chain(runtime, handle)
            .into_iter()
            .find(|ancestor| {
                let overflow =
                    style_property_value(runtime, *ancestor, StyleMode::Computed, "overflow");
                !matches!(overflow.as_str(), "" | "visible" | "clip")
            })
            .or_else(|| runtime.dom_host().node(handle).and_then(Node::parent_node)),
        _ => runtime.dom_host().node(handle).and_then(Node::parent_node),
    }
}

fn flat_ancestor_chain(runtime: &JsContextHost, handle: DomHandle) -> Vec<DomHandle> {
    let mut ancestors = Vec::new();
    let mut current = runtime.dom_host().node(handle).and_then(Node::parent_node);
    while let Some(handle) = current {
        ancestors.push(handle);
        current = runtime.dom_host().node(handle).and_then(Node::parent_node);
    }
    ancestors
}

fn computed_box_component_px(
    runtime: &JsContextHost,
    handle: DomHandle,
    longhand: &str,
    shorthand: &str,
    shorthand_index: usize,
) -> f64 {
    parse_css_px(&style_property_value(
        runtime,
        handle,
        StyleMode::Computed,
        longhand,
    ))
    .or_else(|| {
        let shorthand = style_property_value(runtime, handle, StyleMode::Computed, shorthand);
        box_shorthand_component(&shorthand, shorthand_index).and_then(|value| parse_css_px(&value))
    })
    .unwrap_or(0.0)
}

fn resolve_computed_auto_inset(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if !value.is_empty() && !value.eq_ignore_ascii_case("auto") {
        return None;
    }
    let position = computed_position_with_resolution(runtime, handle, resolution);
    if !matches!(position.as_str(), "relative" | "absolute" | "fixed") {
        return None;
    }
    if (value.is_empty() || value.eq_ignore_ascii_case("auto"))
        && position == "absolute"
        && property == "left"
        && (computed_inset_containing_block(runtime, handle, &position).is_some_and(|container| {
            style_property_value(runtime, container, StyleMode::Computed, "display") == "grid"
        }) || grid_column_is_positioned(runtime, handle))
    {
        return Some("0px".to_owned());
    }
    let opposite = opposite_inset_property(property)?;
    let opposite_value = resolution.raw_property(runtime, handle, opposite);
    if value.is_empty()
        && position != "relative"
        && (opposite_value.is_empty() || opposite_value.eq_ignore_ascii_case("auto"))
    {
        return None;
    }
    if opposite_value.is_empty() || opposite_value.eq_ignore_ascii_case("auto") {
        return Some(resolve_computed_both_auto_inset(
            runtime, handle, property, &position,
        ));
    }
    let opposite = parse_css_px(&opposite_value)
        .or_else(|| {
            resolve_computed_font_relative_length(runtime, handle, &opposite_value)
                .and_then(|value| parse_css_px(&value))
        })
        .or_else(|| {
            resolve_computed_inset_length_percentage(
                runtime,
                handle,
                opposite,
                &opposite_value,
                resolution,
            )
            .and_then(|value| parse_css_px(&value))
        })?;
    if position == "relative" {
        return Some(format_css_px(-opposite));
    }
    let containing_block_size =
        computed_inset_containing_block_size(runtime, handle, &position, property)?;
    let own_size = computed_auto_inset_own_size(runtime, handle, property).unwrap_or(0.0);
    Some(format_css_px(containing_block_size - opposite - own_size))
}

fn grid_column_is_positioned(runtime: &JsContextHost, handle: DomHandle) -> bool {
    ["grid-column-start", "grid-column-end"]
        .into_iter()
        .map(|property| raw_stylo_computed_style_value(runtime, handle, property))
        .any(|value| !value.is_empty() && value != "auto")
}

fn opposite_inset_property(property: &str) -> Option<&'static str> {
    match property {
        "top" => Some("bottom"),
        "right" => Some("left"),
        "bottom" => Some("top"),
        "left" => Some("right"),
        _ => None,
    }
}

fn resolve_computed_both_auto_inset(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    position: &str,
) -> String {
    if position == "relative" {
        return "0px".to_owned();
    }
    let Some(containing_block) = computed_inset_containing_block(runtime, handle, position) else {
        return "0px".to_owned();
    };
    let Some(rect) = computed_style_geometry_rect(runtime, containing_block) else {
        return "0px".to_owned();
    };
    let containing_block_size = if matches!(property, "top" | "bottom") {
        rect.height
    } else {
        rect.width
    };
    let static_offset = static_position_offset(runtime, containing_block, property, position);
    if inset_uses_start_static_position(runtime, containing_block, property) {
        format_css_px(static_offset)
    } else {
        format_css_px(containing_block_size - static_offset)
    }
}

fn static_position_offset(
    runtime: &JsContextHost,
    containing_block: DomHandle,
    property: &str,
    position: &str,
) -> f64 {
    if let Some(offset) = runtime_static_position_offset(runtime, containing_block, property) {
        return offset;
    }
    match (position, matches!(property, "top" | "bottom")) {
        ("fixed", _) => 0.0,
        (_, true) => 15.0,
        (_, false) => 30.0,
    }
}

fn runtime_static_position_offset(
    runtime: &JsContextHost,
    containing_block: DomHandle,
    property: &str,
) -> Option<f64> {
    let parent = runtime
        .dom_host()
        .node(containing_block)
        .and_then(Node::parent_node)?;
    let rects = observable_bounding_client_rects(runtime, &[parent, containing_block]).ok()?;
    let [parent_rect, containing_rect] = rects.as_slice() else {
        return None;
    };
    Some(if matches!(property, "top" | "bottom") {
        (containing_rect.top - parent_rect.top).abs()
    } else {
        (containing_rect.left - parent_rect.left).abs()
    })
    .filter(|offset| *offset > 0.0)
}

fn computed_style_geometry_rect(runtime: &JsContextHost, handle: DomHandle) -> Option<ClientRect> {
    read_bounding_client_rect(runtime, handle).ok()
}

fn inset_uses_start_static_position(
    runtime: &JsContextHost,
    containing_block: DomHandle,
    property: &str,
) -> bool {
    let writing_mode = style_property_value(
        runtime,
        containing_block,
        StyleMode::Computed,
        "writing-mode",
    );
    let direction = computed_direction(runtime, containing_block);
    let writing_mode = if writing_mode.is_empty() {
        "horizontal-tb"
    } else {
        writing_mode.as_str()
    };
    let direction = if direction == "rtl" { "rtl" } else { "ltr" };
    match (writing_mode, direction, property) {
        ("horizontal-tb", _, "top") => true,
        ("horizontal-tb", _, "bottom") => false,
        ("horizontal-tb", "ltr", "left") => true,
        ("horizontal-tb", "ltr", "right") => false,
        ("horizontal-tb", "rtl", "left") => false,
        ("horizontal-tb", "rtl", "right") => true,
        ("vertical-lr", _, "left") => true,
        ("vertical-lr", _, "right") => false,
        ("vertical-rl", _, "left") => false,
        ("vertical-rl", _, "right") => true,
        ("vertical-lr" | "vertical-rl", "ltr", "top") => true,
        ("vertical-lr" | "vertical-rl", "ltr", "bottom") => false,
        ("vertical-lr" | "vertical-rl", "rtl", "top") => false,
        ("vertical-lr" | "vertical-rl", "rtl", "bottom") => true,
        (_, _, "top" | "left") => true,
        _ => false,
    }
}

fn computed_auto_inset_own_size(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<f64> {
    let size_property = if matches!(property, "top" | "bottom") {
        "height"
    } else {
        "width"
    };
    let value = style_property_value(runtime, handle, StyleMode::Computed, size_property);
    parse_css_px(&value).or_else(|| {
        resolve_computed_font_relative_length(runtime, handle, &value)
            .and_then(|value| parse_css_px(&value))
    })
}

fn resolve_computed_horizontal_auto_margin(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if !value.eq_ignore_ascii_case("auto") {
        return None;
    }
    let parent_width =
        containing_block_width_with_resolution(runtime, handle, context, resolution, 0)?;
    let own_width = parse_css_px(&resolution.computed_property(runtime, handle, "width"))?;
    let other_property = if property == "margin-left" {
        "margin-right"
    } else {
        "margin-left"
    };
    let other = resolution.raw_property(runtime, handle, other_property);
    let other_margin = if other.eq_ignore_ascii_case("auto") {
        None
    } else {
        parse_css_px(&other).or(Some(0.0))
    };
    let available = parent_width - own_width - other_margin.unwrap_or(0.0);
    let resolved = if other_margin.is_none() {
        available / 2.0
    } else {
        available
    };
    Some(format_css_px(resolved))
}

fn resolve_computed_horizontal_margin(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if value.eq_ignore_ascii_case("auto") {
        return None;
    }
    let parent_width =
        containing_block_width_with_resolution(runtime, handle, context, resolution, 0)?;
    let resolved = resolve_length_percentage_with_context(
        value,
        parent_width,
        css_numeric_context_with_viewport_and_resolution(
            runtime,
            handle,
            context.viewport(),
            resolution,
        ),
    )?;
    Some(format_css_px(resolved))
}

fn resolve_computed_horizontal_margin_with_inline_fallback(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if computed_length_percentage_value_needs_moli_context(value) {
        inline_style_entry_for_inline_style(runtime, handle, property)
            .and_then(|entry| {
                resolve_computed_horizontal_margin(
                    runtime,
                    handle,
                    &entry.value,
                    context,
                    resolution,
                )
            })
            .or_else(|| {
                resolve_computed_horizontal_margin(runtime, handle, value, context, resolution)
            })
    } else {
        resolve_computed_horizontal_margin(runtime, handle, value, context, resolution).or_else(
            || {
                inline_style_entry_for_inline_style(runtime, handle, property).and_then(|entry| {
                    resolve_computed_horizontal_margin(
                        runtime,
                        handle,
                        &entry.value,
                        context,
                        resolution,
                    )
                })
            },
        )
    }
}

fn resolve_computed_line_height_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("normal") || parse_css_px(value).is_some() {
        return None;
    }
    let font_size =
        computed_font_size_px_with_resolution(runtime, handle, resolution).unwrap_or(16.0);
    if let Some(percent) = parse_css_percent(value) {
        return Some(format_css_px(font_size * percent / 100.0));
    }
    let multiplier = moli_css_parse::parse_number(value)?;
    Some(format_css_px(font_size * multiplier))
}

pub(super) fn computed_line_height_px_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    let value = resolution.raw_property(runtime, handle, "line-height");
    if let Some(px) = parse_css_px(&value) {
        return Some(px);
    }
    resolve_computed_line_height_with_resolution(runtime, handle, &value, resolution)
        .and_then(|value| parse_css_px(&value))
}

fn computed_font_size_px(runtime: &JsContextHost, handle: DomHandle) -> Option<f64> {
    let read = ComputedStyleRead::new(runtime, handle);
    computed_font_size_px_with_resolution(runtime, handle, read.resolution_context())
}

pub(super) fn inline_font_size_px(runtime: &JsContextHost, handle: DomHandle) -> Option<f64> {
    parse_css_px(&style_property_value(
        runtime,
        handle,
        StyleMode::Inline,
        "font-size",
    ))
}

pub(super) fn computed_font_size_px_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    computed_font_size_px_with_resolution_and_depth(runtime, handle, resolution, 0)
}

fn computed_font_size_px_with_resolution_and_depth(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
    depth: usize,
) -> Option<f64> {
    if depth > 32 {
        return None;
    }
    parse_css_px(&resolution.computed_property(runtime, handle, "font-size")).or_else(|| {
        inherited_style_parent(runtime, handle)
            .filter(|parent| *parent != handle)
            .and_then(|parent| {
                computed_font_size_px_with_resolution_and_depth(
                    runtime,
                    parent,
                    resolution,
                    depth + 1,
                )
            })
    })
}

fn resolve_computed_width(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if element_has_no_used_width(runtime, handle, resolution) {
        return None;
    }
    let viewport = context.viewport();
    let parent_width =
        containing_block_width_with_resolution(runtime, handle, context, resolution, 0)?;
    let resolved = resolve_length_percentage_with_context(
        value,
        parent_width,
        css_numeric_context_with_viewport_and_resolution(runtime, handle, viewport, resolution),
    )?;
    Some(format_non_negative_used_css_px(resolved))
}

fn resolve_computed_width_with_inline_fallback(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if computed_length_percentage_value_needs_moli_context(value) {
        inline_style_entry_for_inline_style(runtime, handle, "width")
            .and_then(|entry| {
                resolve_computed_width(runtime, handle, &entry.value, context, resolution)
            })
            .or_else(|| resolve_computed_width(runtime, handle, value, context, resolution))
    } else {
        resolve_computed_width(runtime, handle, value, context, resolution).or_else(|| {
            inline_style_entry_for_inline_style(runtime, handle, "width").and_then(|entry| {
                resolve_computed_width(runtime, handle, &entry.value, context, resolution)
            })
        })
    }
}

fn element_has_no_used_width(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> bool {
    matches!(
        resolution
            .computed_property(runtime, handle, "display")
            .as_str(),
        "none" | "contents" | "inline"
    )
}

fn element_has_no_used_height(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> bool {
    matches!(
        resolution
            .computed_property(runtime, handle, "display")
            .as_str(),
        "none" | "contents" | "inline"
    )
}

fn containing_block_width_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
    depth: usize,
) -> Option<f64> {
    if depth > 32 {
        return None;
    }
    let parent = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)?;
    if let Some(width) = raw_computed_width_px(runtime, parent, resolution) {
        return Some(width);
    }
    let parent_is_viewport_box = runtime.dom_host().node(parent).is_some_and(|node| {
        node.is_html_element_named("body") || node.is_html_element_named("html")
    });
    if let Some(percent) = raw_computed_width_percent(runtime, parent, resolution) {
        let viewport_width = context.viewport_width();
        let parent_parent_width = if parent_is_viewport_box {
            viewport_width
                .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_width)
        } else {
            containing_block_width_with_resolution(runtime, parent, context, resolution, depth + 1)?
        };
        return Some(parent_parent_width * percent / 100.0);
    }
    if parent_is_viewport_box {
        return Some(
            context
                .viewport_width()
                .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_width),
        );
    }
    containing_block_width_with_resolution(runtime, parent, context, resolution, depth + 1)
}

fn resolve_computed_height(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if element_has_no_used_height(runtime, handle, resolution) {
        return None;
    }
    if value.eq_ignore_ascii_case("auto") {
        return None;
    }
    let viewport = context.viewport();
    let parent_height = containing_block_height(runtime, handle, context, resolution, 0)?;
    let resolved = resolve_length_percentage_with_context(
        value,
        parent_height,
        css_numeric_context_with_viewport_and_resolution(runtime, handle, viewport, resolution),
    )?;
    Some(format_non_negative_used_css_px(resolved))
}

fn resolve_computed_height_with_inline_fallback(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if computed_length_percentage_value_needs_moli_context(value) {
        inline_style_entry_for_inline_style(runtime, handle, "height")
            .and_then(|entry| {
                resolve_computed_height(runtime, handle, &entry.value, context, resolution)
            })
            .or_else(|| resolve_computed_height(runtime, handle, value, context, resolution))
    } else {
        resolve_computed_height(runtime, handle, value, context, resolution).or_else(|| {
            inline_style_entry_for_inline_style(runtime, handle, "height").and_then(|entry| {
                resolve_computed_height(runtime, handle, &entry.value, context, resolution)
            })
        })
    }
}

fn computed_length_percentage_value_needs_moli_context(value: &str) -> bool {
    value.contains('%')
}

fn resolve_length_percentage_with_context(
    value: &str,
    basis: f64,
    context: moli_css_parse::CssNumericContext,
) -> Option<f64> {
    moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::LengthPercentage {
            basis,
            unitless: moli_css_parse::UnitlessLength::ZeroOnly,
        },
        context,
    )?
    .px_length()
}

fn containing_block_height(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
    resolution: StyleResolutionContext<'_>,
    depth: usize,
) -> Option<f64> {
    if depth > 32 {
        return None;
    }
    let parent = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)?;
    if let Some(height) = raw_computed_height_px(runtime, parent, resolution) {
        return Some(height);
    }
    let parent_is_viewport_box = runtime.dom_host().node(parent).is_some_and(|node| {
        node.is_html_element_named("body") || node.is_html_element_named("html")
    });
    if let Some(percent) = raw_computed_height_percent(runtime, parent, resolution) {
        let viewport_height = context
            .viewport()
            .height
            .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_height);
        let parent_parent_height = if parent_is_viewport_box {
            viewport_height
        } else {
            containing_block_height(runtime, parent, context, resolution, depth + 1)?
        };
        return Some(parent_parent_height * percent / 100.0);
    }
    if parent_is_viewport_box {
        return Some(
            context
                .viewport()
                .height
                .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_height),
        );
    }
    containing_block_height(runtime, parent, context, resolution, depth + 1)
}

fn raw_computed_width_px(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    parse_css_px(&resolution.raw_property(runtime, handle, "width"))
}

fn raw_computed_width_percent(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    parse_css_percent(&resolution.raw_property(runtime, handle, "width"))
}

fn raw_computed_height_px(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    parse_css_px(&resolution.raw_property(runtime, handle, "height"))
}

fn raw_computed_height_percent(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    parse_css_percent(&resolution.raw_property(runtime, handle, "height"))
}

fn parse_css_px(value: &str) -> Option<f64> {
    let value = value.trim();
    if value == "0" {
        return Some(0.0);
    }
    value
        .strip_suffix("px")?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn parse_css_percent(value: &str) -> Option<f64> {
    value
        .trim()
        .strip_suffix('%')?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

pub(super) fn format_css_px(value: f64) -> String {
    format!("{}px", format_css_numeric_literal(value))
}

/// Matches Blink's `CSSNumericLiteralValue` serialization, which uses `%g`
/// with six significant digits for finite non-integer dimensions.
fn format_css_numeric_literal(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    if !value.is_finite() {
        return value.to_string();
    }

    // Formatting in scientific notation first gives us both the six-digit
    // rounding and the post-rounding exponent without reimplementing floating
    // point decimal conversion.
    let scientific = format!("{value:.5e}");
    let (mantissa, exponent) = scientific
        .rsplit_once('e')
        .expect("finite f64 scientific formatting must contain an exponent");
    let exponent = exponent
        .parse::<i32>()
        .expect("f64 scientific formatting must contain a decimal exponent");
    if (-4..6).contains(&exponent) {
        let fractional_digits = usize::try_from((5 - exponent).max(0)).unwrap_or(0);
        let mut serialized = format!("{value:.fractional_digits$}");
        trim_decimal_zeros(&mut serialized);
        return serialized;
    }

    let mut mantissa = mantissa.to_owned();
    trim_decimal_zeros(&mut mantissa);
    format!("{mantissa}e{exponent:+03}")
}

fn trim_decimal_zeros(serialized: &mut String) {
    if !serialized.contains('.') {
        return;
    }
    while serialized.ends_with('0') {
        serialized.pop();
    }
    if serialized.ends_with('.') {
        serialized.pop();
    }
}

fn format_non_negative_used_css_px(value: f64) -> String {
    if value.is_nan() || value.is_sign_negative() {
        return "0px".to_owned();
    }
    if value == f64::INFINITY {
        return format!("{}px", i64::MAX);
    }
    format_css_px(value)
}

fn resolve_computed_background_image(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
) -> String {
    resolve_background_image_url(value, &style_base_url(runtime, handle))
}

fn resolve_background_image_url(value: &str, base_url: &url::Url) -> String {
    resolve_css_url_function(value, base_url)
}

pub(super) fn compress_box_shorthand_value(value: &str) -> String {
    box_shorthand_value_components(value)
        .and_then(|values| compress_box_components(&values))
        .unwrap_or_else(|| value.to_owned())
}

pub(super) fn compress_box_components(values: &[String]) -> Option<String> {
    match values {
        [start, end] if start == end => Some(start.clone()),
        [start, end] => Some(format!("{start} {end}")),
        [top, right, bottom, left] if top == right && top == bottom && top == left => {
            Some(top.clone())
        }
        [top, right, bottom, left] if top == bottom && right == left => {
            Some(format!("{top} {right}"))
        }
        [top, right, bottom, left] if right == left => Some(format!("{top} {right} {bottom}")),
        [top, right, bottom, left] => Some(format!("{top} {right} {bottom} {left}")),
        _ => None,
    }
}

fn normalize_computed_color(value: &str) -> String {
    let value = value.trim();
    if let Some((red, green, blue)) = system_color_rgb(value) {
        return format!("rgb({red}, {green}, {blue})");
    }
    if let Some((red, green, blue)) = css_named_color_rgb(value) {
        return format!("rgb({red}, {green}, {blue})");
    }
    if value.eq_ignore_ascii_case("currentcolor") {
        return "rgb(0, 0, 0)".to_owned();
    }
    if value.eq_ignore_ascii_case("transparent") {
        return "rgba(0, 0, 0, 0)".to_owned();
    }
    if let Some((red, green, blue)) = css_hex_color_rgb(value) {
        return format!("rgb({red}, {green}, {blue})");
    }
    value.to_owned()
}

pub(super) fn resolve_computed_color_property_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> String {
    if value.eq_ignore_ascii_case("currentcolor") && property != "color" {
        return resolution.computed_property(runtime, handle, "color");
    }
    normalize_computed_color(value)
}

pub(super) fn normalize_computed_color_functions(
    value: &str,
    current_color: Option<&str>,
) -> String {
    top_level_comma_separated_component_values(value)
        .map(|layers| {
            layers
                .into_iter()
                .map(|layer| normalize_computed_color_function_layer(&layer, current_color))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| normalize_computed_color_function_layer(value, current_color))
}

fn normalize_computed_color_function_layer(value: &str, current_color: Option<&str>) -> String {
    let Some(components) = box_shorthand_value_components(value) else {
        return value.to_owned();
    };
    let mut color_component_index = None;
    let mut normalized = components
        .into_iter()
        .enumerate()
        .map(|(index, component)| {
            system_color_rgb(&component)
                .map(|(red, green, blue)| {
                    color_component_index.get_or_insert(index);
                    format!("rgb({red}, {green}, {blue})")
                })
                .or_else(|| {
                    component.eq_ignore_ascii_case("currentcolor").then(|| {
                        color_component_index.get_or_insert(index);
                        current_color.unwrap_or(&component).to_owned()
                    })
                })
                .unwrap_or(component)
        })
        .collect::<Vec<_>>();
    if let Some(index) = color_component_index
        && index > 0
        && index < normalized.len()
    {
        let color = normalized.remove(index);
        normalized.insert(0, color);
    }
    normalized.join(" ")
}

pub(super) fn css_hex_color_rgb(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.strip_prefix('#')?;
    if !hex.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    match hex.len() {
        3 => {
            let mut chars = hex.chars();
            let red = chars.next()?.to_digit(16)? as u8;
            let green = chars.next()?.to_digit(16)? as u8;
            let blue = chars.next()?.to_digit(16)? as u8;
            Some((red * 17, green * 17, blue * 17))
        }
        6 => {
            let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some((red, green, blue))
        }
        _ => None,
    }
}

pub(super) fn css_named_color_rgb(value: &str) -> Option<(u8, u8, u8)> {
    Some(match value.to_ascii_lowercase().as_str() {
        "aliceblue" => (240, 248, 255),
        "antiquewhite" => (250, 235, 215),
        "aqua" => (0, 255, 255),
        "aquamarine" => (127, 255, 212),
        "azure" => (240, 255, 255),
        "beige" => (245, 245, 220),
        "bisque" => (255, 228, 196),
        "black" => (0, 0, 0),
        "blanchedalmond" => (255, 235, 205),
        "blue" => (0, 0, 255),
        "blueviolet" => (138, 43, 226),
        "brown" => (165, 42, 42),
        "burlywood" => (222, 184, 135),
        "cadetblue" => (95, 158, 160),
        "chartreuse" => (127, 255, 0),
        "chocolate" => (210, 105, 30),
        "coral" => (255, 127, 80),
        "cornflowerblue" => (100, 149, 237),
        "cornsilk" => (255, 248, 220),
        "crimson" => (220, 20, 60),
        "cyan" => (0, 255, 255),
        "darkblue" => (0, 0, 139),
        "darkcyan" => (0, 139, 139),
        "darkgoldenrod" => (184, 134, 11),
        "darkgray" | "darkgrey" => (169, 169, 169),
        "darkgreen" => (0, 100, 0),
        "darkkhaki" => (189, 183, 107),
        "darkmagenta" => (139, 0, 139),
        "darkolivegreen" => (85, 107, 47),
        "darkorange" => (255, 140, 0),
        "darkorchid" => (153, 50, 204),
        "darkred" => (139, 0, 0),
        "darksalmon" => (233, 150, 122),
        "darkseagreen" => (143, 188, 143),
        "darkslateblue" => (72, 61, 139),
        "darkslategray" | "darkslategrey" => (47, 79, 79),
        "darkturquoise" => (0, 206, 209),
        "darkviolet" => (148, 0, 211),
        "deeppink" => (255, 20, 147),
        "deepskyblue" => (0, 191, 255),
        "dimgray" | "dimgrey" => (105, 105, 105),
        "dodgerblue" => (30, 144, 255),
        "firebrick" => (178, 34, 34),
        "floralwhite" => (255, 250, 240),
        "forestgreen" => (34, 139, 34),
        "fuchsia" => (255, 0, 255),
        "gainsboro" => (220, 220, 220),
        "ghostwhite" => (248, 248, 255),
        "gold" => (255, 215, 0),
        "goldenrod" => (218, 165, 32),
        "gray" | "grey" => (128, 128, 128),
        "green" => (0, 128, 0),
        "greenyellow" => (173, 255, 47),
        "honeydew" => (240, 255, 240),
        "hotpink" => (255, 105, 180),
        "indianred" => (205, 92, 92),
        "indigo" => (75, 0, 130),
        "ivory" => (255, 255, 240),
        "khaki" => (240, 230, 140),
        "lavender" => (230, 230, 250),
        "lavenderblush" => (255, 240, 245),
        "lawngreen" => (124, 252, 0),
        "lemonchiffon" => (255, 250, 205),
        "lightblue" => (173, 216, 230),
        "lightcoral" => (240, 128, 128),
        "lightcyan" => (224, 255, 255),
        "lightgoldenrodyellow" => (250, 250, 210),
        "lightgray" | "lightgrey" => (211, 211, 211),
        "lightgreen" => (144, 238, 144),
        "lightpink" => (255, 182, 193),
        "lightsalmon" => (255, 160, 122),
        "lightseagreen" => (32, 178, 170),
        "lightskyblue" => (135, 206, 250),
        "lightslategray" | "lightslategrey" => (119, 136, 153),
        "lightsteelblue" => (176, 196, 222),
        "lightyellow" => (255, 255, 224),
        "lime" => (0, 255, 0),
        "limegreen" => (50, 205, 50),
        "linen" => (250, 240, 230),
        "magenta" => (255, 0, 255),
        "maroon" => (128, 0, 0),
        "mediumaquamarine" => (102, 205, 170),
        "mediumblue" => (0, 0, 205),
        "mediumorchid" => (186, 85, 211),
        "mediumpurple" => (147, 112, 219),
        "mediumseagreen" => (60, 179, 113),
        "mediumslateblue" => (123, 104, 238),
        "mediumspringgreen" => (0, 250, 154),
        "mediumturquoise" => (72, 209, 204),
        "mediumvioletred" => (199, 21, 133),
        "midnightblue" => (25, 25, 112),
        "mintcream" => (245, 255, 250),
        "mistyrose" => (255, 228, 225),
        "moccasin" => (255, 228, 181),
        "navajowhite" => (255, 222, 173),
        "navy" => (0, 0, 128),
        "oldlace" => (253, 245, 230),
        "olive" => (128, 128, 0),
        "olivedrab" => (107, 142, 35),
        "orange" => (255, 165, 0),
        "orangered" => (255, 69, 0),
        "orchid" => (218, 112, 214),
        "palegoldenrod" => (238, 232, 170),
        "palegreen" => (152, 251, 152),
        "paleturquoise" => (175, 238, 238),
        "palevioletred" => (219, 112, 147),
        "papayawhip" => (255, 239, 213),
        "peachpuff" => (255, 218, 185),
        "peru" => (205, 133, 63),
        "pink" => (255, 192, 203),
        "plum" => (221, 160, 221),
        "powderblue" => (176, 224, 230),
        "purple" => (128, 0, 128),
        "rebeccapurple" => (102, 51, 153),
        "red" => (255, 0, 0),
        "rosybrown" => (188, 143, 143),
        "royalblue" => (65, 105, 225),
        "saddlebrown" => (139, 69, 19),
        "salmon" => (250, 128, 114),
        "sandybrown" => (244, 164, 96),
        "seagreen" => (46, 139, 87),
        "seashell" => (255, 245, 238),
        "sienna" => (160, 82, 45),
        "silver" => (192, 192, 192),
        "skyblue" => (135, 206, 235),
        "slateblue" => (106, 90, 205),
        "slategray" | "slategrey" => (112, 128, 144),
        "snow" => (255, 250, 250),
        "springgreen" => (0, 255, 127),
        "steelblue" => (70, 130, 180),
        "tan" => (210, 180, 140),
        "teal" => (0, 128, 128),
        "thistle" => (216, 191, 216),
        "tomato" => (255, 99, 71),
        "turquoise" => (64, 224, 208),
        "violet" => (238, 130, 238),
        "wheat" => (245, 222, 179),
        "white" => (255, 255, 255),
        "whitesmoke" => (245, 245, 245),
        "yellow" => (255, 255, 0),
        "yellowgreen" => (154, 205, 50),
        _ => return None,
    })
}
