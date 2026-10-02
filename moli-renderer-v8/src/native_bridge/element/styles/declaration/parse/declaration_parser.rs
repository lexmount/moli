use super::pdb_compat::{
    css_value_uses_unresolved_cssom_storage, cssom_animation_property_write_uses_pdb,
    cssom_border_image_property_write_uses_pdb, cssom_border_property_write_uses_pdb,
    cssom_font_property_write_uses_pdb, cssom_font_variant_property_write_uses_pdb,
    cssom_ordinary_longhand_value_can_use_direct_pdb_write, cssom_outline_property_write_uses_pdb,
    cssom_overflow_property_write_uses_pdb, cssom_structured_property_write_uses_pdb,
    cssom_text_decoration_property_write_uses_pdb, cssom_text_emphasis_property_write_uses_pdb,
    cssom_transition_property_write_uses_pdb, cssom_webkit_text_stroke_property_write_uses_pdb,
    parse_style_property_entries_with_pdb, style_property_mutation_affected_names_with_pdb,
};
use super::property_access::serialize_font_variant_shorthand_values;
use super::*;

pub(crate) fn parse_style_property_entries_with_base(
    name: &str,
    value: &str,
    priority: bool,
    base_url: Option<&url::Url>,
) -> Option<ParsedStylePropertyEntries> {
    crate::style_engine::ensure_stylo_browser_compat_prefs();
    let name = canonical_style_property_name(name);
    if name.starts_with("--") {
        if !moli_css_parse::is_cssom_custom_property_name(&name) {
            return None;
        }
        let value = moli_css_parse::normalize_custom_property_specified_value(value)?;
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.clone(),
                value,
                priority,
            }],
            affected_names: vec![name],
        });
    }

    if !supported_declared_property(&name) {
        return None;
    }

    if crate::detached_css_style::css_style_declaration_is_shell_property(&name) {
        let value = crate::css_style::parse_css_style_shell_keyword(value)?;
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.clone(),
                value: value.to_owned(),
                priority,
            }],
            affected_names: vec![name],
        });
    }

    if mask_compat_property_name(&name)
        && !stylo_mask_property_name(&name)
        && !mask_compat_value_is_supported(&name, value)
    {
        return None;
    }
    if webkit_transform_origin_compat_property_name(&name)
        && !webkit_transform_origin_compat_value_is_supported(&name, value)
    {
        return None;
    }
    let property_write_uses_pdb = cssom_border_property_write_uses_pdb(&name)
        || cssom_border_image_property_write_uses_pdb(&name)
        || cssom_outline_property_write_uses_pdb(&name)
        || stylo_mask_property_name(&name)
        || cssom_overflow_property_write_uses_pdb(&name)
        || cssom_animation_property_write_uses_pdb(&name)
        || cssom_text_decoration_property_write_uses_pdb(&name, value)
        || cssom_text_emphasis_property_write_uses_pdb(&name)
        || cssom_font_property_write_uses_pdb(&name, value)
        || cssom_font_variant_property_write_uses_pdb(&name)
        || cssom_transition_property_write_uses_pdb(&name)
        || cssom_webkit_text_stroke_property_write_uses_pdb(&name);
    if property_write_uses_pdb {
        return parse_style_property_entries_with_pdb(&name, value, priority);
    }

    if moli_css_parse::css_value_may_contain_var_function(value) {
        let value = moli_css_parse::normalize_css_variable_specified_value(value)?;
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.clone(),
                value,
                priority,
            }],
            affected_names: vec![name],
        });
    }

    if moli_css_parse::css_value_may_contain_env_function(value) {
        let value = normalize_style_value_with_base(&name, value, base_url);
        if value.is_empty() {
            return None;
        }
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.clone(),
                value,
                priority,
            }],
            affected_names: vec![name],
        });
    }

    if name == "width"
        && value
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("anchor-size(")
    {
        let value = normalize_style_value_with_base(&name, value, base_url);
        if value.is_empty() {
            return None;
        }
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.clone(),
                value,
                priority,
            }],
            affected_names: vec![name],
        });
    }

    if cssom_resolved_base_fallback_write_uses_pdb(&name, value) {
        return parse_style_property_entries_with_pdb(&name, value, priority);
    }

    if name == "background-image" && background_image_value_requires_stylo_parser(value) {
        return parse_strict_style_property_entries(&name, value, priority, base_url);
    }

    if cssom_style_entry_requires_structured_parser(&name) {
        return parse_strict_style_property_entries(&name, value, priority, base_url);
    }

    parse_normalized_style_property_entries(&name, value, priority, base_url)
}

pub(super) fn cssom_resolved_base_fallback_write_uses_pdb(name: &str, value: &str) -> bool {
    !(css_value_uses_unresolved_cssom_storage(value)
        || moli_css_parse::css_value_may_contain_env_function(value)
        || name == "width"
            && value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("anchor-size("))
        && (cssom_structured_property_write_uses_pdb(name)
            || cssom_ordinary_longhand_value_can_use_direct_pdb_write(name, value))
}

pub(super) fn parse_strict_style_property_entries(
    name: &str,
    value: &str,
    priority: bool,
    base_url: Option<&url::Url>,
) -> Option<ParsedStylePropertyEntries> {
    if name == "all" {
        parse_style_property_with_stylo(name, value, base_url)?;
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.to_owned(),
                value: value.trim().to_ascii_lowercase(),
                priority,
            }],
            affected_names: vec![name.to_owned()],
        });
    }

    if name == "animation-timing-function" {
        parse_style_property_with_stylo(name, value, base_url)?;
        let value = normalize_transition_timing_function_list(value)?;
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.to_owned(),
                value,
                priority,
            }],
            affected_names: vec![name.to_owned()],
        });
    }

    if let Some(keyword) = css_wide_keyword(value)
        && let Some(longhands) = shorthand_longhands(name)
    {
        let mut affected_names = Vec::with_capacity(longhands.len() + 1);
        affected_names.push(name.to_owned());
        affected_names.extend(longhands.iter().map(|longhand| (*longhand).to_owned()));
        let entries = affected_names
            .iter()
            .filter(|affected| *affected != name)
            .map(|affected| StyleEntry {
                name: affected.clone(),
                value: keyword.clone(),
                priority,
            })
            .collect();
        return Some(ParsedStylePropertyEntries {
            entries,
            affected_names,
        });
    }

    if name == "transition" {
        let longhand_values = parse_transition_shorthand_entries(value)?;
        let entries = transition_shorthand_longhands()
            .iter()
            .zip(longhand_values)
            .map(|(longhand, values)| StyleEntry {
                name: (*longhand).to_owned(),
                value: values.join(", "),
                priority,
            })
            .collect::<Vec<_>>();
        let mut affected_names = vec![name.to_owned()];
        affected_names.extend(
            transition_shorthand_longhands()
                .iter()
                .map(|longhand| (*longhand).to_owned()),
        );
        return Some(ParsedStylePropertyEntries {
            entries,
            affected_names,
        });
    }

    if let Some(value) = normalize_transition_longhand(name, value) {
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.to_owned(),
                value,
                priority,
            }],
            affected_names: vec![name.to_owned()],
        });
    }

    if let Some(entries) = parse_transition_numeric_property_entries(name, value, priority) {
        return Some(entries);
    }

    if let Some(longhands) = shorthand_longhands(name)
        && matches!(name, "border-color" | "border-style")
    {
        if let Some(keyword) = css_wide_keyword(value) {
            let entries = longhands
                .iter()
                .map(|longhand| StyleEntry {
                    name: (*longhand).to_owned(),
                    value: keyword.clone(),
                    priority,
                })
                .collect();
            let mut affected_names = vec![name.to_owned()];
            affected_names.extend(longhands.iter().map(|longhand| (*longhand).to_owned()));
            return Some(ParsedStylePropertyEntries {
                entries,
                affected_names,
            });
        }
        let value = normalize_style_value_with_base(name, value, base_url);
        if value.is_empty() {
            return None;
        }
        let components = box_shorthand_components(&value)?;
        let entries = longhands
            .iter()
            .zip(components)
            .map(|(longhand, value)| StyleEntry {
                name: (*longhand).to_owned(),
                value,
                priority,
            })
            .collect();
        let mut affected_names = vec![name.to_owned()];
        affected_names.extend(longhands.iter().map(|longhand| (*longhand).to_owned()));
        return Some(ParsedStylePropertyEntries {
            entries,
            affected_names,
        });
    }

    if let Some(mut declarations) = parse_style_property_with_stylo(name, value, base_url) {
        let mut affected_names = style_property_expanded_affected_names_with_pdb(name)?;
        let mut entries = Vec::new();
        for declaration in declarations.drain().declarations {
            let PropertyDeclarationId::Longhand(id) = declaration.id() else {
                continue;
            };
            let mut value = CssString::new();
            declaration.to_css(&mut value).ok()?;
            entries.push(StyleEntry {
                name: id.name().to_owned(),
                value,
                priority,
            });
        }
        if cssom_shorthand_store_should_preserve_property_entry(name) && !affected_names.is_empty()
        {
            affected_names.insert(0, name.to_owned());
            affected_names.dedup();
            if let Some(entry) = cssom_preserved_shorthand_entry(name, value, priority, &entries) {
                if border_shorthand_resets_border_image(name) {
                    affected_names.push("border-image".to_owned());
                    affected_names.dedup();
                }
                return Some(ParsedStylePropertyEntries {
                    entries: vec![entry],
                    affected_names,
                });
            }
        }
        if !entries.is_empty() {
            return Some(ParsedStylePropertyEntries {
                entries,
                affected_names,
            });
        }
    }

    parse_css_numeric_property_entries(name, value, priority)
}

pub(super) fn parse_transition_numeric_property_entries(
    name: &str,
    value: &str,
    priority: bool,
) -> Option<ParsedStylePropertyEntries> {
    matches!(name, "transition-delay" | "transition-duration")
        .then(|| parse_css_numeric_property_entries(name, value, priority))
        .flatten()
}

pub(super) fn parse_animation_numeric_property_entries(
    name: &str,
    value: &str,
    priority: bool,
) -> Option<ParsedStylePropertyEntries> {
    matches!(
        name,
        "animation-delay" | "animation-duration" | "animation-iteration-count"
    )
    .then(|| parse_css_numeric_property_entries(name, value, priority))
    .flatten()
}

pub(super) fn cssom_shorthand_store_should_preserve_property_entry(name: &str) -> bool {
    matches!(
        name,
        "border"
            | "background"
            | "border-top"
            | "border-right"
            | "border-bottom"
            | "border-left"
            | "border-width"
            | "outline"
            | "font"
            | "font-variant"
    )
}

pub(super) fn cssom_preserved_shorthand_entry(
    name: &str,
    input_value: &str,
    priority: bool,
    longhand_entries: &[StyleEntry],
) -> Option<StyleEntry> {
    let value = css_wide_keyword(input_value)
        .or_else(|| cssom_preserved_shorthand_value(name, longhand_entries))
        .or_else(|| {
            let value = normalize_style_value_with_base(name, input_value, None);
            (!value.is_empty()).then_some(value)
        })?;
    Some(StyleEntry {
        name: name.to_owned(),
        value,
        priority,
    })
}

pub(super) fn cssom_preserved_shorthand_value(
    name: &str,
    longhand_entries: &[StyleEntry],
) -> Option<String> {
    if name == "border" {
        return border_shorthand_value_from_longhands(longhand_entries);
    }
    if let Some(prefix) = border_side_shorthand_prefix(name) {
        return border_side_shorthand_value_from_longhands(longhand_entries, prefix);
    }
    if name == "outline" {
        return outline_shorthand_value_from_longhands(longhand_entries);
    }
    if name == "font-variant" {
        return font_variant_shorthand_value_from_longhands(longhand_entries);
    }
    let longhands = shorthand_longhands(name)?;
    let values = longhands
        .iter()
        .map(|longhand| style_entry_value(longhand_entries, longhand))
        .collect::<Option<Vec<_>>>()?;
    if values.iter().any(|value| css_wide_keyword(value).is_some()) {
        let first = values.first()?;
        return values
            .iter()
            .all(|value| value == first)
            .then(|| first.clone());
    }
    compress_box_components(&values)
}

pub(super) fn border_shorthand_value_from_longhands(entries: &[StyleEntry]) -> Option<String> {
    let top = border_side_shorthand_value_from_longhands(entries, "border-top")?;
    let right = border_side_shorthand_value_from_longhands(entries, "border-right")?;
    let bottom = border_side_shorthand_value_from_longhands(entries, "border-bottom")?;
    let left = border_side_shorthand_value_from_longhands(entries, "border-left")?;
    (top == right && top == bottom && top == left).then_some(top)
}

pub(super) fn outline_shorthand_value_from_longhands(entries: &[StyleEntry]) -> Option<String> {
    let width = style_entry_value(entries, "outline-width")?;
    let style = style_entry_value(entries, "outline-style")?;
    let color = style_entry_value(entries, "outline-color")?;
    border_side_shorthand_value(width, style, color)
}

pub(super) fn font_variant_shorthand_value_from_longhands(
    entries: &[StyleEntry],
) -> Option<String> {
    let values = font_variant_longhands()
        .iter()
        .map(|longhand| style_entry_value(entries, longhand).or_else(|| Some("normal".to_owned())))
        .collect::<Option<Vec<_>>>()?;
    serialize_font_variant_shorthand_values(&values)
}

pub(super) fn border_side_shorthand_value_from_longhands(
    entries: &[StyleEntry],
    prefix: &str,
) -> Option<String> {
    let width = style_entry_value(entries, &format!("{prefix}-width"))?;
    let style = style_entry_value(entries, &format!("{prefix}-style"))?;
    let color = style_entry_value(entries, &format!("{prefix}-color"))?;
    border_side_shorthand_value(width, style, color)
}

pub(super) fn border_side_shorthand_value(
    width: String,
    style: String,
    color: String,
) -> Option<String> {
    if [width.as_str(), style.as_str(), color.as_str()]
        .iter()
        .any(|value| css_wide_keyword(value).is_some())
    {
        return (width == style && width == color).then_some(width);
    }
    let mut parts = Vec::new();
    if width != "medium" {
        parts.push(width);
    }
    if style != "none" {
        parts.push(style);
    }
    if color != "currentcolor" {
        parts.push(color);
    }
    Some(parts.join(" "))
}

pub(super) fn style_entry_value(entries: &[StyleEntry], name: &str) -> Option<String> {
    entries
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.value.clone())
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

pub(super) fn border_shorthand_resets_border_image(name: &str) -> bool {
    name == "border"
}

pub(super) fn border_side_shorthand_prefix(name: &str) -> Option<&'static str> {
    match name {
        "border-top" => Some("border-top"),
        "border-right" => Some("border-right"),
        "border-bottom" => Some("border-bottom"),
        "border-left" => Some("border-left"),
        _ => None,
    }
}

pub(super) fn style_property_expanded_affected_names_with_pdb(name: &str) -> Option<Vec<String>> {
    let mut affected_names = style_property_mutation_affected_names_with_pdb(name)?;
    if shorthand_longhands(name).is_some() {
        affected_names.retain(|affected| affected != name);
    }
    Some(affected_names)
}

pub(super) fn parse_style_property_with_stylo(
    name: &str,
    value: &str,
    base_url: Option<&url::Url>,
) -> Option<SourcePropertyDeclaration> {
    let base_url = base_url.cloned().unwrap_or_else(about_blank_url);
    let url_data = UrlExtraData::from(base_url);
    let property_id = PropertyId::parse_enabled_for_all_content(name).ok()?;
    let mut declarations = SourcePropertyDeclaration::default();
    parse_one_declaration_into(
        &mut declarations,
        property_id.clone(),
        value,
        Origin::Author,
        &url_data,
        None,
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        CssRuleType::Style,
    )
    .ok()?;
    Some(declarations)
}

pub(super) fn about_blank_url() -> url::Url {
    url::Url::parse("about:blank").expect("static about:blank URL should parse")
}

pub(super) fn parse_normalized_style_property_entries(
    name: &str,
    value: &str,
    priority: bool,
    base_url: Option<&url::Url>,
) -> Option<ParsedStylePropertyEntries> {
    let value = normalize_style_value_with_base(name, value, base_url);
    if value.is_empty() {
        return None;
    }
    if value_mixes_css_wide_keyword(&value) {
        return None;
    }
    if name == "font" {
        let mut affected_names = Vec::with_capacity(font_variant_longhands().len() + 2);
        affected_names.push(name.to_owned());
        affected_names.push("font-variant".to_owned());
        affected_names.extend(
            font_variant_longhands()
                .iter()
                .map(|longhand| (*longhand).to_owned()),
        );
        return Some(ParsedStylePropertyEntries {
            entries: vec![StyleEntry {
                name: name.to_owned(),
                value,
                priority,
            }],
            affected_names,
        });
    }
    let (entries, affected_names) = if let Some(longhands) = shorthand_longhands(name)
        && let Some(components) = box_shorthand_components(&value)
    {
        let entries = longhands
            .iter()
            .zip(components)
            .map(|(longhand, value)| StyleEntry {
                name: (*longhand).to_owned(),
                value,
                priority,
            })
            .collect();
        let mut affected_names = Vec::with_capacity(longhands.len() + 1);
        affected_names.push(name.to_owned());
        affected_names.extend(longhands.iter().map(|longhand| (*longhand).to_owned()));
        (entries, affected_names)
    } else {
        (
            vec![StyleEntry {
                name: name.to_owned(),
                value,
                priority,
            }],
            vec![name.to_owned()],
        )
    };
    Some(ParsedStylePropertyEntries {
        entries,
        affected_names,
    })
}

pub(super) fn value_mixes_css_wide_keyword(value: &str) -> bool {
    let mut input = cssparser::ParserInput::new(value);
    let mut input = cssparser::Parser::new(&mut input);
    let mut component_count = 0usize;
    let mut has_css_wide_keyword = false;
    while let Ok(token) = input.next_including_whitespace_and_comments().cloned() {
        match token {
            cssparser::Token::WhiteSpace(_) | cssparser::Token::Comment(_) => {}
            cssparser::Token::Ident(ident) => {
                component_count += 1;
                has_css_wide_keyword |= css_wide_keyword(ident.as_ref()).is_some();
            }
            _ => component_count += 1,
        }
    }
    has_css_wide_keyword && component_count > 1
}

pub(crate) fn cssom_style_entry_requires_structured_parser(name: &str) -> bool {
    css_math_value_property_requires_stylo_parser(name)
        || css_color_value_property_requires_stylo_parser(name)
        || name == "all"
        || name == "animation"
        || name.starts_with("animation-")
        || name == "background"
        || name == "background-blend-mode"
        || name == "bookmark-level"
        || name == "bookmark-state"
        || name == "color-scheme"
        || name == "column-rule-width"
        || name == "column-width"
        || name == "content"
        || name == "forced-color-adjust"
        || font_variant_longhands().contains(&name)
        || name == "grid-column"
        || name == "isolation"
        || name == "link-parameters"
        || name == "mix-blend-mode"
        || name == "overscroll-behavior"
        || name.starts_with("overscroll-behavior-")
        || name == "outline"
        || name == "orphans"
        || name == "page-break-after"
        || name == "page-break-before"
        || name == "page-break-inside"
        || name == "print-color-adjust"
        || name == "quotes"
        || name == "scroll-margin-top"
        || name == "scroll-padding-bottom"
        || name == "scroll-snap-align"
        || name == "scrollbar-color"
        || name == "scrollbar-width"
        || name == "shape-margin"
        || name == "text-size-adjust"
        || name == "text-decoration"
        || name.starts_with("text-decoration-")
        || name == "text-emphasis"
        || name.starts_with("text-emphasis-")
        || name == "text-shadow"
        || name == "text-underline-position"
        || name == "text-underline-offset"
        || name == "transition"
        || name.starts_with("transition-")
        || name == "-webkit-text-stroke"
        || name.starts_with("-webkit-text-stroke-")
        || name == "widows"
        || name == "will-change"
        || name == "zoom"
}

pub(super) fn css_color_value_property_requires_stylo_parser(name: &str) -> bool {
    matches!(
        name,
        "accent-color" | "background-color" | "caret-color" | "color"
    )
}

pub(super) fn background_image_value_requires_stylo_parser(value: &str) -> bool {
    let lower = value.trim_start().to_ascii_lowercase();
    lower.starts_with("image-set(") || lower.starts_with("-webkit-image-set(")
}

pub(super) fn css_math_value_property_requires_stylo_parser(name: &str) -> bool {
    matches!(
        name,
        "background-size"
            | "block-size"
            | "bottom"
            | "border"
            | "border-bottom"
            | "border-bottom-width"
            | "border-left"
            | "border-left-width"
            | "border-right"
            | "border-right-width"
            | "border-top"
            | "border-top-width"
            | "border-width"
            | "height"
            | "left"
            | "letter-spacing"
            | "margin"
            | "margin-bottom"
            | "margin-left"
            | "margin-right"
            | "margin-top"
            | "margin-block"
            | "margin-block-end"
            | "margin-block-start"
            | "margin-inline"
            | "margin-inline-end"
            | "margin-inline-start"
            | "max-height"
            | "max-width"
            | "min-height"
            | "min-width"
            | "opacity"
            | "padding"
            | "padding-bottom"
            | "padding-left"
            | "padding-right"
            | "padding-top"
            | "padding-block-end"
            | "padding-block-start"
            | "padding-inline-end"
            | "padding-inline-start"
            | "right"
            | "rotate"
            | "scale"
            | "tab-size"
            | "text-indent"
            | "top"
            | "transform"
            | "width"
            | "z-index"
    )
}

pub(super) fn parse_css_numeric_property_entries(
    name: &str,
    value: &str,
    priority: bool,
) -> Option<ParsedStylePropertyEntries> {
    let value = value.trim();
    let supported = match css_numeric_property_rule(name)? {
        CssNumericPropertyRule::TimeList { non_negative } => {
            css_time_list_is_supported(value, non_negative)
        }
        CssNumericPropertyRule::AnimationDurationList => {
            value.eq_ignore_ascii_case("auto") || css_time_list_is_supported(value, true)
        }
        CssNumericPropertyRule::AnimationIterationCountList => {
            css_animation_iteration_count_list_is_supported(value)
        }
    };
    supported.then(|| ParsedStylePropertyEntries {
        entries: vec![StyleEntry {
            name: name.to_owned(),
            value: value.to_owned(),
            priority,
        }],
        affected_names: vec![name.to_owned()],
    })
}

pub(super) fn normalize_transition_longhand(name: &str, value: &str) -> Option<String> {
    let value = value.trim();
    if css_wide_keyword(value).is_some() {
        return Some(value.to_ascii_lowercase());
    }
    match name {
        "transition-property" => normalize_transition_property_list(value),
        "transition-timing-function" => normalize_transition_timing_function_list(value),
        "transition-behavior" => normalize_transition_behavior_list(value),
        _ => None,
    }
}

pub(super) fn css_numeric_property_rule(name: &str) -> Option<CssNumericPropertyRule> {
    Some(match name {
        "animation-delay" | "transition-delay" => CssNumericPropertyRule::TimeList {
            non_negative: false,
        },
        "animation-duration" => CssNumericPropertyRule::AnimationDurationList,
        "transition-duration" => CssNumericPropertyRule::TimeList { non_negative: true },
        "animation-iteration-count" => CssNumericPropertyRule::AnimationIterationCountList,
        _ => return None,
    })
}

pub(super) fn css_time_list_is_supported(value: &str, non_negative: bool) -> bool {
    top_level_comma_separated_component_values(value)
        .filter(|components| !components.is_empty())
        .is_some_and(|components| {
            components.into_iter().all(|component| {
                let time = moli_css_parse::resolve_css_numeric(
                    &component,
                    moli_css_parse::CssNumericKind::Time,
                    moli_css_parse::CssNumericContext::supports_probe(),
                )
                .and_then(moli_css_parse::CssNumericValue::time_seconds);
                time.is_some() && (!non_negative || time.is_some_and(|seconds| seconds >= 0.0))
            })
        })
}

pub(super) fn css_animation_iteration_count_list_is_supported(value: &str) -> bool {
    top_level_comma_separated_component_values(value)
        .filter(|components| !components.is_empty())
        .is_some_and(|components| {
            components.into_iter().all(|component| {
                component.eq_ignore_ascii_case("infinite")
                    || moli_css_parse::resolve_css_numeric(
                        &component,
                        moli_css_parse::CssNumericKind::Number,
                        moli_css_parse::CssNumericContext::supports_probe(),
                    )
                    .and_then(moli_css_parse::CssNumericValue::number)
                    .is_some_and(|value| value >= 0.0)
            })
        })
}
