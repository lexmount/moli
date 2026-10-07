use super::*;

pub(in crate::native_bridge::element::styles) fn style_property_value_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    property: &str,
    context: StyleComputationContext,
) -> String {
    if mode == StyleMode::Computed {
        let Some(property) = canonical_computed_cssom_query_property_name(property) else {
            return String::new();
        };
        return computed_style_property_value_with_context(runtime, handle, &property, context);
    }
    style_property_value_with_viewport_width(
        runtime,
        handle,
        mode,
        property,
        context.viewport_width(),
    )
}

pub(super) fn computed_style_property_value_with_context(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    context: StyleComputationContext,
) -> String {
    ComputedStyleRead::new_with_context(runtime, handle, context).property(property)
}

pub(super) fn computed_style_property_value_after_style_update(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    context: StyleComputationContext,
    prepared_inputs: Option<&FullStyleWorldSnapshot>,
    observation: Option<&dyn RetainedStyleObservation>,
    prepared_style: Option<&StyloComputedStyleSnapshot>,
) -> String {
    if !computed_style_applies(runtime, handle) {
        return String::new();
    }
    if property == "display"
        && element_hidden_attribute_state(runtime, handle) == HiddenAttributeState::Hidden
    {
        return "none".to_owned();
    }
    let inputs = prepared_inputs;
    let resolution = if let Some(observation) = observation {
        StyleResolutionContext::observed(context, inputs, observation, handle, prepared_style)
    } else if let Some(style) = prepared_style {
        if let Some(inputs) = inputs {
            StyleResolutionContext::retained(context, inputs, handle, style)
        } else {
            StyleResolutionContext::retained_without_inputs(context, handle, style)
        }
    } else if let Some(inputs) = inputs {
        StyleResolutionContext::prepared(context, inputs)
    } else {
        StyleResolutionContext::independent(context)
    };
    if property == "direction" {
        return computed_direction_with_resolution(runtime, handle, resolution);
    }
    if matches!(property, "left" | "right" | "top" | "bottom")
        && let Some(midpoint) =
            active_css_animation_midpoint_px_with_resolution(runtime, handle, property, resolution)
    {
        return format!("{midpoint}px");
    }
    if property == "transform"
        && let Some(transform) =
            active_css_animation_transform_value_with_resolution(runtime, handle, resolution)
    {
        return transform;
    }
    if color_property_is_resolved_color(property)
        && let Some(value) =
            active_css_animation_static_value(runtime, handle, property, resolution)
    {
        return resolve_computed_color_property_value(
            runtime, handle, property, &value, resolution,
        );
    }
    if property == "animation" {
        return computed_animation_shorthand_value(runtime, handle, context);
    }
    if property == "animation-range" {
        return computed_animation_range_shorthand_value(runtime, handle, context);
    }
    if property == "transition" {
        return computed_transition_shorthand_value(runtime, handle, context);
    }
    if property == "flex-flow" {
        let direction =
            computed_style_property_value_with_context(runtime, handle, "flex-direction", context);
        let wrap =
            computed_style_property_value_with_context(runtime, handle, "flex-wrap", context);
        return format!("{direction} {wrap}");
    }
    if property == "text-decoration" {
        return computed_text_decoration_shorthand_value(runtime, handle, context);
    }
    if property == "text-emphasis" {
        return computed_text_emphasis_shorthand_value(runtime, handle, context);
    }
    if property == "font-variant" {
        return computed_font_variant_shorthand_value(runtime, handle, resolution);
    }
    if property == "mask" {
        return computed_mask_shorthand_value(runtime, handle, resolution);
    }
    if property == "border" {
        return computed_border_shorthand_value(runtime, handle, context);
    }
    if let Some(side) = border_side_shorthand_property(property) {
        return computed_border_side_shorthand_value(runtime, handle, side, context);
    }
    if property == "-webkit-text-stroke" {
        return computed_webkit_text_stroke_shorthand_value(runtime, handle, context);
    }
    if matches!(property, "text-decoration-fill" | "text-decoration-stroke")
        && let Some(value) = computed_text_decoration_paint_value(runtime, handle, property)
    {
        return value;
    }
    if matches!(
        property,
        "animation-timing-function" | "transition-timing-function"
    ) && let Some(value) = computed_timing_function_list_value(runtime, handle, property)
    {
        return value;
    }
    if matches!(property, "animation-range-start" | "animation-range-end")
        && let Some(value) = computed_animation_range_endpoint_value(runtime, handle, property)
    {
        return value;
    }
    if property == "text-decoration-line"
        && let Some(value) = computed_inline_text_decoration_line_value(runtime, handle)
    {
        return value;
    }
    if matches!(property, "background-position" | "mask-position")
        && let Some(value) = computed_axis_position_shorthand_value(
            runtime,
            handle,
            property.strip_suffix("position").unwrap_or_default(),
            context,
        )
    {
        return value;
    }
    if css_numeric_computed_property_rule(property).is_some()
        && let Some(value) = computed_css_numeric_property_value(runtime, handle, property)
    {
        return value;
    }
    if property == "zoom"
        && let Some(value) = computed_inline_zoom_value(runtime, handle, resolution)
    {
        return value;
    }

    let raw_value = if let Some(style) = prepared_style {
        style.resolved_property_value(property).and_then(|value| {
            normalize_stylo_computed_style_value_with_resolution(
                runtime, handle, property, &value, resolution,
            )
        })
    } else if let Some(inputs) = inputs {
        normalized_stylo_computed_style_value_with_inputs(
            runtime, handle, property, context, inputs,
        )
    } else {
        None
    }
    .or_else(|| computed_style_property_value_from_moli(runtime, handle, property));
    let value = raw_value
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| computed_style_default_value(runtime, handle, property));
    let value = shadow_tree_inherited_value_for_initial_stylo_value(
        runtime, handle, property, &value, resolution,
    )
    .unwrap_or(value);
    if let Some(value) = active_registered_length_custom_property_animation_value(
        runtime, handle, property, &value, resolution,
    ) {
        return value;
    }
    let value = inputs
        .map(|inputs| {
            resolve_computed_custom_function_calls(
                runtime, handle, property, &value, inputs, context,
            )
        })
        .unwrap_or(value);
    resolve_moli_computed_style_value(runtime, handle, property, &value, context, resolution)
}

pub(super) fn computed_property_requires_stylesheet_sources(
    property: &str,
    style: Option<&StyloComputedStyleSnapshot>,
) -> bool {
    property.starts_with("--")
        && style
            .and_then(|style| style.property_value(property))
            .is_some_and(|value| !dashed_no_arg_function_calls(&value).is_empty())
}

pub(super) fn computed_style_property_value_with_prepared_inputs(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    inputs: &FullStyleWorldSnapshot,
    context: StyleComputationContext,
    prepared_style: Option<&StyloComputedStyleSnapshot>,
) -> String {
    computed_style_property_value_after_style_update(
        runtime,
        handle,
        property,
        context,
        Some(inputs),
        None,
        prepared_style,
    )
}

pub(super) fn computed_transform_matrix_value(value: &str) -> Option<String> {
    if value.trim().eq_ignore_ascii_case("none") {
        return None;
    }
    moli_geometry::parse_dom_matrix_value(value)?.css_text()
}

fn computed_style_property_value_from_moli(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    match property {
        "animation-range-start" | "animation-range-end" => {
            computed_animation_range_endpoint_value(runtime, handle, property)
        }
        "animation-delay"
        | "animation-duration"
        | "animation-iteration-count"
        | "transition-delay"
        | "transition-duration" => computed_css_numeric_property_value(runtime, handle, property),
        "animation-timing-function" | "transition-timing-function" => {
            computed_timing_function_list_value(runtime, handle, property)
        }
        "bookmark-level" => Some(computed_non_inherited_css_keyword_property_value(
            runtime, handle, property, "none",
        )),
        "bookmark-state" => Some(computed_non_inherited_css_keyword_property_value(
            runtime, handle, property, "open",
        )),
        "color-scheme" => Some(computed_inherited_css_keyword_property_value(
            runtime, handle, property, "normal",
        )),
        "forced-color-adjust" => Some(computed_inherited_css_keyword_property_value(
            runtime, handle, property, "auto",
        )),
        "print-color-adjust" => Some(computed_inherited_css_keyword_property_value(
            runtime, handle, property, "economy",
        )),
        "quotes" => Some(computed_inherited_css_keyword_property_value(
            runtime, handle, property, "auto",
        )),
        "scrollbar-color" => Some(computed_inherited_css_keyword_property_value(
            runtime, handle, property, "auto",
        )),
        "scrollbar-width" => Some(computed_non_inherited_css_keyword_property_value(
            runtime, handle, property, "auto",
        )),
        "link-parameters" => Some(computed_non_inherited_css_keyword_property_value(
            runtime, handle, property, "none",
        )),
        "text-size-adjust" => Some(computed_text_size_adjust_value(runtime, handle)),
        "transition-property" | "transition-behavior" => {
            inline_style_entry_for_inline_style(runtime, handle, property).map(|entry| entry.value)
        }
        _ => None,
    }
}

fn computed_inherited_css_keyword_property_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    initial: &str,
) -> String {
    let Some(value) = inline_style_property_value_for_inline_style(runtime, handle, property)
    else {
        return inherited_computed_style_value(runtime, handle, property, initial);
    };
    match css_wide_keyword(&value).as_deref() {
        Some("inherit") | Some("unset") => {
            inherited_computed_style_value(runtime, handle, property, initial)
        }
        Some(_) => initial.to_owned(),
        None => value,
    }
}

fn computed_text_size_adjust_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    let Some(value) =
        inline_style_property_value_for_inline_style(runtime, handle, "text-size-adjust")
    else {
        return inherited_computed_style_value(runtime, handle, "text-size-adjust", "auto");
    };
    match css_wide_keyword(&value).as_deref() {
        Some("inherit") | Some("unset") => {
            inherited_computed_style_value(runtime, handle, "text-size-adjust", "auto")
        }
        Some(_) => "auto".to_owned(),
        None => computed_text_size_adjust_specified_value(runtime, handle, &value).unwrap_or(value),
    }
}

pub(super) fn computed_text_size_adjust_specified_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    value: &str,
) -> Option<String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("auto") {
        return Some("auto".to_owned());
    }
    if value.eq_ignore_ascii_case("none") {
        return Some("100%".to_owned());
    }
    let percent =
        resolve_css_percentage_only_with_context(value, css_numeric_context(runtime, handle))?;
    (percent >= 0.0).then(|| format_css_percent(percent))
}

fn computed_non_inherited_css_keyword_property_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    initial: &str,
) -> String {
    let Some(value) = inline_style_property_value_for_inline_style(runtime, handle, property)
    else {
        return initial.to_owned();
    };
    match css_wide_keyword(&value).as_deref() {
        Some("inherit") => inherited_computed_style_value(runtime, handle, property, initial),
        Some(_) => initial.to_owned(),
        None => value,
    }
}

fn computed_text_emphasis_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> String {
    let style =
        computed_style_property_value_with_context(runtime, handle, "text-emphasis-style", context);
    let color =
        computed_style_property_value_with_context(runtime, handle, "text-emphasis-color", context);
    format!("{style} {color}")
}

fn computed_font_variant_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> String {
    let values = font_variant_longhands()
        .iter()
        .map(|property| resolution.computed_property(runtime, handle, property))
        .collect::<Vec<_>>();
    serialize_font_variant_shorthand_values(&values).unwrap_or_default()
}

fn computed_mask_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> String {
    const LONGHANDS: [&str; 9] = [
        "mask-mode",
        "mask-repeat",
        "mask-clip",
        "mask-origin",
        "mask-composite",
        "mask-position-x",
        "mask-position-y",
        "mask-size",
        "mask-image",
    ];

    let mut block = moli_css_parse::CssDeclarationBlock::default();
    for property in LONGHANDS {
        let value = resolution.computed_property(runtime, handle, property);
        if value.is_empty()
            || block.set_property(property, &value, false)
                == moli_css_parse::CssSetResult::ParseError
        {
            return String::new();
        }
    }
    block.property_value("mask").unwrap_or_default()
}

fn computed_webkit_text_stroke_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> String {
    let width = computed_style_property_value_with_context(
        runtime,
        handle,
        "-webkit-text-stroke-width",
        context,
    );
    let color = computed_style_property_value_with_context(
        runtime,
        handle,
        "-webkit-text-stroke-color",
        context,
    );
    format!("{width} {color}")
}

fn computed_text_decoration_paint_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    let entry = inline_style_entry_for_inline_style(runtime, handle, property)?;
    match css_wide_keyword(&entry.value).as_deref() {
        Some("inherit") => Some(inherited_computed_style_value(
            runtime,
            handle,
            property,
            "match-text",
        )),
        Some(_) => Some("match-text".to_owned()),
        None => Some(entry.value),
    }
}

pub(super) fn inline_text_decoration_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> String {
    let Some((entries, shared_css_wide_keyword)) =
        inline_shorthand_entries(runtime, handle, text_decoration_shorthand_longhands(), &[])
    else {
        return String::new();
    };
    if let Some(keyword) = shared_css_wide_keyword {
        return keyword;
    }
    serialize_text_decoration_shorthand(
        &entries[0].value,
        &entries[1].value,
        &entries[2].value,
        &entries[3].value,
        text_decoration_value_is_currentcolor(&entries[3].value),
    )
}

fn computed_text_decoration_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> String {
    let line = computed_style_property_value_with_context(
        runtime,
        handle,
        "text-decoration-line",
        context,
    );
    let thickness = computed_style_property_value_with_context(
        runtime,
        handle,
        "text-decoration-thickness",
        context,
    );
    let style = computed_style_property_value_with_context(
        runtime,
        handle,
        "text-decoration-style",
        context,
    );
    let color = computed_style_property_value_with_context(
        runtime,
        handle,
        "text-decoration-color",
        context,
    );
    let color_is_currentcolor =
        computed_text_decoration_color_is_currentcolor(runtime, handle, context);
    serialize_text_decoration_shorthand(&line, &thickness, &style, &color, color_is_currentcolor)
}

fn serialize_text_decoration_shorthand(
    line: &str,
    thickness: &str,
    style: &str,
    color: &str,
    color_is_currentcolor: bool,
) -> String {
    let line = text_decoration_component_or_initial(line, "none");
    let thickness = text_decoration_component_or_initial(thickness, "auto");
    let style = text_decoration_component_or_initial(style, "solid");
    let defaults =
        line == "none" && thickness == "auto" && style == "solid" && color_is_currentcolor;

    let mut values = Vec::new();
    if defaults || line != "none" {
        values.push(line);
    }
    if thickness != "auto" {
        values.push(thickness);
    }
    if style != "solid" {
        values.push(style);
    }
    if !color_is_currentcolor {
        values.push(text_decoration_component_or_initial(color, "currentcolor"));
    }
    values.join(" ")
}

fn computed_text_decoration_color_is_currentcolor(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> bool {
    inline_style_entry_for_inline_style(runtime, handle, "text-decoration-color")
        .map(|entry| text_decoration_value_is_currentcolor(&entry.value))
        .or_else(|| {
            raw_stylo_computed_style_value_with_context(
                runtime,
                handle,
                "text-decoration-color",
                context,
            )
            .map(|value| text_decoration_value_is_currentcolor(&value))
        })
        .unwrap_or(true)
}

fn text_decoration_value_is_currentcolor(value: &str) -> bool {
    text_decoration_component_or_initial(value, "currentcolor").eq_ignore_ascii_case("currentcolor")
}

fn text_decoration_component_or_initial<'a>(value: &'a str, initial: &'static str) -> &'a str {
    let value = value.trim();
    if value.is_empty() || css_wide_keyword(value).is_some() {
        initial
    } else {
        value
    }
}

fn computed_inline_text_decoration_line_value(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    let entry = inline_style_entry_for_inline_style(runtime, handle, "text-decoration-line")?;
    match css_wide_keyword(&entry.value).as_deref() {
        Some("inherit") => Some(inherited_computed_style_value(
            runtime,
            handle,
            "text-decoration-line",
            "none",
        )),
        Some(_) => Some("none".to_owned()),
        None => Some(entry.value),
    }
}

fn computed_inline_zoom_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    let entry = inline_style_entry_for_inline_style(runtime, handle, "zoom")?;
    match css_wide_keyword(&entry.value).as_deref() {
        Some("inherit") => Some(inherited_computed_style_value(runtime, handle, "zoom", "1")),
        Some(_) => Some("1".to_owned()),
        None => resolve_computed_zoom_with_resolution(runtime, handle, &entry.value, resolution),
    }
}

fn computed_animation_range_endpoint_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    let entry = inline_style_entry_for_inline_style(runtime, handle, property)?;
    let context = css_numeric_context(runtime, handle);
    let kind = match property {
        "animation-range-start" => AnimationRangeEndpointKind::Start,
        "animation-range-end" => AnimationRangeEndpointKind::End,
        _ => return None,
    };
    let values = top_level_comma_separated_component_values(&entry.value)
        .unwrap_or_else(|| vec![entry.value]);
    let resolved = values
        .into_iter()
        .map(|value| computed_single_animation_range_endpoint(&value, kind, context))
        .collect::<Option<Vec<_>>>()?;
    (!resolved.is_empty()).then(|| resolved.join(", "))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AnimationRangeEndpointKind {
    Start,
    End,
}

fn computed_single_animation_range_endpoint(
    value: &str,
    kind: AnimationRangeEndpointKind,
    context: moli_css_parse::CssNumericContext,
) -> Option<String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("normal") {
        return Some("normal".to_owned());
    }
    let lowered = value.to_ascii_lowercase();
    let (name, offset) = if let Some(name) = animation_range_name_prefix(&lowered) {
        let offset = value[name.len()..].trim();
        (Some(name), offset)
    } else {
        (None, value)
    };
    if offset.is_empty() {
        return name.map(str::to_owned);
    }
    let serialized_offset = computed_animation_range_offset(offset, context)?;
    Some(match name {
        Some(name) if animation_range_offset_is_default_for_endpoint(&serialized_offset, kind) => {
            name.to_owned()
        }
        Some(name) => format!("{name} {serialized_offset}"),
        None => serialized_offset,
    })
}

fn animation_range_offset_is_default_for_endpoint(
    offset: &str,
    kind: AnimationRangeEndpointKind,
) -> bool {
    match kind {
        AnimationRangeEndpointKind::Start => offset == "0%",
        AnimationRangeEndpointKind::End => offset == "100%",
    }
}

fn computed_animation_range_offset(
    value: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<String> {
    if let Some(percent) = resolve_css_percentage_only(value) {
        return Some(format_css_percent(percent));
    }
    if let Some(px) = resolve_css_length_only(value, context) {
        return Some(format_css_px(px));
    }
    if value.contains('%') {
        return Some(value.to_owned());
    }
    let px = moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::LengthPercentage {
            basis: 0.0,
            unitless: moli_css_parse::UnitlessLength::ZeroOnly,
        },
        context,
    )?
    .px_length()?;
    Some(format_css_px(px))
}

fn resolve_css_percentage_only(value: &str) -> Option<f64> {
    resolve_css_percentage_only_with_context(
        value,
        moli_css_parse::CssNumericContext::supports_probe(),
    )
}

fn resolve_css_percentage_only_with_context(
    value: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<f64> {
    let value = value.trim();
    if !value.contains('%') {
        return None;
    }
    let basis_100 = moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::LengthPercentage {
            basis: 100.0,
            unitless: moli_css_parse::UnitlessLength::ZeroOnly,
        },
        context,
    )?
    .px_length()?;
    let basis_200 = moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::LengthPercentage {
            basis: 200.0,
            unitless: moli_css_parse::UnitlessLength::ZeroOnly,
        },
        context,
    )?
    .px_length()?;
    ((basis_200 - (basis_100 * 2.0)).abs() < 1e-9).then_some(basis_100)
}

fn resolve_css_length_only(value: &str, context: moli_css_parse::CssNumericContext) -> Option<f64> {
    if value.contains('%') {
        return None;
    }
    moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::PxLength(moli_css_parse::UnitlessLength::ZeroOnly),
        context,
    )?
    .px_length()
}

fn computed_css_numeric_property_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    let rule = css_numeric_computed_property_rule(property)?;
    let entry = inline_style_entry_for_inline_style(runtime, handle, property)?;
    let context = css_numeric_context(runtime, handle);
    let values = top_level_comma_separated_component_values(&entry.value)
        .unwrap_or_else(|| vec![entry.value]);
    let resolved = values
        .into_iter()
        .map(|value| match rule {
            CssNumericComputedPropertyRule::TimeList { non_negative } => {
                let seconds = moli_css_parse::resolve_css_numeric(
                    &value,
                    moli_css_parse::CssNumericKind::Time,
                    context,
                )?
                .time_seconds()?;
                (!non_negative || seconds >= 0.0).then(|| format_css_seconds(seconds))
            }
            CssNumericComputedPropertyRule::AnimationDurationList => {
                if value.eq_ignore_ascii_case("auto") {
                    return Some(resolve_computed_animation_duration(runtime, handle, &value));
                }
                let seconds = moli_css_parse::resolve_css_numeric(
                    &value,
                    moli_css_parse::CssNumericKind::Time,
                    context,
                )?
                .time_seconds()?;
                (seconds >= 0.0).then(|| format_css_seconds(seconds))
            }
            CssNumericComputedPropertyRule::AnimationIterationCountList => {
                if value.eq_ignore_ascii_case("infinite") {
                    return Some("infinite".to_owned());
                }
                let number = moli_css_parse::resolve_css_numeric(
                    &value,
                    moli_css_parse::CssNumericKind::Number,
                    context,
                )?
                .number()?;
                (number >= 0.0).then(|| format_css_number(number))
            }
        })
        .collect::<Option<Vec<_>>>()?;
    (!resolved.is_empty()).then(|| resolved.join(", "))
}

#[derive(Clone, Copy)]
enum CssNumericComputedPropertyRule {
    TimeList { non_negative: bool },
    AnimationDurationList,
    AnimationIterationCountList,
}

fn css_numeric_computed_property_rule(property: &str) -> Option<CssNumericComputedPropertyRule> {
    Some(match property {
        "animation-delay" | "transition-delay" => CssNumericComputedPropertyRule::TimeList {
            non_negative: false,
        },
        "animation-duration" => CssNumericComputedPropertyRule::AnimationDurationList,
        "transition-duration" => CssNumericComputedPropertyRule::TimeList { non_negative: true },
        "animation-iteration-count" => CssNumericComputedPropertyRule::AnimationIterationCountList,
        _ => return None,
    })
}

fn css_numeric_context(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> moli_css_parse::CssNumericContext {
    let read = ComputedStyleRead::new(runtime, handle);
    css_numeric_context_with_viewport_and_resolution(
        runtime,
        handle,
        read.context.viewport(),
        read.resolution_context(),
    )
}

pub(super) fn css_numeric_context_with_viewport_and_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    viewport: StyleViewport,
    resolution: StyleResolutionContext<'_>,
) -> moli_css_parse::CssNumericContext {
    let width = nearest_size_container_width(runtime, handle, resolution).unwrap_or(100.0);
    let font_size = inline_font_size_px(runtime, handle)
        .or_else(|| computed_font_size_px_with_resolution(runtime, handle, resolution))
        .unwrap_or(16.0);
    let root_font_size = runtime
        .dom_host()
        .owner_document_handle(handle)
        .and_then(|document| {
            runtime
                .dom_host()
                .document_element_handle_for_document(document)
        })
        .and_then(|document_element| {
            inline_font_size_px(runtime, document_element).or_else(|| {
                computed_font_size_px_with_resolution(runtime, document_element, resolution)
            })
        })
        .unwrap_or(16.0);
    let line_height =
        computed_line_height_px_with_resolution(runtime, handle, resolution).unwrap_or(font_size);
    let viewport_width = viewport
        .width
        .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_width);
    let viewport_height = viewport
        .height
        .unwrap_or(moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_height);
    moli_css_parse::CssNumericContext {
        container_lengths: Some(moli_css_parse::ContainerQueryLengthContext {
            width_px: width,
            height_px: width,
            inline_size_px: width,
            block_size_px: width,
        }),
        font_size_px: Some(font_size),
        root_font_size_px: Some(root_font_size),
        line_height_px: Some(line_height),
        viewport_width_px: Some(viewport_width),
        viewport_height_px: Some(viewport_height),
        // Stylo 0.20 resolves tree-counting functions lazily from its computed
        // value context. Do not duplicate that work for every renderer numeric
        // context, most of which never contains sibling-index()/sibling-count().
        sibling_index: None,
        sibling_count: None,
    }
}

fn nearest_size_container_width(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    let mut current = flat_tree_element_parent(runtime, handle);
    let mut visited = HashSet::new();
    while let Some(candidate) = current {
        if !visited.insert(candidate) {
            return None;
        }
        if element_is_size_container(runtime, candidate, resolution) {
            return inline_width_px_with_resolution(runtime, candidate, resolution);
        }
        current = flat_tree_element_parent(runtime, candidate);
    }
    None
}

fn element_is_size_container(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> bool {
    let container = resolution.computed_property(runtime, handle, "container");
    if container
        .split_once('/')
        .is_some_and(|(_, ty)| container_type_is_size_container(ty))
    {
        return true;
    }
    let ty = resolution.computed_property(runtime, handle, "container-type");
    container_type_is_size_container(&ty)
}

fn format_css_seconds(seconds: f64) -> String {
    format!("{}s", format_css_number(seconds))
}

fn format_css_percent(percent: f64) -> String {
    format!("{}%", format_css_number(percent))
}

pub(super) fn format_css_number(value: f64) -> String {
    let value = normalize_css_number_for_serialization(value);
    format_css_number_exact(value)
}

fn format_css_number_exact(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    let mut serialized = format!("{value:.12}");
    if serialized.contains('.') {
        while serialized.ends_with('0') {
            serialized.pop();
        }
        if serialized.ends_with('.') {
            serialized.pop();
        }
    }
    serialized
}

fn normalize_css_number_for_serialization(value: f64) -> f64 {
    let rounded_integer = value.round();
    // PDB/Stylo values can carry f32 roundoff into the f64 CSSOM boundary.
    // Scale the integer tolerance with f32 precision, but cap it so genuine
    // fractional values remain observable.
    let integer_tolerance = (value.abs() * f64::from(f32::EPSILON)).clamp(1e-6, 1e-5);
    if (value - rounded_integer).abs() < integer_tolerance {
        return rounded_integer;
    }
    for scale in [10.0, 100.0, 1_000.0, 10_000.0, 100_000.0, 1_000_000.0] {
        let rounded = (value * scale).round() / scale;
        if (value - rounded).abs() < 2e-6 {
            return rounded;
        }
    }
    value
}

fn computed_animation_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> String {
    serialize_animation_shorthand_from_longhands([
        animation_longhand_components(runtime, handle, context, "animation-duration", "0s"),
        animation_longhand_components(
            runtime,
            handle,
            context,
            "animation-timing-function",
            "ease",
        ),
        animation_longhand_components(runtime, handle, context, "animation-delay", "0s"),
        animation_longhand_components(runtime, handle, context, "animation-iteration-count", "1"),
        animation_longhand_components(runtime, handle, context, "animation-direction", "normal"),
        animation_longhand_components(runtime, handle, context, "animation-fill-mode", "none"),
        animation_longhand_components(runtime, handle, context, "animation-play-state", "running"),
        animation_longhand_components(runtime, handle, context, "animation-name", "none"),
    ])
}

pub(super) fn inline_animation_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> String {
    let Some((entries, shared_css_wide_keyword)) = inline_shorthand_entries(
        runtime,
        handle,
        animation_shorthand_longhands(),
        &[
            "animation-timeline",
            "animation-range-start",
            "animation-range-end",
        ],
    ) else {
        return String::new();
    };
    if let Some(keyword) = shared_css_wide_keyword {
        return keyword;
    }

    let mut longhands: [Vec<String>; 8] = Default::default();
    for (index, entry) in entries
        .iter()
        .take(animation_shorthand_longhands().len())
        .enumerate()
    {
        longhands[index] = top_level_comma_separated_component_values(&entry.value)
            .unwrap_or_else(|| vec![entry.value.clone()]);
    }
    serialize_animation_shorthand_from_longhands(longhands)
}

pub(super) fn inline_animation_range_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> String {
    let Some((entries, shared_css_wide_keyword)) = inline_shorthand_entries(
        runtime,
        handle,
        &["animation-range-start", "animation-range-end"],
        &[],
    ) else {
        return String::new();
    };
    if let Some(keyword) = shared_css_wide_keyword {
        return keyword;
    }
    serialize_animation_range_shorthand(&entries[0].value, &entries[1].value)
}

fn computed_animation_range_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> String {
    let start = computed_style_property_value_with_context(
        runtime,
        handle,
        "animation-range-start",
        context,
    );
    let end =
        computed_style_property_value_with_context(runtime, handle, "animation-range-end", context);
    serialize_animation_range_shorthand(&start, &end)
}

pub(crate) fn serialize_animation_range_shorthand(start: &str, end: &str) -> String {
    let starts = top_level_comma_separated_component_values(start)
        .unwrap_or_else(|| vec![start.trim().to_owned()]);
    let ends = top_level_comma_separated_component_values(end)
        .unwrap_or_else(|| vec![end.trim().to_owned()]);
    if starts.is_empty() || starts.len() != ends.len() {
        return String::new();
    }
    starts
        .iter()
        .zip(ends.iter())
        .map(|(start, end)| serialize_single_animation_range(start, end))
        .collect::<Vec<_>>()
        .join(", ")
}

fn serialize_single_animation_range(start: &str, end: &str) -> String {
    let start = start.trim();
    let end = end.trim();
    if start.is_empty() || end.is_empty() {
        return String::new();
    }
    if start.eq_ignore_ascii_case("normal") && end.eq_ignore_ascii_case("normal") {
        return "normal".to_owned();
    }
    if let Some(start_name) = animation_range_name_only(start)
        && animation_range_name_only(end).is_some_and(|end_name| end_name == start_name)
    {
        return start.to_owned();
    }
    if animation_range_is_length_percentage_only(start)
        && (end.eq_ignore_ascii_case("normal") || animation_range_is_default_end_offset(end))
    {
        return start.to_owned();
    }
    if let Some(start_name) = animation_range_name_with_offset(start)
        && animation_range_name_only(end).is_some_and(|end_name| end_name == start_name)
    {
        start.to_owned()
    } else {
        format!("{start} {end}")
    }
}

fn animation_range_name_only(value: &str) -> Option<&'static str> {
    let name = animation_range_name_prefix(value)?;
    (value[name.len()..].trim().is_empty()).then_some(name)
}

fn animation_range_name_with_offset(value: &str) -> Option<&'static str> {
    let name = animation_range_name_prefix(value)?;
    (!value[name.len()..].trim().is_empty()).then_some(name)
}

fn animation_range_name_prefix(value: &str) -> Option<&'static str> {
    [
        "entry-crossing",
        "exit-crossing",
        "cover",
        "contain",
        "entry",
        "exit",
    ]
    .into_iter()
    .find(|name| {
        value == *name
            || value
                .strip_prefix(name)
                .is_some_and(|rest| rest.starts_with(char::is_whitespace))
    })
}

fn animation_range_is_length_percentage_only(value: &str) -> bool {
    !value.eq_ignore_ascii_case("normal") && animation_range_name_prefix(value).is_none()
}

fn animation_range_is_default_end_offset(value: &str) -> bool {
    value == "100%" || value == "calc(100%)"
}

pub(crate) fn serialize_animation_shorthand_from_longhands(longhands: [Vec<String>; 8]) -> String {
    if let Some(keyword) = shared_css_wide_keyword_for_longhands(&longhands) {
        return keyword;
    }
    let animation_count = longhands
        .iter()
        .map(Vec::len)
        .max()
        .filter(|count| *count > 0)
        .unwrap_or(1);
    (0..animation_count)
        .map(|index| {
            serialize_single_computed_animation(
                animation_value_at(&longhands[0], index, "0s"),
                animation_value_at(&longhands[1], index, "ease"),
                animation_value_at(&longhands[2], index, "0s"),
                animation_value_at(&longhands[3], index, "1"),
                animation_value_at(&longhands[4], index, "normal"),
                animation_value_at(&longhands[5], index, "none"),
                animation_value_at(&longhands[6], index, "running"),
                animation_value_at(&longhands[7], index, "none"),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn animation_longhand_components(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
    property: &str,
    initial: &str,
) -> Vec<String> {
    let value = computed_style_property_value_with_context(runtime, handle, property, context);
    let value = if value.is_empty() || css_wide_keyword(&value).is_some() {
        initial.to_owned()
    } else {
        value
    };
    top_level_comma_separated_component_values(&value).unwrap_or_else(|| vec![value])
}

fn animation_value_at<'a>(values: &'a [String], index: usize, initial: &'a str) -> &'a str {
    values
        .get(index)
        .or_else(|| values.last())
        .map(String::as_str)
        .unwrap_or(initial)
}

fn serialize_single_computed_animation(
    duration: &str,
    timing_function: &str,
    delay: &str,
    iteration_count: &str,
    direction: &str,
    fill_mode: &str,
    play_state: &str,
    name: &str,
) -> String {
    let duration = if duration == "auto" { "0s" } else { duration };
    let has_duration = duration != "0s";
    let has_timing_function = timing_function != "ease";
    let has_delay = delay != "0s";
    let has_iteration_count = iteration_count != "1";
    let has_direction = direction != "normal";
    let has_fill_mode = fill_mode != "none";
    let has_play_state = play_state != "running";
    let has_name = name != "none";
    let mut components = Vec::new();

    if has_duration || has_delay {
        components.push(duration);
    }
    if has_timing_function || animation_timing_keyword_name_requires_disambiguation(name) {
        components.push(timing_function);
    }
    if has_delay {
        components.push(delay);
    }
    if has_iteration_count {
        components.push(iteration_count);
    }
    if has_direction || animation_direction_keyword_name_requires_disambiguation(name) {
        components.push(direction);
    }
    if has_fill_mode || animation_fill_mode_keyword_name_requires_disambiguation(name) {
        components.push(fill_mode);
    }
    if has_play_state || animation_play_state_keyword_name_requires_disambiguation(name) {
        components.push(play_state);
    }
    if has_name || components.is_empty() {
        components.push(name);
    }

    components.join(" ")
}

fn animation_timing_keyword_name_requires_disambiguation(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "ease" | "linear" | "ease-in" | "ease-out" | "ease-in-out" | "step-start" | "step-end"
    )
}

fn animation_direction_keyword_name_requires_disambiguation(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "normal" | "reverse" | "alternate" | "alternate-reverse"
    )
}

fn animation_fill_mode_keyword_name_requires_disambiguation(name: &str) -> bool {
    if name.eq_ignore_ascii_case("none") {
        return false;
    }
    matches!(
        name.to_ascii_lowercase().as_str(),
        "none" | "forwards" | "backwards" | "both"
    )
}

fn animation_play_state_keyword_name_requires_disambiguation(name: &str) -> bool {
    matches!(name.to_ascii_lowercase().as_str(), "running" | "paused")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParsedTransitionComponent {
    property: String,
    duration: String,
    timing_function: String,
    delay: String,
    behavior: String,
}

pub(crate) fn parse_transition_shorthand_entries(value: &str) -> Option<[Vec<String>; 5]> {
    let transitions = parse_transition_shorthand_components(value)?;
    Some([
        transitions
            .iter()
            .map(|transition| transition.property.clone())
            .collect(),
        transitions
            .iter()
            .map(|transition| transition.duration.clone())
            .collect(),
        transitions
            .iter()
            .map(|transition| transition.timing_function.clone())
            .collect(),
        transitions
            .iter()
            .map(|transition| transition.delay.clone())
            .collect(),
        transitions
            .iter()
            .map(|transition| transition.behavior.clone())
            .collect(),
    ])
}

fn parse_transition_shorthand_components(value: &str) -> Option<Vec<ParsedTransitionComponent>> {
    let layers =
        top_level_comma_separated_component_values(value).filter(|layers| !layers.is_empty())?;
    layers
        .into_iter()
        .map(|layer| parse_single_transition(&layer))
        .collect()
}

fn parse_single_transition(value: &str) -> Option<ParsedTransitionComponent> {
    let tokens = box_shorthand_value_components(value)?;
    if tokens.iter().any(|token| css_wide_keyword(token).is_some()) {
        return (tokens.len() == 1).then(|| ParsedTransitionComponent {
            property: tokens[0].clone(),
            duration: tokens[0].clone(),
            timing_function: tokens[0].clone(),
            delay: tokens[0].clone(),
            behavior: tokens[0].clone(),
        });
    }
    let mut transition = ParsedTransitionComponent {
        property: "all".to_owned(),
        duration: "0s".to_owned(),
        timing_function: "ease".to_owned(),
        delay: "0s".to_owned(),
        behavior: "normal".to_owned(),
    };
    let mut seen_property = false;
    let mut seen_duration = false;
    let mut seen_delay = false;
    let mut seen_timing_function = false;
    let mut seen_behavior = false;
    for token in tokens {
        if let Some(time) = normalize_transition_time_token(&token) {
            if !seen_duration {
                let seconds = css_time_seconds(&token)?;
                if seconds < 0.0 {
                    return None;
                }
                transition.duration = time;
                seen_duration = true;
                continue;
            }
            if !seen_delay {
                transition.delay = time;
                seen_delay = true;
                continue;
            }
            return None;
        }
        if let Some(timing_function) = normalize_timing_function_value(&token) {
            if seen_timing_function {
                return None;
            }
            transition.timing_function = timing_function;
            seen_timing_function = true;
            continue;
        }
        match token.to_ascii_lowercase().as_str() {
            "normal" | "allow-discrete" if !seen_behavior => {
                transition.behavior = token.to_ascii_lowercase();
                seen_behavior = true;
                continue;
            }
            _ => {}
        }
        if seen_property || !transition_property_token_is_valid(&token) {
            return None;
        }
        transition.property = transition_property_token_serialization(&token)?;
        seen_property = true;
    }
    if transition.property.eq_ignore_ascii_case("none")
        && (seen_duration || seen_delay || seen_timing_function || seen_behavior)
    {
        return None;
    }
    Some(transition)
}

fn normalize_transition_time_token(value: &str) -> Option<String> {
    css_time_seconds(value).map(format_css_seconds)
}

fn css_time_seconds(value: &str) -> Option<f64> {
    moli_css_parse::resolve_css_numeric(
        value,
        moli_css_parse::CssNumericKind::Time,
        moli_css_parse::CssNumericContext::supports_probe(),
    )?
    .time_seconds()
}

fn transition_property_token_is_valid(value: &str) -> bool {
    transition_property_token_serialization(value).is_some()
}

fn transition_property_token_serialization(value: &str) -> Option<String> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    let ident = input
        .parse_entirely(|input| {
            input
                .expect_ident_cloned()
                .map_err(|_| input.new_custom_error::<(), ()>(()))
        })
        .ok()?;
    let lowered = ident.to_ascii_lowercase();
    if css_wide_keyword(&lowered).is_some() || lowered == "default" {
        return None;
    }
    if lowered == "all" || lowered == "none" {
        return Some(lowered);
    }
    let mut serialized = String::new();
    serialize_identifier(&ident, &mut serialized).ok()?;
    Some(serialized)
}

pub(crate) fn normalize_transition_property_list(value: &str) -> Option<String> {
    let layers =
        top_level_comma_separated_component_values(value).filter(|layers| !layers.is_empty())?;
    let properties = layers
        .into_iter()
        .map(|layer| transition_property_token_serialization(&layer))
        .collect::<Option<Vec<_>>>()?;
    if properties.len() > 1 && properties.iter().any(|layer| layer == "none") {
        return None;
    }
    Some(properties.join(", "))
}

pub(crate) fn normalize_transition_behavior_list(value: &str) -> Option<String> {
    let layers =
        top_level_comma_separated_component_values(value).filter(|layers| !layers.is_empty())?;
    layers
        .into_iter()
        .map(|layer| {
            let layer = layer.to_ascii_lowercase();
            matches!(layer.as_str(), "normal" | "allow-discrete").then_some(layer)
        })
        .collect::<Option<Vec<_>>>()
        .map(|layers| layers.join(", "))
}

pub(crate) fn normalize_transition_timing_function_list(value: &str) -> Option<String> {
    let layers = top_level_comma_separated_raw_component_values(value)?;
    layers
        .iter()
        .map(|layer| normalize_timing_function_value(layer))
        .collect::<Option<Vec<_>>>()
        .map(|layers| layers.join(", "))
}

fn normalize_timing_function_value(value: &str) -> Option<String> {
    let value = value.trim();
    let lowered = value.to_ascii_lowercase();
    match lowered.as_str() {
        "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out" => return Some(lowered),
        "step-start" => return Some("steps(1, start)".to_owned()),
        "step-end" => return Some("steps(1)".to_owned()),
        _ => {}
    }
    if let Some(inner) = css_function_inner(value, "cubic-bezier") {
        return normalize_cubic_bezier_timing_function_value(inner);
    }
    if let Some(inner) = css_function_inner(value, "steps") {
        let arguments = top_level_comma_separated_component_values(inner)?;
        if arguments.is_empty() || arguments.len() > 2 {
            return None;
        }
        let steps = normalize_steps_count_for_specified_value(&arguments[0])?;
        let position = arguments
            .get(1)
            .map(|position| position.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "jump-end".to_owned());
        if position == "jump-none"
            && let Ok(number) = steps.parse::<f64>()
            && number <= 1.0
        {
            return None;
        }
        match position.as_str() {
            "start" => {}
            "end" | "jump-end" => {
                if arguments.len() == 1 || position == "end" || position == "jump-end" {
                    return Some(format!("steps({steps})"));
                }
            }
            "jump-start" | "jump-both" => {}
            "jump-none" => {}
            _ => return None,
        }
        return Some(format!(
            "steps({steps}, {})",
            match position.as_str() {
                "end" => "jump-end",
                other => other,
            }
        ));
    }
    if css_function_inner(value, "linear").is_some() {
        let parsed = parse_linear_timing_function_value(value)?;
        if parsed.force_computed_serialization {
            return computed_linear_timing_function_value_from_parsed(
                &parsed,
                moli_css_parse::CssNumericContext::supports_probe(),
            );
        }
        return Some(specified_linear_timing_function_value(&parsed));
    }
    None
}

struct SpecifiedTimingNumber {
    serialized: String,
    value: f64,
    literal: bool,
}

fn normalize_cubic_bezier_timing_function_value(inner: &str) -> Option<String> {
    let arguments = top_level_comma_separated_component_values(inner)?;
    if arguments.len() != 4 {
        return None;
    }
    let numbers = arguments
        .iter()
        .map(|argument| normalize_timing_number_specified_value(argument))
        .collect::<Option<Vec<_>>>()?;
    for index in [0, 2] {
        if numbers[index].literal && !(0.0..=1.0).contains(&numbers[index].value) {
            return None;
        }
    }
    Some(format!(
        "cubic-bezier({}, {}, {}, {})",
        numbers[0].serialized, numbers[1].serialized, numbers[2].serialized, numbers[3].serialized
    ))
}

fn normalize_timing_number_specified_value(value: &str) -> Option<SpecifiedTimingNumber> {
    let trimmed = value.trim();
    if let Some(number) = moli_css_parse::parse_number(trimmed) {
        return Some(SpecifiedTimingNumber {
            serialized: format_css_number(number),
            value: number,
            literal: true,
        });
    }
    let serialized = moli_css_parse::normalize_cssom_component_value_serialization(trimmed)?;
    let value = match computed_timing_number_value(
        &serialized,
        moli_css_parse::CssNumericContext::supports_probe(),
    ) {
        Some(value) => value,
        None if dynamic_timing_number_specified_value_is_supported(&serialized) => {
            return Some(SpecifiedTimingNumber {
                serialized,
                value: 0.0,
                literal: false,
            });
        }
        None => return None,
    };
    let serialized = if css_static_calc_expression_can_be_folded(&serialized) {
        format!("calc({})", format_css_number(value))
    } else {
        serialized
    };
    Some(SpecifiedTimingNumber {
        serialized,
        value,
        literal: false,
    })
}

fn timing_number_has_dynamic_math(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("sign(") || lower.contains("sibling-index(") || lower.contains("sibling-count(")
}

fn dynamic_timing_number_specified_value_is_supported(value: &str) -> bool {
    let value = value.trim();
    if value.eq_ignore_ascii_case("sibling-index()")
        || value.eq_ignore_ascii_case("sibling-count()")
    {
        return true;
    }
    if css_function_inner(value, "sign").is_some() {
        return true;
    }
    css_function_inner(value, "calc")
        .is_some_and(|_| timing_number_has_dynamic_math(value) && balanced_css_function(value))
}

fn balanced_css_function(value: &str) -> bool {
    moli_css_parse::balanced_function_len(value).is_some_and(|len| len == value.len())
}

struct ParsedLinearTimingFunction {
    stops: Vec<ParsedLinearStop>,
    force_computed_serialization: bool,
}

struct ParsedLinearStop {
    output_raw: String,
    output: String,
    offsets: Vec<ParsedLinearStopOffset>,
}

struct ParsedLinearStopOffset {
    raw: String,
    specified: String,
}

fn parse_linear_timing_function_value(value: &str) -> Option<ParsedLinearTimingFunction> {
    let inner = css_function_inner(value, "linear")?;
    let raw_stops = top_level_comma_separated_raw_component_values(inner)?;
    if raw_stops.len() < 2 {
        return None;
    }
    let mut force_computed_serialization = false;
    let stops = raw_stops
        .into_iter()
        .map(|raw_stop| {
            let components = top_level_whitespace_separated_raw_component_values(&raw_stop)?;
            if components.is_empty() || components.len() > 3 {
                return None;
            }
            let (output, force_computed_output) =
                normalize_linear_stop_output_value(&components[0])?;
            force_computed_serialization |= force_computed_output;
            let offsets = components
                .iter()
                .skip(1)
                .map(|component| {
                    Some(ParsedLinearStopOffset {
                        raw: component.clone(),
                        specified: normalize_linear_stop_percentage_specified_value(component)?,
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            Some(ParsedLinearStop {
                output_raw: components[0].clone(),
                output,
                offsets,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(ParsedLinearTimingFunction {
        stops,
        force_computed_serialization,
    })
}

fn normalize_linear_stop_output_value(value: &str) -> Option<(String, bool)> {
    if css_calc_nan_number_is_zero(value) {
        return Some(("0".to_owned(), true));
    }
    let value = normalize_timing_number_specified_value(value)?;
    Some((value.serialized, false))
}

fn normalize_linear_stop_percentage_specified_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed
        .strip_suffix('%')
        .and_then(moli_css_parse::parse_number)
        .is_some_and(f64::is_finite)
    {
        let percent = trimmed
            .strip_suffix('%')
            .and_then(moli_css_parse::parse_number)?;
        return Some(format!("{}%", format_css_number_exact(percent)));
    }
    let serialized = moli_css_parse::normalize_cssom_component_value_serialization(trimmed)?;
    let percent = computed_linear_stop_percentage_value(
        &serialized,
        moli_css_parse::CssNumericContext::supports_probe(),
    )?;
    if css_static_calc_expression_can_be_folded(&serialized) {
        Some(format!("calc({})", format_css_percent(percent)))
    } else {
        Some(serialized)
    }
}

fn specified_linear_timing_function_value(parsed: &ParsedLinearTimingFunction) -> String {
    let stops = parsed
        .stops
        .iter()
        .map(|stop| {
            let mut components = Vec::with_capacity(stop.offsets.len() + 1);
            components.push(stop.output.clone());
            components.extend(stop.offsets.iter().map(|offset| offset.specified.clone()));
            components.join(" ")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("linear({stops})")
}

fn normalize_steps_count_for_specified_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if css_steps_count_expression_is_valid(trimmed) {
        let serialized = moli_css_parse::normalize_cssom_component_value_serialization(trimmed)
            .unwrap_or_else(|| trimmed.to_owned());
        if css_static_calc_expression_can_be_folded(&serialized) {
            let number = moli_css_parse::resolve_css_numeric(
                trimmed,
                moli_css_parse::CssNumericKind::Number,
                moli_css_parse::CssNumericContext::supports_probe(),
            )?
            .number()?;
            return Some(format!("calc({})", format_css_number(number)));
        }
        return Some(serialized);
    }
    let number = normalize_css_integer_token(value)?;
    (!number.starts_with('-') && number != "0").then_some(number)
}

fn css_steps_count_expression_is_valid(value: &str) -> bool {
    let value = value.trim();
    if value.eq_ignore_ascii_case("sibling-index()")
        || value.eq_ignore_ascii_case("sibling-count()")
    {
        return true;
    }
    css_function_inner(value, "calc").is_some_and(css_steps_count_calc_expression_is_valid)
}

fn css_steps_count_calc_expression_is_valid(value: &str) -> bool {
    let mut rest = value.trim();
    while !rest.is_empty() {
        let trimmed = rest.trim_start();
        rest = trimmed;
        if let Some(next) = rest.strip_prefix("sibling-index()") {
            rest = next;
            continue;
        }
        if let Some(next) = rest.strip_prefix("sibling-count()") {
            rest = next;
            continue;
        }
        if rest.to_ascii_lowercase().starts_with("sign(")
            && let Some(len) = moli_css_parse::balanced_function_len(rest)
        {
            rest = &rest[len..];
            continue;
        }
        if let Some(len) = moli_css_parse::number_len(rest) {
            rest = &rest[len..];
            continue;
        }
        let Some(ch) = rest.chars().next() else {
            return true;
        };
        if matches!(ch, '+' | '-' | '*' | '/' | '(' | ')') {
            rest = &rest[ch.len_utf8()..];
            continue;
        }
        return false;
    }
    true
}

fn computed_timing_function_list_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    let entry = inline_style_entry_for_inline_style(runtime, handle, property)?;
    let context = css_numeric_context(runtime, handle);
    let values = top_level_comma_separated_raw_component_values(&entry.value)
        .unwrap_or_else(|| vec![entry.value]);
    let resolved = values
        .into_iter()
        .map(|value| computed_timing_function_value(&value, context))
        .collect::<Option<Vec<_>>>()?;
    (!resolved.is_empty()).then(|| resolved.join(", "))
}

fn computed_timing_function_value(
    value: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<String> {
    let value = value.trim();
    match value.to_ascii_lowercase().as_str() {
        "linear" | "ease" | "ease-in" | "ease-out" | "ease-in-out" => {
            return Some(value.to_owned());
        }
        "step-start" => return Some("steps(1, start)".to_owned()),
        "step-end" => return Some("steps(1)".to_owned()),
        _ => {}
    }
    if let Some(inner) = css_function_inner(value, "cubic-bezier") {
        return computed_cubic_bezier_timing_function_value(inner, context);
    }
    if let Some(inner) = css_function_inner(value, "steps") {
        let arguments = top_level_comma_separated_component_values(inner)?;
        if arguments.is_empty() || arguments.len() > 2 {
            return None;
        }
        let mut steps = moli_css_parse::resolve_css_numeric(
            &arguments[0],
            moli_css_parse::CssNumericKind::Number,
            context,
        )?
        .number()?;
        steps = steps.round();
        let position = arguments
            .get(1)
            .map(|position| position.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "jump-end".to_owned());
        match position.as_str() {
            "start" => {
                if steps < 1.0 {
                    steps = 1.0;
                }
                Some(format!("steps({}, start)", format_css_number(steps)))
            }
            "end" | "jump-end" => {
                if steps < 1.0 {
                    steps = 1.0;
                }
                Some(format!("steps({})", format_css_number(steps)))
            }
            "jump-start" | "jump-both" => {
                if steps < 1.0 {
                    steps = 1.0;
                }
                Some(format!("steps({}, {position})", format_css_number(steps)))
            }
            "jump-none" => {
                if steps < 2.0 {
                    steps = 2.0;
                }
                Some(format!("steps({}, jump-none)", format_css_number(steps)))
            }
            _ => None,
        }
    } else if css_function_inner(value, "linear").is_some() {
        computed_linear_timing_function_value(value, context)
    } else {
        normalize_timing_function_value(value)
    }
}

fn computed_cubic_bezier_timing_function_value(
    inner: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<String> {
    let arguments = top_level_comma_separated_component_values(inner)?;
    if arguments.len() != 4 {
        return None;
    }
    let mut numbers = arguments
        .iter()
        .map(|argument| computed_timing_number_value(argument, context))
        .collect::<Option<Vec<_>>>()?;
    numbers[0] = numbers[0].clamp(0.0, 1.0);
    numbers[2] = numbers[2].clamp(0.0, 1.0);
    Some(format!(
        "cubic-bezier({}, {}, {}, {})",
        format_css_number(numbers[0]),
        format_css_number(numbers[1]),
        format_css_number(numbers[2]),
        format_css_number(numbers[3])
    ))
}

fn computed_linear_timing_function_value(
    value: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<String> {
    let parsed = parse_linear_timing_function_value(value)?;
    computed_linear_timing_function_value_from_parsed(&parsed, context)
}

fn computed_linear_timing_function_value_from_parsed(
    parsed: &ParsedLinearTimingFunction,
    context: moli_css_parse::CssNumericContext,
) -> Option<String> {
    let mut points = Vec::new();
    for stop in &parsed.stops {
        let output = if css_calc_nan_number_is_zero(&stop.output_raw) {
            0.0
        } else {
            computed_timing_number_value(&stop.output_raw, context)?
        };
        let output = format_css_number(output);
        if stop.offsets.is_empty() {
            points.push(ComputedLinearPoint {
                output,
                offset: None,
            });
        } else {
            for offset in &stop.offsets {
                points.push(ComputedLinearPoint {
                    output: output.clone(),
                    offset: Some(computed_linear_stop_percentage_value(&offset.raw, context)?),
                });
            }
        }
    }
    if points.len() < 2 {
        return None;
    }
    let mut offsets = points.iter().map(|point| point.offset).collect::<Vec<_>>();
    assign_linear_stop_offsets(&mut offsets)?;
    let stops = points
        .into_iter()
        .zip(offsets)
        .map(|(point, offset)| {
            format!(
                "{} {}",
                point.output,
                format_css_linear_percent(offset.expect("linear stop offsets should be assigned"))
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("linear({stops})"))
}

struct ComputedLinearPoint {
    output: String,
    offset: Option<f64>,
}

fn assign_linear_stop_offsets(offsets: &mut [Option<f64>]) -> Option<()> {
    if offsets.len() < 2 {
        return None;
    }
    let specified = offsets
        .iter()
        .enumerate()
        .filter_map(|(index, offset)| offset.map(|_| index))
        .collect::<Vec<_>>();
    if specified.is_empty() {
        let denominator = (offsets.len() - 1) as f64;
        for (index, offset) in offsets.iter_mut().enumerate() {
            *offset = Some(index as f64 * 100.0 / denominator);
        }
        return Some(());
    }

    let first = specified[0];
    if first > 0 {
        let end = offsets[first]?;
        for (index, offset) in offsets.iter_mut().enumerate().take(first) {
            *offset = Some(index as f64 * end / first as f64);
        }
    }

    for pair in specified.windows(2) {
        let start_index = pair[0];
        let end_index = pair[1];
        let start = offsets[start_index]?;
        let end = offsets[end_index]?;
        let span = (end_index - start_index) as f64;
        for (index, offset) in offsets
            .iter_mut()
            .enumerate()
            .take(end_index)
            .skip(start_index + 1)
        {
            let progress = (index - start_index) as f64 / span;
            *offset = Some(start + (end - start) * progress);
        }
    }

    let last = *specified.last()?;
    if last + 1 < offsets.len() {
        let start = offsets[last]?;
        let end = start.max(100.0);
        let span = (offsets.len() - 1 - last) as f64;
        for (index, offset) in offsets.iter_mut().enumerate().skip(last + 1) {
            let progress = (index - last) as f64 / span;
            *offset = Some(start + (end - start) * progress);
        }
    }
    Some(())
}

fn computed_timing_number_value(
    value: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<f64> {
    moli_css_parse::resolve_css_numeric(value, moli_css_parse::CssNumericKind::Number, context)?
        .number()
}

fn computed_linear_stop_percentage_value(
    value: &str,
    context: moli_css_parse::CssNumericContext,
) -> Option<f64> {
    if let Some(percent) = value
        .trim()
        .strip_suffix('%')
        .and_then(moli_css_parse::parse_number)
    {
        return Some(percent);
    }
    moli_css_parse::resolve_css_numeric(value, moli_css_parse::CssNumericKind::Percentage, context)?
        .percentage()
}

fn css_calc_nan_number_is_zero(value: &str) -> bool {
    let compact = value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    compact.eq_ignore_ascii_case("calc(0/0)")
}

fn css_static_calc_expression_can_be_folded(value: &str) -> bool {
    if css_function_inner(value.trim(), "calc").is_none() {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    !lower.contains("sign(")
        && !lower.contains("sibling-index(")
        && !lower.contains("sibling-count(")
        && !lower.contains("var(")
        && !lower.contains("env(")
}

fn format_css_linear_percent(percent: f64) -> String {
    if percent == 0.0 {
        return "0%".to_owned();
    }
    let rounded = (percent * 1_000_000.0).round() / 1_000_000.0;
    let mut serialized = format!("{rounded:.6}");
    while serialized.ends_with('0') {
        serialized.pop();
    }
    if serialized.ends_with('.') {
        serialized.pop();
    }
    format!("{serialized}%")
}

fn css_function_inner<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    let lower = value.to_ascii_lowercase();
    let prefix = format!("{name}(");
    if !lower.starts_with(&prefix) || !value.ends_with(')') {
        return None;
    }
    moli_css_parse::balanced_function_len(value)
        .filter(|len| *len == value.len())
        .map(|_| &value[prefix.len()..value.len() - 1])
}

fn top_level_comma_separated_raw_component_values(value: &str) -> Option<Vec<String>> {
    split_top_level_raw_component_values(value, RawComponentSeparator::Comma)
}

fn top_level_whitespace_separated_raw_component_values(value: &str) -> Option<Vec<String>> {
    split_top_level_raw_component_values(value, RawComponentSeparator::Whitespace)
}

enum RawComponentSeparator {
    Comma,
    Whitespace,
}

fn split_top_level_raw_component_values(
    value: &str,
    separator: RawComponentSeparator,
) -> Option<Vec<String>> {
    let mut components = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (index, ch) in value.char_indices() {
        match ch {
            '(' => {
                depth += 1;
                start.get_or_insert(index);
            }
            ')' => {
                depth = depth.checked_sub(1)?;
                start.get_or_insert(index);
            }
            ',' if depth == 0 && matches!(separator, RawComponentSeparator::Comma) => {
                let component = value[start.unwrap_or(0)..index].trim();
                if component.is_empty() {
                    return None;
                }
                components.push(component.to_owned());
                start = None;
            }
            ch if ch.is_whitespace()
                && depth == 0
                && matches!(separator, RawComponentSeparator::Whitespace) =>
            {
                if let Some(component_start) = start.take() {
                    let component = value[component_start..index].trim();
                    if !component.is_empty() {
                        components.push(component.to_owned());
                    }
                }
            }
            ch if ch.is_whitespace() => {}
            _ => {
                start.get_or_insert(index);
            }
        }
    }
    if depth != 0 {
        return None;
    }
    let component = value[start.unwrap_or(value.len())..].trim();
    if !component.is_empty() {
        components.push(component.to_owned());
    }
    (!components.is_empty()).then_some(components)
}

pub(super) fn inline_transition_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> String {
    transition_shorthand_from_longhands(runtime, handle, None)
}

fn computed_transition_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> String {
    transition_shorthand_from_longhands(runtime, handle, Some(context))
}

fn transition_shorthand_from_longhands(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: Option<StyleComputationContext>,
) -> String {
    let mut longhands: [Vec<String>; 5] = Default::default();
    if let Some(context) = context {
        for (index, longhand) in transition_shorthand_longhands().iter().enumerate() {
            let value =
                computed_style_property_value_with_context(runtime, handle, longhand, context);
            let value = if value.is_empty() {
                computed_style_default_value(runtime, handle, longhand)
            } else {
                value
            };
            longhands[index] =
                top_level_comma_separated_component_values(&value).unwrap_or_else(|| vec![value]);
        }
    } else {
        let Some((entries, shared_css_wide_keyword)) =
            inline_shorthand_entries(runtime, handle, transition_shorthand_longhands(), &[])
        else {
            return String::new();
        };
        if let Some(keyword) = shared_css_wide_keyword {
            return keyword;
        }
        for (index, entry) in entries.iter().enumerate() {
            longhands[index] = top_level_comma_separated_component_values(&entry.value)
                .unwrap_or_else(|| vec![entry.value.clone()]);
        }
    }
    serialize_transition_shorthand_from_longhands(longhands)
}

fn inline_shorthand_entries(
    runtime: &JsContextHost,
    handle: DomHandle,
    longhands: &[&str],
    reset_only_longhands: &[&str],
) -> Option<(Vec<StyleEntry>, Option<String>)> {
    let mut entries = Vec::with_capacity(longhands.len() + reset_only_longhands.len());
    let mut priority = None;
    for longhand in longhands.iter().chain(reset_only_longhands.iter()) {
        let value = inline_longhand_property_value_for_shorthand(runtime, handle, longhand)?;
        let entry_priority =
            inline_longhand_property_priority_for_shorthand(runtime, handle, longhand)?;
        if priority.is_some_and(|current| current != entry_priority) {
            return None;
        }
        priority = Some(entry_priority);
        entries.push(StyleEntry {
            name: (*longhand).to_owned(),
            value,
            priority: entry_priority,
        });
    }

    let css_wide_keywords = entries
        .iter()
        .map(|entry| css_wide_keyword(&entry.value))
        .collect::<Option<Vec<_>>>();
    if entries
        .iter()
        .any(|entry| css_wide_keyword(&entry.value).is_some())
    {
        let keywords = css_wide_keywords?;
        let first = keywords.first()?.clone();
        if keywords.iter().all(|keyword| keyword == &first) {
            return Some((entries, Some(first)));
        }
        return None;
    }

    Some((entries, None))
}

fn inline_longhand_property_value_for_shorthand(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    let property = canonical_style_property_name(property);
    if let Some(state) = runtime.element_inline_style_declaration_state(handle)
        && let Some(value) = inline_state_property_value_with_pdb(state, &property)
        && !value.is_empty()
    {
        return Some(value);
    }
    inline_style_entry_for_inline_style(runtime, handle, &property)
        .map(|entry| entry.value)
        .filter(|value| !value.is_empty())
}

fn inline_longhand_property_priority_for_shorthand(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<bool> {
    let property = canonical_style_property_name(property);
    if let Some(state) = runtime.element_inline_style_declaration_state(handle)
        && let Some(priority) = inline_state_property_priority_with_pdb(state, &property)
    {
        return Some(priority);
    }
    inline_style_entry_for_inline_style(runtime, handle, &property).map(|entry| entry.priority)
}

pub(crate) fn serialize_transition_shorthand_from_longhands(longhands: [Vec<String>; 5]) -> String {
    if let Some(keyword) = shared_css_wide_keyword_for_longhands(&longhands) {
        return keyword;
    }
    let transition_count = longhands
        .iter()
        .map(Vec::len)
        .max()
        .filter(|count| *count > 0)
        .unwrap_or(1);
    (0..transition_count)
        .map(|index| {
            serialize_single_transition(
                transition_value_at(&longhands[0], index, "all"),
                transition_value_at(&longhands[1], index, "0s"),
                transition_value_at(&longhands[2], index, "ease"),
                transition_value_at(&longhands[3], index, "0s"),
                transition_value_at(&longhands[4], index, "normal"),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn shared_css_wide_keyword_for_longhands<const N: usize>(
    longhands: &[Vec<String>; N],
) -> Option<String> {
    let keyword = longhands
        .first()?
        .first()
        .and_then(|value| css_wide_keyword(value))?;
    longhands
        .iter()
        .all(|values| matches!(values.as_slice(), [value] if value == &keyword))
        .then_some(keyword)
}

fn transition_value_at<'a>(values: &'a [String], index: usize, initial: &'a str) -> &'a str {
    values
        .get(index)
        .or_else(|| values.last())
        .map(String::as_str)
        .unwrap_or(initial)
}

fn serialize_single_transition(
    property: &str,
    duration: &str,
    timing_function: &str,
    delay: &str,
    behavior: &str,
) -> String {
    if css_wide_keyword(property).is_some()
        && property == duration
        && property == timing_function
        && property == delay
        && property == behavior
    {
        return property.to_owned();
    }
    if property == "none"
        && duration == "0s"
        && timing_function == "ease"
        && delay == "0s"
        && behavior == "normal"
    {
        return "none".to_owned();
    }
    let mut components = Vec::new();
    if property != "all" {
        components.push(property);
    }
    if duration != "0s" || delay != "0s" {
        components.push(duration);
    }
    if timing_function != "ease" {
        components.push(timing_function);
    }
    if delay != "0s" {
        components.push(delay);
    }
    if behavior != "normal" {
        components.push(behavior);
    }
    if components.is_empty() {
        components.push("all");
    }
    components.join(" ")
}

fn shadow_tree_inherited_value_for_initial_stylo_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    let initial = match property {
        "color" => "rgb(0, 0, 0)",
        "font-size" => "16px",
        _ => return None,
    };
    if runtime.dom_host().containing_shadow_root(handle).is_none()
        || value != initial
        || inline_style_entry_for_inline_style(runtime, handle, property).is_some()
    {
        return None;
    }
    inherited_style_parent(runtime, handle)
        .map(|parent| resolution.computed_property(runtime, parent, property))
        .filter(|value| !value.is_empty() && value != initial)
}
