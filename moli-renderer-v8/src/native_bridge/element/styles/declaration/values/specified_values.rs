use super::*;

pub(crate) fn active_css_animations(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Vec<crate::style_engine::CssAnimationMetadata> {
    if !runtime
        .dom_host()
        .node(handle)
        .is_some_and(|node| node.is_connected() && node.as_element().is_some())
    {
        return Vec::new();
    }
    let read = ComputedStyleRead::new(runtime, handle);
    if read
        .rendered_style_facts()
        .is_some_and(|facts| facts.display == ComputedDisplayKind::None)
    {
        return Vec::new();
    }
    let Some(style) = read.computed_values() else {
        return Vec::new();
    };
    runtime.css_animations_from_current_observation(handle, &style)
}

pub(crate) fn css_animation_start_applies(runtime: &JsContextHost, handle: DomHandle) -> bool {
    // Event eligibility and getAnimations must use the same cascade. Even
    // empty keyframes and implicit endpoints produce animation events.
    !active_css_animations(runtime, handle).is_empty()
}

pub(super) fn active_css_animation_midpoint_px_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    let (from, to) = active_css_animation_property_values_with_resolution(
        runtime, handle, property, resolution,
    )?;
    let from = moli_css_parse::parse_px_length(&from, moli_css_parse::UnitlessLength::ZeroOnly)?;
    let to = moli_css_parse::parse_px_length(&to, moli_css_parse::UnitlessLength::ZeroOnly)?;
    Some((from + to) / 2.0)
}

pub(super) fn active_registered_length_custom_property_animation_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    underlying_value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if !property.starts_with("--") {
        return None;
    }
    let document = stylesheet_source_document_for_handle(runtime, handle)?;
    let registration = runtime.registered_css_custom_property_registration(document, property)?;
    if registration.syntax.trim() != "<length>" {
        return None;
    }
    let (from, to) = active_css_animation_property_values_with_resolution(
        runtime, handle, property, resolution,
    )?;
    let from = registered_length_keyframe_endpoint_px(&from, underlying_value)?;
    let to = registered_length_keyframe_endpoint_px(&to, underlying_value)?;
    Some(format!("{}px", (from + to) / 2.0))
}

fn registered_length_keyframe_endpoint_px(value: &str, underlying_value: &str) -> Option<f64> {
    let value = if value.trim().eq_ignore_ascii_case("revert") {
        underlying_value
    } else {
        value
    };
    moli_css_parse::parse_px_length(value, moli_css_parse::UnitlessLength::ZeroOnly)
}

fn active_css_animation_translate_x_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<f64> {
    let (from, to) = active_css_animation_property_values_with_resolution(
        runtime,
        handle,
        "transform",
        resolution,
    )?;
    let from = transform_translate_x_px(&from)?;
    let to = transform_translate_x_px(&to)?;
    Some((from + to) / 2.0)
}

pub(crate) fn active_css_animation_transform_value(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<String> {
    let read = ComputedStyleRead::new(runtime, handle);
    active_css_animation_transform_value_with_resolution(runtime, handle, read.resolution_context())
}

pub(super) fn active_css_animation_transform_value_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    active_css_animation_translate_x_with_resolution(runtime, handle, resolution)
        .map(|value| format!("translateX({value}px)"))
}

pub(super) fn active_css_animation_static_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    let (from, to) = active_css_animation_property_values_with_resolution(
        runtime, handle, property, resolution,
    )?;
    (from.eq_ignore_ascii_case(&to)).then_some(from)
}

fn active_css_animation_property_values_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<(String, String)> {
    let names = active_css_animation_names_with_resolution(runtime, handle, resolution);
    if names.is_empty() {
        return None;
    }
    let document = stylesheet_source_document_for_handle(runtime, handle)?;
    for source in effective_raw_stylesheet_sources(
        runtime,
        document,
        false,
        resolution.computation.viewport(),
    ) {
        if let Some(values) =
            keyframe_property_values(&source.serialized_css_text(), &names, property)
        {
            return Some(values);
        }
    }
    None
}

fn active_css_animation_names_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    resolution: StyleResolutionContext<'_>,
) -> Vec<String> {
    let animation_name = resolution.computed_property(runtime, handle, "animation-name");
    let mut names = animation_name
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("none"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    // A non-empty computed animation-name (including the initial `none`) is
    // authoritative. Only use the legacy shorthand fallback when the
    // longhand serializer itself is unavailable.
    if names.is_empty() && animation_name.trim().is_empty() {
        let animation = resolution.computed_property(runtime, handle, "animation");
        names = animation_shorthand_names(&animation);
    }
    names
}

pub(super) fn animation_shorthand_names(value: &str) -> Vec<String> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    let mut names = Vec::new();
    let mut current = None;
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => {}
            Token::Comma => {
                if let Some(name) = current.take() {
                    names.push(name);
                }
            }
            Token::Ident(value) if !ident_is_animation_shorthand_keyword(&value) => {
                current = Some(value.to_string());
            }
            Token::Function(_) => {
                let _ = input.parse_nested_block(|_| Ok::<_, cssparser::ParseError<'_, ()>>(()));
            }
            _ => {}
        }
    }
    if let Some(name) = current {
        names.push(name);
    }
    names
}

fn ident_is_animation_shorthand_keyword(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "none"
            | "normal"
            | "linear"
            | "ease"
            | "ease-in"
            | "ease-out"
            | "ease-in-out"
            | "step-start"
            | "step-end"
            | "infinite"
            | "alternate"
            | "alternate-reverse"
            | "reverse"
            | "forwards"
            | "backwards"
            | "both"
            | "running"
            | "paused"
    )
}

pub(super) fn keyframe_property_values(
    css_text: &str,
    animation_names: &[String],
    property: &str,
) -> Option<(String, String)> {
    let rules = moli_css_parse::parse_stylesheet_rule_snapshots_with_stylo(css_text);
    keyframe_rule_snapshots_property_values(&rules, animation_names, property, 0)
}

pub(super) const KEYFRAME_NESTING_DEPTH_LIMIT: usize = 32;

#[cfg(test)]
pub(super) fn keyframe_has_supported_animation_values(
    css_text: &str,
    animation_names: &[String],
) -> bool {
    let rules = moli_css_parse::parse_stylesheet_rule_snapshots_with_stylo(css_text);
    keyframe_rule_snapshots_have_supported_animation_values(&rules, animation_names, 0)
}

fn keyframe_rule_snapshots_property_values(
    rules: &[moli_css_parse::CssRuleSnapshot],
    animation_names: &[String],
    property: &str,
    depth: usize,
) -> Option<(String, String)> {
    if depth > KEYFRAME_NESTING_DEPTH_LIMIT {
        return None;
    }
    for rule in rules {
        match rule.rule_type {
            CssRuleType::Keyframes if keyframe_rule_name_matches(rule, animation_names) => {
                if let Some(values) =
                    keyframe_child_rule_snapshots_property_values(&rule.child_rules, property)
                {
                    return Some(values);
                }
            }
            CssRuleType::Media | CssRuleType::Supports => {
                if let Some(values) = keyframe_rule_snapshots_property_values(
                    &rule.child_rules,
                    animation_names,
                    property,
                    depth + 1,
                ) {
                    return Some(values);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
fn keyframe_rule_snapshots_have_supported_animation_values(
    rules: &[moli_css_parse::CssRuleSnapshot],
    animation_names: &[String],
    depth: usize,
) -> bool {
    if depth > KEYFRAME_NESTING_DEPTH_LIMIT {
        return false;
    }
    for rule in rules {
        match rule.rule_type {
            CssRuleType::Keyframes if keyframe_rule_name_matches(rule, animation_names) => {
                if keyframe_child_rule_snapshots_have_supported_animation_values(&rule.child_rules)
                {
                    return true;
                }
            }
            CssRuleType::Media | CssRuleType::Supports
                if keyframe_rule_snapshots_have_supported_animation_values(
                    &rule.child_rules,
                    animation_names,
                    depth + 1,
                ) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

fn keyframe_rule_name_matches(
    rule: &moli_css_parse::CssRuleSnapshot,
    animation_names: &[String],
) -> bool {
    moli_css_parse::parse_keyframes_rule_view_with_stylo(&rule.css_text).is_some_and(|view| {
        animation_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&view.name))
    })
}

#[cfg(test)]
fn keyframe_child_rule_snapshots_have_supported_animation_values(
    rules: &[moli_css_parse::CssRuleSnapshot],
) -> bool {
    keyframe_child_rule_snapshots_property_values(rules, "left").is_some_and(|(from, to)| {
        moli_css_parse::parse_px_length(&from, moli_css_parse::UnitlessLength::ZeroOnly).is_some()
            && moli_css_parse::parse_px_length(&to, moli_css_parse::UnitlessLength::ZeroOnly)
                .is_some()
    }) || keyframe_child_rule_snapshots_property_values(rules, "transform").is_some_and(
        |(from, to)| {
            transform_translate_x_px(&from).is_some() && transform_translate_x_px(&to).is_some()
        },
    ) || keyframe_child_rule_snapshots_property_values(rules, "color")
        .is_some_and(|(from, to)| from.eq_ignore_ascii_case(&to))
        || keyframe_child_rule_snapshots_property_values(rules, "background-color")
            .is_some_and(|(from, to)| from.eq_ignore_ascii_case(&to))
}

fn keyframe_child_rule_snapshots_property_values(
    rules: &[moli_css_parse::CssRuleSnapshot],
    property: &str,
) -> Option<(String, String)> {
    let mut from = None;
    let mut to = None;
    for snapshot in rules {
        if snapshot.rule_type != CssRuleType::Keyframe {
            continue;
        }
        let Some(selector_text) = snapshot.selector_text.as_deref() else {
            continue;
        };
        let Some(selector_text) =
            moli_css_parse::normalize_keyframe_selector_text_with_stylo(selector_text)
        else {
            continue;
        };
        let Some(style_text) = snapshot.declaration_text.as_deref() else {
            continue;
        };
        let value = parse_inline_css_text_with_base(style_text, None)
            .into_iter()
            .find(|entry| entry.name == property)
            .map(|entry| entry.value);
        let Some(value) = value else {
            continue;
        };
        if keyframe_selector_text_contains_normalized_endpoint(&selector_text, "0%") {
            from = Some(value.clone());
        }
        if keyframe_selector_text_contains_normalized_endpoint(&selector_text, "100%") {
            to = Some(value);
        }
    }
    Some((from?, to?))
}

fn keyframe_selector_text_contains_normalized_endpoint(
    selector_text: &str,
    endpoint: &str,
) -> bool {
    selector_text
        .split(',')
        .any(|selector| selector.trim() == endpoint)
}

fn transform_translate_x_px(value: &str) -> Option<f64> {
    let function = moli_css_parse::parse_transform_function_list(value)?
        .into_iter()
        .find(|function| matches!(function.name.as_str(), "translate" | "translatex"))?;
    let raw = function.arguments.first()?;
    moli_css_parse::parse_px_length(raw, moli_css_parse::UnitlessLength::ZeroOnly)
}

fn stylo_computed_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    context: StyleComputationContext,
) -> Option<String> {
    ComputedStyleRead::new_with_context(runtime, handle, context).raw_primary_property(property)
}

fn stylo_computed_pseudo_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    pseudo_element: &str,
    property: &str,
    context: StyleComputationContext,
) -> Option<String> {
    let read = ComputedStyleRead::new_with_context(runtime, handle, context);
    let value = read.raw_pseudo_property(pseudo_element, property);
    (!value.is_empty()).then_some(value)
}

pub(super) fn normalized_stylo_computed_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    context: StyleComputationContext,
) -> Option<String> {
    let value = stylo_computed_style_value(runtime, handle, property, context)?;
    normalize_stylo_computed_style_value(runtime, handle, property, &value, context)
}

pub(super) fn normalized_stylo_computed_style_value_with_inputs(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    context: StyleComputationContext,
    inputs: &FullStyleWorldSnapshot,
) -> Option<String> {
    let read_document = context.resolved_read_document(runtime, handle);
    let value = runtime.computed_style_property_value_from_stylo(
        handle,
        property,
        None,
        inputs,
        read_document,
        context.viewport,
    )?;
    normalize_stylo_computed_style_value(runtime, handle, property, &value, context)
}

fn normalize_stylo_computed_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    context: StyleComputationContext,
) -> Option<String> {
    normalize_stylo_computed_style_value_with_resolution(
        runtime,
        handle,
        property,
        value,
        StyleResolutionContext::independent(context),
    )
}

pub(super) fn normalize_stylo_computed_style_value_with_resolution(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    value: &str,
    resolution: StyleResolutionContext<'_>,
) -> Option<String> {
    if property == "text-size-adjust" {
        return computed_text_size_adjust_specified_value(runtime, handle, value)
            .or_else(|| Some(value.to_owned()));
    }
    if property == "touch-action" {
        return Some(normalize_touch_action_serialization(property, value));
    }
    if color_property_is_resolved_color(property) {
        return Some(resolve_computed_color_property_value(
            runtime, handle, property, value, resolution,
        ));
    }
    Some(value.to_owned())
}

fn normalize_touch_action_serialization(property: &str, value: &str) -> String {
    if property == "touch-action" && value == "pan-x pan-y pinch-zoom" {
        "manipulation".to_owned()
    } else {
        value.to_owned()
    }
}

pub(super) fn normalized_stylo_computed_pseudo_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    pseudo_element: &str,
    property: &str,
    context: StyleComputationContext,
) -> Option<String> {
    let value =
        stylo_computed_pseudo_style_value(runtime, handle, pseudo_element, property, context)?;
    normalize_stylo_computed_style_value(runtime, handle, property, &value, context)
}

pub(super) fn inline_style_entry_for_inline_style(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<StyleEntry> {
    // Non-computed CSSStyleDeclaration reads are specified/inline style only.
    // Stylesheet cascade is resolved by Stylo in the computed-style branch.
    inline_style_entry(runtime, handle, property)
}

pub(super) fn inline_style_property_value_for_inline_style(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    let property = canonical_style_property_name(property);
    if let Some(state) = runtime.element_inline_style_declaration_state(handle)
        && let Some(value) = inline_state_property_value_with_pdb(state, &property)
    {
        return Some(normalize_touch_action_serialization(&property, &value));
    }
    inline_style_entry_for_inline_style(runtime, handle, &property)
        .map(|entry| normalize_touch_action_serialization(&property, &entry.value))
}

pub(crate) fn raw_inline_style_property_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<String> {
    inline_style_property_value_for_inline_style(runtime, handle, property)
}

pub(in crate::native_bridge::element::styles) fn normalize_style_value(
    name: &str,
    value: &str,
) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if !name.starts_with("--")
        && !moli_css_parse::css_declaration_value_has_valid_env_functions(trimmed)
    {
        return String::new();
    }
    if name == "content" {
        return normalize_content_specified_value(trimmed).unwrap_or_else(|| trimmed.to_owned());
    }
    if name == "font-family" {
        return normalize_cssom_font_family_value(trimmed).unwrap_or_else(|| trimmed.to_owned());
    }
    if name == "all" {
        return css_wide_keyword(trimmed).unwrap_or_default();
    }
    let serialized = if name.starts_with("--") {
        trimmed.to_owned()
    } else {
        moli_css_parse::normalize_cssom_component_value_serialization(trimmed)
            .unwrap_or_else(|| trimmed.to_owned())
    };
    let serialized = serialized.as_str();
    match name {
        "width" | "margin" | "min-width" | "max-width" | "padding" | "inset-inline-end"
        | "inset-inline-start" | "left" | "right" | "top" | "bottom" | "outline"
            if serialized == "0" =>
        {
            "0px".to_owned()
        }
        "accent-color" | "color" | "background-color" | "caret-color" | "outline-color"
            if simple_var_function_parts(serialized).is_some() =>
        {
            serialized.to_owned()
        }
        "accent-color" if serialized.eq_ignore_ascii_case("auto") => "auto".to_owned(),
        "border-color" => serialized.to_owned(),
        name if color_property_is_resolved_color(name)
            && !specified_color_value_is_valid(serialized) =>
        {
            String::new()
        }
        "width" | "height" | "min-width" | "max-width" if is_negative_length_like(serialized) => {
            String::new()
        }
        "font" => normalize_font_shorthand_specified_value(serialized)
            .unwrap_or_else(|| serialized.to_owned()),
        "flex" => normalize_cssom_flex_shorthand_value(serialized)
            .unwrap_or_else(|| serialized.to_owned()),
        "flex-basis" => {
            normalize_cssom_flex_basis_value(serialized).unwrap_or_else(|| serialized.to_owned())
        }
        "width" if serialized.starts_with("anchor-size(") => {
            normalize_anchor_size_function(serialized).unwrap_or_else(|| serialized.to_owned())
        }
        _ => serialized.to_owned(),
    }
}

pub(in crate::native_bridge::element::styles) fn normalize_style_value_with_base(
    name: &str,
    value: &str,
    _base_url: Option<&url::Url>,
) -> String {
    normalize_style_value(name, value)
}

fn normalize_font_shorthand_specified_value(value: &str) -> Option<String> {
    normalize_cssom_component_value_serialization_with_spaced_slash(value)
}

fn normalize_content_specified_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if cssom_value_is_attr_function(trimmed) {
        return Some(trimmed.to_owned());
    }
    if let Some(counter) = normalize_cssom_counter_function(trimmed) {
        return Some(counter);
    }
    moli_css_parse::normalize_cssom_component_value_serialization(trimmed)
}

pub(super) fn normalize_cssom_font_family_value(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let mut input = ParserInput::new(trimmed);
    let mut input = Parser::new(&mut input);
    let token = input
        .next_including_whitespace_and_comments()
        .cloned()
        .ok()?;
    if let Token::QuotedString(name) = token {
        while let Ok(token) = input.next_including_whitespace_and_comments() {
            if !matches!(token, Token::WhiteSpace(_) | Token::Comment(_)) {
                return moli_css_parse::normalize_cssom_component_value_serialization(trimmed);
            }
        }
        let name = name.to_string();
        if font_family_name_can_serialize_unquoted(&name) {
            return Some(name);
        }
        let mut quoted = String::new();
        serialize_string(&name, &mut quoted).ok()?;
        return Some(quoted);
    }
    moli_css_parse::normalize_cssom_component_value_serialization(trimmed)
}

fn font_family_name_can_serialize_unquoted(name: &str) -> bool {
    if name.is_empty()
        || name.trim() != name
        || name.chars().any(|ch| ch.is_whitespace() && ch != ' ')
    {
        return false;
    }
    let lowered = name.to_ascii_lowercase();
    if font_family_name_is_reserved(&lowered) {
        return false;
    }
    name.split(' ').all(font_family_ident_is_valid)
}

fn font_family_name_is_reserved(lowered: &str) -> bool {
    matches!(
        lowered,
        "serif"
            | "sans-serif"
            | "monospace"
            | "cursive"
            | "fantasy"
            | "system-ui"
            | "ui-serif"
            | "ui-sans-serif"
            | "ui-monospace"
            | "ui-rounded"
            | "math"
            | "fangsong"
            | "initial"
            | "inherit"
            | "unset"
            | "revert"
            | "revert-layer"
            | "revert-rule"
            | "default"
    )
}

fn font_family_ident_is_valid(ident: &str) -> bool {
    let mut chars = ident.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if first.is_ascii_digit() {
        return false;
    }
    if first == '-' && chars.clone().next().is_some_and(|ch| ch.is_ascii_digit()) {
        return false;
    }
    font_family_ident_char_is_valid(first) && chars.all(font_family_ident_char_is_valid)
}

fn font_family_ident_char_is_valid(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || !ch.is_ascii()
}

fn normalize_anchor_size_function(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let inner = trimmed
        .strip_prefix("anchor-size(")?
        .strip_suffix(')')?
        .trim();
    let (head, fallback) = match split_top_level_once(inner, ',') {
        Some((head, fallback)) => (head.trim(), Some(fallback.trim())),
        None => (inner, None),
    };
    let tokens = head.split_whitespace().collect::<Vec<_>>();
    let canonical_head = match tokens.as_slice() {
        [single] => (*single).to_owned(),
        [first, second] if first.starts_with("--") || !second.starts_with("--") => {
            format!("{first} {second}")
        }
        [first, second] => format!("{second} {first}"),
        _ => head.to_owned(),
    };
    let fallback = fallback
        .map(|value| normalize_anchor_size_function(value).unwrap_or_else(|| value.to_owned()));
    Some(match fallback {
        Some(fallback) => format!("anchor-size({canonical_head}, {fallback})"),
        None => format!("anchor-size({canonical_head})"),
    })
}

fn split_top_level_once(input: &str, needle: char) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut escape = false;
    for (index, ch) in input.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match ch {
            '\\' if quote.is_some() => {
                escape = true;
            }
            '"' | '\'' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                }
            }
            '(' if quote.is_none() => depth += 1,
            ')' if quote.is_none() && depth > 0 => depth -= 1,
            _ if ch == needle && quote.is_none() && depth == 0 => {
                return Some((&input[..index], &input[index + ch.len_utf8()..]));
            }
            _ => {}
        }
    }
    None
}

fn cssom_value_is_attr_function(value: &str) -> bool {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    matches!(
        input.next_including_whitespace_and_comments(),
        Ok(Token::Function(name)) if name.eq_ignore_ascii_case("attr")
    )
}

fn normalize_cssom_counter_function(value: &str) -> Option<String> {
    let mut input = ParserInput::new(value);
    let mut input = Parser::new(&mut input);
    let Ok(Token::Function(name)) = input.next_including_whitespace_and_comments() else {
        return None;
    };
    if !name.eq_ignore_ascii_case("counter") {
        return None;
    }
    let (counter_name, style): (String, Option<String>) = input
        .parse_nested_block(|input| {
            let counter_name = input.expect_ident_cloned()?.to_string();
            let style = if input.is_exhausted() {
                None
            } else {
                input.expect_comma()?;
                let style = input.expect_ident_cloned()?.to_string();
                input.expect_exhausted()?;
                Some(style)
            };
            Ok::<_, cssparser::ParseError<'_, ()>>((counter_name, style))
        })
        .ok()?;
    input.expect_exhausted().ok()?;
    if style.is_some_and(|style| style.eq_ignore_ascii_case("decimal")) {
        return Some(format!("counter({counter_name})"));
    }
    None
}

fn is_negative_length_like(value: &str) -> bool {
    let value = value.trim_start();
    value.starts_with('-')
        && value
            .chars()
            .nth(1)
            .is_some_and(|ch| ch.is_ascii_digit() || ch == '.')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SimpleVarFunction {
    pub(super) name: String,
    pub(super) fallback: Option<String>,
}

pub(super) fn simple_var_function_parts(value: &str) -> Option<SimpleVarFunction> {
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    parser.expect_function_matching("var").ok()?;
    let parts = parser
        .parse_nested_block(|input| {
            let name = input.expect_ident_cloned()?;
            let name = name.to_string();
            let fallback = if input.is_exhausted() {
                None
            } else {
                input.expect_comma()?;
                Some(
                    simple_var_fallback_component_text(input)
                        .ok_or_else(|| input.new_custom_error(()))?,
                )
            };
            Ok::<_, cssparser::ParseError<'_, ()>>(SimpleVarFunction { name, fallback })
        })
        .ok()?;
    parser.expect_exhausted().ok()?;
    parts.name.starts_with("--").then_some(parts)
}

fn simple_var_fallback_component_text(input: &mut Parser<'_, '_>) -> Option<String> {
    let start = input.position();
    while input.next_including_whitespace_and_comments().is_ok() {}
    let value = input.slice_from(start).trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub(super) fn inline_style_entry(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<StyleEntry> {
    let property = canonical_style_property_name(property);
    if property == "all" {
        return inline_all_style_entry(runtime, handle);
    }
    if let Some((shorthand, shorthand_index)) = box_shorthand_for_longhand(&property) {
        return inline_style_entries_for_property(runtime, handle, |entry| {
            if entry.name == property {
                Some(entry.clone())
            } else if entry.name == "all" && all_shorthand_applies_to(&property) {
                Some(StyleEntry {
                    name: "all".to_owned(),
                    value: entry.value.clone(),
                    priority: entry.priority,
                })
            } else if entry.name == shorthand {
                if moli_css_parse::css_value_may_contain_var_function(&entry.value) {
                    return Some(StyleEntry {
                        name: property.clone(),
                        value: String::new(),
                        priority: entry.priority,
                    });
                }
                box_shorthand_component(&entry.value, shorthand_index).map(|value| StyleEntry {
                    name: property.clone(),
                    value,
                    priority: entry.priority,
                })
            } else {
                None
            }
        });
    }
    inline_style_entries_for_property(runtime, handle, |entry| match entry.name.as_str() {
        name if name == property => Some(entry.clone()),
        "all" if all_shorthand_applies_to(&property) => Some(StyleEntry {
            name: "all".to_owned(),
            value: entry.value.clone(),
            priority: entry.priority,
        }),
        _ => None,
    })
}

fn inline_all_style_entry(runtime: &JsContextHost, handle: DomHandle) -> Option<StyleEntry> {
    let entries = style_entries(runtime, handle);
    let all_index = entries
        .iter()
        .enumerate()
        .rev()
        .find(|(_, entry)| entry.name == "all")
        .map(|(index, _)| index)?;
    let all = entries[all_index].clone();
    let overridden = entries.iter().skip(all_index + 1).any(|entry| {
        all_shorthand_applies_to(&entry.name)
            && (entry.priority != all.priority || entry.value != all.value)
    });
    (!overridden).then_some(all)
}

fn inline_style_entries_for_property(
    runtime: &JsContextHost,
    handle: DomHandle,
    candidate: impl Fn(&StyleEntry) -> Option<StyleEntry>,
) -> Option<StyleEntry> {
    let mut normal = None;
    let mut important = None;
    style_entries(runtime, handle)
        .into_iter()
        .filter_map(|entry| candidate(&entry))
        .for_each(|entry| {
            if entry.priority {
                important = Some(entry);
            } else {
                normal = Some(entry);
            }
        });
    important.or(normal)
}

fn box_shorthand_for_longhand(property: &str) -> Option<(&'static str, usize)> {
    Some(match property {
        "margin-top" => ("margin", 0),
        "margin-right" => ("margin", 1),
        "margin-bottom" => ("margin", 2),
        "margin-left" => ("margin", 3),
        "padding-top" => ("padding", 0),
        "padding-right" => ("padding", 1),
        "padding-bottom" => ("padding", 2),
        "padding-left" => ("padding", 3),
        "overscroll-behavior-x" => ("overscroll-behavior", 0),
        "overscroll-behavior-y" => ("overscroll-behavior", 1),
        _ => return None,
    })
}

pub(super) fn box_shorthand_component(value: &str, shorthand_index: usize) -> Option<String> {
    let components = box_shorthand_value_components(value)?;
    match components.as_slice() {
        [single] => Some(single.clone()),
        [vertical, _horizontal] if shorthand_index == 0 || shorthand_index == 2 => {
            Some(vertical.clone())
        }
        [_vertical, horizontal] => Some(horizontal.clone()),
        [top, _right, _bottom] if shorthand_index == 0 => Some(top.clone()),
        [_top, right, _bottom] if shorthand_index == 1 || shorthand_index == 3 => {
            Some(right.clone())
        }
        [_top, _right, bottom] => Some(bottom.clone()),
        [top, _right, _bottom, _left] if shorthand_index == 0 => Some(top.clone()),
        [_top, right, _bottom, _left] if shorthand_index == 1 => Some(right.clone()),
        [_top, _right, bottom, _left] if shorthand_index == 2 => Some(bottom.clone()),
        [_top, _right, _bottom, left] => Some(left.clone()),
        _ => None,
    }
}

pub(in crate::native_bridge::element::styles::declaration) fn computed_style_default_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> String {
    let property = property.to_ascii_lowercase();
    match property.as_str() {
        "display" => runtime
            .dom_host()
            .node(handle)
            .and_then(Node::as_element)
            .map(|element| match element.local_name() {
                "table" => "table",
                "thead" => "table-header-group",
                "tbody" => "table-row-group",
                "tfoot" => "table-footer-group",
                "col" => "table-column",
                "colgroup" => "table-column-group",
                "tr" => "table-row",
                "td" | "th" => "table-cell",
                "caption" => "table-caption",
                "div" | "body" | "html" | "p" | "section" | "article" | "header" | "footer"
                | "main" | "nav" | "ul" | "ol" | "li" | "form" => "block",
                "slot" => "contents",
                _ => "inline",
            })
            .unwrap_or("inline")
            .to_owned(),
        "visibility" => "visible".to_owned(),
        "will-change" => "auto".to_owned(),
        "zoom" => "1".to_owned(),
        "direction" => "ltr".to_owned(),
        "unicode-bidi" => "normal".to_owned(),
        "container-name" => "none".to_owned(),
        "container-type" => "normal".to_owned(),
        "container" => "none".to_owned(),
        "bookmark-level" => "none".to_owned(),
        "bookmark-state" => "open".to_owned(),
        "color-scheme" => inherited_computed_style_value(runtime, handle, "color-scheme", "normal"),
        "forced-color-adjust" => {
            inherited_computed_style_value(runtime, handle, "forced-color-adjust", "auto")
        }
        "opacity" => "1".to_owned(),
        "pointer-events" => {
            inherited_computed_style_value(runtime, handle, "pointer-events", "auto")
        }
        "touch-action" => "auto".to_owned(),
        "accent-color" => "auto".to_owned(),
        "appearance" | "-webkit-appearance" => "none".to_owned(),
        "color" => inherited_computed_style_value(runtime, handle, "color", "rgb(0, 0, 0)"),
        "font-size" => inherited_computed_style_value(runtime, handle, "font-size", "16px"),
        "font-style" => "normal".to_owned(),
        "font-variant" => "normal".to_owned(),
        "font-variant-alternates"
        | "font-variant-caps"
        | "font-variant-east-asian"
        | "font-variant-emoji"
        | "font-variant-ligatures"
        | "font-variant-numeric"
        | "font-variant-position" => "normal".to_owned(),
        "font-weight" => "400".to_owned(),
        "line-height" => "normal".to_owned(),
        "link-parameters" => "none".to_owned(),
        "content" => "normal".to_owned(),
        "content-visibility" => "visible".to_owned(),
        "background-color" => "rgba(0, 0, 0, 0)".to_owned(),
        "background-attachment" => "scroll".to_owned(),
        "background-blend-mode" | "mix-blend-mode" => "normal".to_owned(),
        "background-image" => "none".to_owned(),
        "background-position-x"
        | "background-position-y"
        | "mask-position-x"
        | "mask-position-y" => "0%".to_owned(),
        "background" => "none".to_owned(),
        "alignment-baseline" => "baseline".to_owned(),
        "baseline-source" => "auto".to_owned(),
        "border-collapse" => "separate".to_owned(),
        "border-image" => "none".to_owned(),
        "caption-side" => "top".to_owned(),
        "clear" => "none".to_owned(),
        "clip" => "auto".to_owned(),
        "empty-cells" => "show".to_owned(),
        "isolation" => "auto".to_owned(),
        "mask" => "none".to_owned(),
        "-webkit-text-stroke" => {
            let color = inherited_computed_style_value(runtime, handle, "color", "rgb(0, 0, 0)");
            format!("0px {color}")
        }
        "-webkit-text-stroke-color" => {
            inherited_computed_style_value(runtime, handle, "color", "rgb(0, 0, 0)")
        }
        "-webkit-text-stroke-width" => "0px".to_owned(),
        "text-decoration-fill" | "text-decoration-stroke" => "match-text".to_owned(),
        "text-decoration-inset" => "0px".to_owned(),
        "text-decoration-line" => "none".to_owned(),
        "text-decoration-skip-ink" => "auto".to_owned(),
        "text-decoration-skip-spaces" => "start end".to_owned(),
        "text-decoration-style" => "solid".to_owned(),
        "text-emphasis-style" => "none".to_owned(),
        "text-emphasis-color" => {
            inherited_computed_style_value(runtime, handle, "color", "rgb(0, 0, 0)")
        }
        "text-emphasis-position" => "auto".to_owned(),
        "text-shadow" => "none".to_owned(),
        "text-transform" => "none".to_owned(),
        "text-underline-position" => "auto".to_owned(),
        "text-decoration-thickness" | "text-underline-offset" => "auto".to_owned(),
        "border" | "border-bottom" | "border-left" | "border-right" | "border-top" => {
            "medium none currentcolor".to_owned()
        }
        "border-radius" => "0px".to_owned(),
        "-webkit-border-radius" => "0px".to_owned(),
        "font" => "16px sans-serif".to_owned(),
        "flex" | "-webkit-flex" => "0 1 auto".to_owned(),
        "flex-flow" | "-webkit-flex-flow" => "row nowrap".to_owned(),
        "gap" | "row-gap" | "column-gap" | "place-content" => "normal".to_owned(),
        "grid-column" | "grid-column-start" | "grid-column-end" => "auto".to_owned(),
        "list-style" => "disc outside none".to_owned(),
        "list-style-image" => "none".to_owned(),
        "list-style-position" => "outside".to_owned(),
        "list-style-type" => "disc".to_owned(),
        "outline" => "medium none currentcolor".to_owned(),
        "outline-style" => "none".to_owned(),
        "overscroll-behavior"
        | "overscroll-behavior-block"
        | "overscroll-behavior-inline"
        | "overscroll-behavior-x"
        | "overscroll-behavior-y" => "auto".to_owned(),
        "print-color-adjust" => {
            inherited_computed_style_value(runtime, handle, "print-color-adjust", "economy")
        }
        "quotes" => inherited_computed_style_value(runtime, handle, "quotes", "auto"),
        "scrollbar-color" => {
            inherited_computed_style_value(runtime, handle, "scrollbar-color", "auto")
        }
        "scrollbar-width" => "auto".to_owned(),
        "text-size-adjust" => {
            inherited_computed_style_value(runtime, handle, "text-size-adjust", "auto")
        }
        "left" | "right" | "top" | "bottom" => "auto".to_owned(),
        "block-size" => "auto".to_owned(),
        "orphans" | "widows" => "2".to_owned(),
        "page-break-after" | "page-break-before" | "page-break-inside" => "auto".to_owned(),
        "table-layout" => "auto".to_owned(),
        "transition" | "-webkit-transition" => "all".to_owned(),
        "transition-behavior" => "normal".to_owned(),
        "transition-delay" | "transition-duration" => "0s".to_owned(),
        "transition-property" => "all".to_owned(),
        "transition-timing-function" => "ease".to_owned(),
        "animation" | "-webkit-animation" => "none".to_owned(),
        "rotate" | "scale" | "transform" | "-webkit-transform" => "none".to_owned(),
        "-webkit-mask"
        | "-webkit-mask-box-image"
        | "-webkit-mask-box-image-source"
        | "-webkit-mask-image" => "none".to_owned(),
        "-webkit-mask-box-image-outset" | "-webkit-mask-box-image-slice" => "0".to_owned(),
        "-webkit-mask-box-image-repeat" => "stretch".to_owned(),
        "-webkit-mask-box-image-width" | "-webkit-mask-size" => "auto".to_owned(),
        "-webkit-mask-clip" | "-webkit-mask-origin" => "border-box".to_owned(),
        "-webkit-mask-composite" => "source-over".to_owned(),
        "-webkit-mask-position" => "0% 0%".to_owned(),
        "-webkit-mask-repeat" => "repeat".to_owned(),
        "-webkit-perspective" => "none".to_owned(),
        "user-select" | "-webkit-user-select" => "auto".to_owned(),
        "white-space" => "normal".to_owned(),
        _ => String::new(),
    }
}

pub(super) fn inherited_computed_style_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
    initial: &str,
) -> String {
    inherited_style_parent(runtime, handle)
        .map(|parent| style_property_value(runtime, parent, StyleMode::Computed, property))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| initial.to_owned())
}

pub(super) fn inherited_style_parent(
    runtime: &JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    flat_tree_element_parent(runtime, handle)
}

pub(crate) fn style_property_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    property: &str,
) -> String {
    if mode == StyleMode::Computed {
        return ComputedStyleRead::new(runtime, handle).property(property);
    }
    style_property_value_with_viewport_width(runtime, handle, mode, property, None)
}

fn normalize_specified_cssom_value(property: &str, value: String) -> String {
    let value = if property == "font-family" {
        normalize_cssom_font_family_value(&value).unwrap_or(value)
    } else {
        value
    };
    normalize_touch_action_serialization(property, &value)
}

pub(in crate::native_bridge::element::styles) fn style_property_value_with_viewport_width(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    property: &str,
    viewport_width: Option<f64>,
) -> String {
    let canonicalize = if mode == StyleMode::Computed {
        canonical_computed_cssom_query_property_name
    } else {
        canonical_specified_cssom_query_property_name
    };
    let Some(property) = canonicalize(property) else {
        return String::new();
    };
    if mode == StyleMode::Computed {
        let viewport = StyleViewport {
            width: viewport_width.or_else(|| runtime.style_viewport().width),
            ..runtime.style_viewport()
        };
        return computed_style_property_value_with_context(
            runtime,
            handle,
            &property,
            StyleComputationContext::new(viewport),
        );
    }
    if let Some(state) = runtime.element_inline_style_declaration_state(handle)
        && let Some(value) = inline_state_property_value_with_pdb(state, &property)
    {
        return normalize_specified_cssom_value(&property, value);
    }
    let entries = style_entries(runtime, handle);
    if let Some(value) = style_entries_property_value_with_pdb(&entries, &property) {
        return normalize_specified_cssom_value(&property, value);
    }
    if property == "overflow" {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        let overflow_x = inline_style_entry_for_inline_style(runtime, handle, "overflow-x")
            .map(|entry| entry.value);
        let overflow_y = inline_style_entry_for_inline_style(runtime, handle, "overflow-y")
            .map(|entry| entry.value);
        return match (overflow_x, overflow_y) {
            (Some(left), Some(right)) if left == right => left,
            (Some(left), Some(right)) => format!("{left} {right}"),
            _ => String::new(),
        };
    }
    if property == "overflow-x" || property == "overflow-y" {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "overflow") {
            let tokens = entry.value.split_whitespace().collect::<Vec<_>>();
            return match tokens.as_slice() {
                [single] => (*single).to_owned(),
                [left, right] if property == "overflow-x" => (*left).to_owned(),
                [left, right] if property == "overflow-y" => (*right).to_owned(),
                _ => String::new(),
            };
        }
        return String::new();
    }
    if property == "animation" {
        return inline_animation_shorthand_value(runtime, handle);
    }
    if property == "animation-range" {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property)
            && entry.name == property
        {
            return entry.value;
        }
        return inline_animation_range_shorthand_value(runtime, handle);
    }
    if property == "transition" {
        return inline_transition_shorthand_value(runtime, handle);
    }
    if property == "text-decoration" {
        return inline_text_decoration_shorthand_value(runtime, handle);
    }
    if property == "text-emphasis" {
        return inline_text_emphasis_shorthand_value(runtime, handle);
    }
    if property == "font-variant" {
        return inline_font_variant_shorthand_value(runtime, handle);
    }
    if property == "list-style" {
        return inline_list_style_shorthand_value(runtime, handle);
    }
    if property == "outline" {
        return inline_outline_shorthand_value(runtime, handle);
    }
    if property == "border" {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        return inline_border_shorthand_value(runtime, handle);
    }
    if let Some(side) = border_side_shorthand_property(&property) {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "border") {
            return entry.value;
        }
        let value = inline_border_side_shorthand_value(runtime, handle, side);
        if !value.is_empty() {
            return value;
        }
        return inline_border_side_component_shorthand_value(runtime, handle, side);
    }
    if border_color_property(&property) {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        if property == "border-color"
            && let Some(value) =
                inline_shorthand_value_from_longhands(runtime, handle, border_color_longhands())
        {
            return value;
        }
        if let Some(index) = border_color_property_index(&property)
            && let Some(value) =
                border_component_from_component_shorthand(runtime, handle, "border-color", index)
        {
            return value;
        }
        if let Some(color) = border_color_from_shorthand(runtime, handle) {
            return color;
        }
        return String::new();
    }
    if border_style_property(&property) {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        if property == "border-style"
            && let Some(value) =
                inline_shorthand_value_from_longhands(runtime, handle, border_style_longhands())
        {
            return value;
        }
        if let Some(index) = border_style_property_index(&property)
            && let Some(value) =
                border_component_from_component_shorthand(runtime, handle, "border-style", index)
        {
            return value;
        }
        if let Some(style) = border_style_from_shorthand(runtime, handle) {
            return style;
        }
        return String::new();
    }
    if border_width_property_index(&property).is_some() {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        if property == "border-width"
            && let Some(value) =
                inline_shorthand_value_from_longhands(runtime, handle, border_width_longhands())
        {
            return value;
        }
        if let Some(index) = border_width_property_index(&property).filter(|index| *index < 4)
            && let Some(value) =
                border_component_from_component_shorthand(runtime, handle, "border-width", index)
        {
            return value;
        }
        if let Some(value) = border_width_from_shorthand(runtime, handle, &property) {
            return value;
        }
        return String::new();
    }
    if let Some(longhands) = shorthand_longhands(&property) {
        if let Some((index, entry)) =
            inline_exact_style_entry_with_index(runtime, handle, &property)
        {
            if moli_css_parse::css_value_may_contain_var_function(&entry.value)
                && exact_shorthand_has_later_overriding_longhand(
                    runtime, handle, index, &entry, longhands,
                )
            {
                return String::new();
            }
            return compress_box_shorthand_value(&entry.value);
        }
        let mut values = Vec::with_capacity(longhands.len());
        let mut priority = None;
        for longhand in longhands {
            let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, longhand) else {
                return String::new();
            };
            if priority.is_some_and(|current| current != entry.priority) {
                return String::new();
            }
            priority = Some(entry.priority);
            values.push(entry.value);
        }
        if values.iter().any(|value| value.is_empty()) {
            return String::new();
        }
        if values.iter().any(|value| css_wide_keyword(value).is_some())
            && !values.windows(2).all(|pair| pair[0] == pair[1])
        {
            return String::new();
        }
        return compress_box_components(&values).unwrap_or_default();
    }
    if let Some((shorthand, index)) = list_style_longhand_index(&property)
        && let Some(value) = fixed_shorthand_component(runtime, handle, shorthand, index)
    {
        return value;
    }
    if let Some((shorthand, index)) = outline_longhand_index(&property)
        && let Some(value) = fixed_shorthand_component(runtime, handle, shorthand, index)
    {
        return value;
    }
    if let Some(index) = font_variant_longhand_index(&property) {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "font-variant") {
            return font_variant_longhand_value_from_shorthand(&entry.value, index)
                .unwrap_or_default();
        }
        return String::new();
    }
    if property == "background-color" {
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, &property) {
            return entry.value;
        }
        if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "background")
            && let Some(color) = background_shorthand_color(&entry.value)
        {
            return color;
        }
        return String::new();
    }
    inline_style_entry_for_inline_style(runtime, handle, &property)
        .or_else(|| inset_shorthand_style_entry(runtime, handle, &property))
        .or_else(|| logical_inset_style_entry(runtime, handle, &property))
        .map(|entry| entry.value)
        .unwrap_or_default()
}

fn inline_exact_style_entry_with_index(
    runtime: &JsContextHost,
    handle: DomHandle,
    property: &str,
) -> Option<(usize, StyleEntry)> {
    let mut normal = None;
    let mut important = None;
    for (index, entry) in style_entries(runtime, handle).into_iter().enumerate() {
        if entry.name != property {
            continue;
        }
        if entry.priority {
            important = Some((index, entry));
        } else {
            normal = Some((index, entry));
        }
    }
    important.or(normal)
}

fn exact_shorthand_has_later_overriding_longhand(
    runtime: &JsContextHost,
    handle: DomHandle,
    shorthand_index: usize,
    shorthand: &StyleEntry,
    longhands: &[&str],
) -> bool {
    style_entries(runtime, handle)
        .into_iter()
        .enumerate()
        .skip(shorthand_index + 1)
        .any(|(_, entry)| {
            longhands.contains(&entry.name.as_str()) && (!shorthand.priority || entry.priority)
        })
}

fn inline_shorthand_value_from_longhands(
    runtime: &JsContextHost,
    handle: DomHandle,
    longhands: &[&str],
) -> Option<String> {
    let mut values = Vec::with_capacity(longhands.len());
    let mut priority = None;
    for longhand in longhands {
        let entry = inline_style_entry_for_inline_style(runtime, handle, longhand)?;
        if priority.is_some_and(|current| current != entry.priority) {
            return None;
        }
        priority = Some(entry.priority);
        values.push(entry.value);
    }
    if values.iter().any(|value| value.is_empty()) {
        return None;
    }
    if values.iter().any(|value| css_wide_keyword(value).is_some()) {
        let first = values.first()?;
        return values
            .iter()
            .all(|value| value == first)
            .then(|| first.clone());
    }
    compress_box_components(&values)
}

fn inline_text_emphasis_shorthand_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    let Some(style) = inline_style_entry_for_inline_style(runtime, handle, "text-emphasis-style")
    else {
        return String::new();
    };
    let Some(color) = inline_style_entry_for_inline_style(runtime, handle, "text-emphasis-color")
    else {
        return String::new();
    };
    if style.priority != color.priority {
        return String::new();
    }
    if style.value == color.value && css_wide_keyword(&style.value).is_some() {
        return style.value;
    }
    if css_wide_keyword(&style.value).is_some() || css_wide_keyword(&color.value).is_some() {
        return String::new();
    }
    format!("{} {}", style.value, color.value)
}

fn inline_font_variant_shorthand_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    let exact = inline_exact_style_entry_with_index(runtime, handle, "font-variant");
    let mut values = exact
        .as_ref()
        .and_then(|(_, entry)| font_variant_longhand_values_from_shorthand(&entry.value))
        .unwrap_or_else(|| vec!["normal".to_owned(); font_variant_longhands().len()]);
    let mut has_font_variant_state = exact.is_some();

    for (index, longhand) in font_variant_longhands().iter().enumerate() {
        let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, longhand) else {
            continue;
        };
        values[index] = entry.value;
        has_font_variant_state = true;
    }

    if !has_font_variant_state {
        return String::new();
    }
    serialize_font_variant_shorthand_values(&values).unwrap_or_default()
}

fn font_variant_longhand_index(property: &str) -> Option<usize> {
    font_variant_longhands()
        .iter()
        .position(|longhand| *longhand == property)
}

fn font_variant_longhand_value_from_shorthand(value: &str, index: usize) -> Option<String> {
    font_variant_longhand_values_from_shorthand(value).and_then(|values| values.get(index).cloned())
}

fn font_variant_longhand_values_from_shorthand(value: &str) -> Option<Vec<String>> {
    let value = value.trim();
    if let Some(keyword) = css_wide_keyword(value) {
        return Some(vec![keyword; font_variant_longhands().len()]);
    }
    let mut values = vec!["normal".to_owned(); font_variant_longhands().len()];
    match value.to_ascii_lowercase().as_str() {
        "normal" => {}
        "none" => values[0] = "none".to_owned(),
        "common-ligatures discretionary-ligatures" => values[0] = value.to_owned(),
        "small-caps" => values[1] = value.to_owned(),
        "historical-forms" => values[2] = value.to_owned(),
        "oldstyle-nums stacked-fractions" => values[3] = value.to_owned(),
        "ruby" => values[4] = value.to_owned(),
        "sub" | "super" => values[5] = value.to_owned(),
        "emoji" | "text" | "unicode" => values[6] = value.to_owned(),
        _ => return None,
    }
    Some(values)
}

pub(super) fn serialize_font_variant_shorthand_values(values: &[String]) -> Option<String> {
    if values.len() != font_variant_longhands().len() {
        return None;
    }
    if values.iter().any(|value| css_wide_keyword(value).is_some()) {
        let first = values.first()?;
        return values
            .iter()
            .all(|value| value == first)
            .then(|| first.clone());
    }
    let non_normal = values
        .iter()
        .filter(|value| !value.eq_ignore_ascii_case("normal"))
        .collect::<Vec<_>>();
    if non_normal.is_empty() {
        return Some("normal".to_owned());
    }
    if values[0].eq_ignore_ascii_case("none") {
        return (non_normal.len() == 1).then(|| "none".to_owned());
    }
    Some(
        non_normal
            .into_iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn inline_list_style_shorthand_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "list-style") {
        return entry.value;
    }
    inline_fixed_shorthand_value(
        runtime,
        handle,
        &["list-style-position", "list-style-type", "list-style-image"],
        serialize_list_style_shorthand_components,
    )
}

fn inline_outline_shorthand_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    if let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, "outline") {
        return entry.value;
    }
    inline_fixed_shorthand_value(
        runtime,
        handle,
        &["outline-color", "outline-style", "outline-width"],
        |values| Some(format!("{} {} {}", values[0], values[1], values[2])),
    )
}

fn inline_fixed_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    longhands: &[&str],
    serialize: impl Fn(&[String]) -> Option<String>,
) -> String {
    let mut values = Vec::with_capacity(longhands.len());
    let mut priority = None;
    for longhand in longhands {
        let Some(entry) = inline_style_entry_for_inline_style(runtime, handle, longhand) else {
            return String::new();
        };
        if priority.is_some_and(|current| current != entry.priority) {
            return String::new();
        }
        priority = Some(entry.priority);
        values.push(entry.value);
    }
    if values.iter().any(|value| value.is_empty()) {
        return String::new();
    }
    if values.iter().any(|value| css_wide_keyword(value).is_some()) {
        let Some(first) = values.first() else {
            return String::new();
        };
        if values.iter().all(|value| value == first) {
            return first.clone();
        }
        return String::new();
    }
    serialize(&values).unwrap_or_default()
}

fn serialize_list_style_shorthand_components(values: &[String]) -> Option<String> {
    let [position, list_type, image] = values else {
        return None;
    };
    let mut parts = Vec::new();
    if !position.eq_ignore_ascii_case("outside") {
        parts.push(position.as_str());
    }
    if !list_type.eq_ignore_ascii_case("disc") {
        parts.push(list_type.as_str());
    }
    if !image.eq_ignore_ascii_case("none") {
        parts.push(image.as_str());
    }
    Some(if parts.is_empty() {
        "outside disc".to_owned()
    } else {
        parts.join(" ")
    })
}

fn fixed_shorthand_component(
    runtime: &JsContextHost,
    handle: DomHandle,
    shorthand: &str,
    index: usize,
) -> Option<String> {
    let entry = inline_style_entry_for_inline_style(runtime, handle, shorthand)?;
    if css_wide_keyword(&entry.value).is_some() {
        return Some(entry.value);
    }
    let values = match shorthand {
        "list-style" => Vec::from(list_style_components_for_value(&entry.value)?),
        "outline" => outline_components(&entry.value)?,
        _ => return None,
    };
    values.get(index).cloned()
}

fn list_style_longhand_index(property: &str) -> Option<(&'static str, usize)> {
    match property {
        "list-style-position" => Some(("list-style", 0)),
        "list-style-type" => Some(("list-style", 1)),
        "list-style-image" => Some(("list-style", 2)),
        _ => None,
    }
}

fn outline_longhand_index(property: &str) -> Option<(&'static str, usize)> {
    match property {
        "outline-color" => Some(("outline", 0)),
        "outline-style" => Some(("outline", 1)),
        "outline-width" => Some(("outline", 2)),
        _ => None,
    }
}

fn list_style_components_for_value(value: &str) -> Option<[String; 3]> {
    let mut position = "outside".to_owned();
    let mut list_type = "disc".to_owned();
    let mut image = "none".to_owned();
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    for token in &tokens {
        if matches!(*token, "inside" | "outside") {
            position = (*token).to_owned();
        } else if *token == "none" {
            if tokens.len() == 1 {
                list_type = "none".to_owned();
            } else {
                image = "none".to_owned();
            }
        } else if token.starts_with("url(") {
            image = (*token).to_owned();
        } else {
            list_type = (*token).to_owned();
        }
    }
    Some([position, list_type, image])
}

fn outline_components(value: &str) -> Option<Vec<String>> {
    Some(vec![
        border_shorthand_color(value).unwrap_or_else(|| "currentcolor".to_owned()),
        border_shorthand_style(value).unwrap_or_else(|| "none".to_owned()),
        border_shorthand_width(value).unwrap_or_else(|| "medium".to_owned()),
    ])
}

fn inline_border_shorthand_value(runtime: &JsContextHost, handle: DomHandle) -> String {
    let top = inline_border_side_shorthand_value(runtime, handle, BorderSide::Top);
    if top.is_empty() {
        return String::new();
    }
    let sides = [
        inline_border_side_shorthand_value(runtime, handle, BorderSide::Right),
        inline_border_side_shorthand_value(runtime, handle, BorderSide::Bottom),
        inline_border_side_shorthand_value(runtime, handle, BorderSide::Left),
    ];
    if sides.iter().all(|side| side == &top) {
        top
    } else {
        String::new()
    }
}

fn inline_border_side_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    side: BorderSide,
) -> String {
    let Some(width) = inline_style_entry_for_inline_style(runtime, handle, side.width_property())
    else {
        return String::new();
    };
    let Some(style) = inline_style_entry_for_inline_style(runtime, handle, side.style_property())
    else {
        return String::new();
    };
    let Some(color) = inline_style_entry_for_inline_style(runtime, handle, side.color_property())
    else {
        return String::new();
    };
    serialize_border_side_shorthand(&width, &style, &color).unwrap_or_default()
}

fn inline_border_side_component_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    side: BorderSide,
) -> String {
    let width = inline_style_entry_for_inline_style(runtime, handle, side.width_property())
        .or_else(|| {
            border_component_style_entry_from_component_shorthand(
                runtime,
                handle,
                "border-width",
                side.width_property(),
                side.component_index(),
            )
        });
    let style = inline_style_entry_for_inline_style(runtime, handle, side.style_property())
        .or_else(|| {
            border_component_style_entry_from_component_shorthand(
                runtime,
                handle,
                "border-style",
                side.style_property(),
                side.component_index(),
            )
        });
    let color = inline_style_entry_for_inline_style(runtime, handle, side.color_property())
        .or_else(|| {
            border_component_style_entry_from_component_shorthand(
                runtime,
                handle,
                "border-color",
                side.color_property(),
                side.component_index(),
            )
        });
    let (Some(width), Some(style), Some(color)) = (width, style, color) else {
        return String::new();
    };
    serialize_border_side_shorthand(&width, &style, &color).unwrap_or_default()
}

pub(super) fn computed_border_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> String {
    let top = computed_border_side_shorthand_value(runtime, handle, BorderSide::Top, context);
    if top.is_empty() {
        return computed_style_default_value(runtime, handle, "border");
    }
    let sides = [
        computed_border_side_shorthand_value(runtime, handle, BorderSide::Right, context),
        computed_border_side_shorthand_value(runtime, handle, BorderSide::Bottom, context),
        computed_border_side_shorthand_value(runtime, handle, BorderSide::Left, context),
    ];
    if sides.iter().all(|side| side == &top) {
        top
    } else {
        String::new()
    }
}

pub(super) fn computed_border_side_shorthand_value(
    runtime: &JsContextHost,
    handle: DomHandle,
    side: BorderSide,
    context: StyleComputationContext,
) -> String {
    let width =
        computed_style_property_value_with_context(runtime, handle, side.width_property(), context);
    let style =
        computed_style_property_value_with_context(runtime, handle, side.style_property(), context);
    let color =
        computed_style_property_value_with_context(runtime, handle, side.color_property(), context);
    serialize_border_side_components(&width, &style, &color).unwrap_or_default()
}

fn serialize_border_side_shorthand(
    width: &StyleEntry,
    style: &StyleEntry,
    color: &StyleEntry,
) -> Option<String> {
    if width.priority != style.priority || width.priority != color.priority {
        return None;
    }
    serialize_border_side_components(&width.value, &style.value, &color.value)
}

fn serialize_border_side_components(width: &str, style: &str, color: &str) -> Option<String> {
    let css_wide_keywords = [width, style, color]
        .iter()
        .map(|value| css_wide_keyword(value))
        .collect::<Option<Vec<_>>>();
    if [width, style, color]
        .iter()
        .any(|value| css_wide_keyword(value).is_some())
    {
        let keywords = css_wide_keywords?;
        let first = keywords.first()?.clone();
        return keywords
            .iter()
            .all(|keyword| keyword == &first)
            .then_some(first);
    }
    Some(format!("{width} {style} {color}"))
}

#[derive(Clone, Copy)]
pub(super) enum BorderSide {
    Top,
    Right,
    Bottom,
    Left,
}

impl BorderSide {
    fn component_index(self) -> usize {
        match self {
            Self::Top => 0,
            Self::Right => 1,
            Self::Bottom => 2,
            Self::Left => 3,
        }
    }

    fn width_property(self) -> &'static str {
        match self {
            Self::Top => "border-top-width",
            Self::Right => "border-right-width",
            Self::Bottom => "border-bottom-width",
            Self::Left => "border-left-width",
        }
    }

    fn style_property(self) -> &'static str {
        match self {
            Self::Top => "border-top-style",
            Self::Right => "border-right-style",
            Self::Bottom => "border-bottom-style",
            Self::Left => "border-left-style",
        }
    }

    fn color_property(self) -> &'static str {
        match self {
            Self::Top => "border-top-color",
            Self::Right => "border-right-color",
            Self::Bottom => "border-bottom-color",
            Self::Left => "border-left-color",
        }
    }
}

pub(super) fn border_side_shorthand_property(property: &str) -> Option<BorderSide> {
    Some(match property {
        "border-top" => BorderSide::Top,
        "border-right" => BorderSide::Right,
        "border-bottom" => BorderSide::Bottom,
        "border-left" => BorderSide::Left,
        _ => return None,
    })
}
