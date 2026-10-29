use cssparser::serialize_identifier;

pub fn canonical_style_property_name(property: &str) -> String {
    let property = property.trim();
    if property.starts_with("--") {
        return property.to_owned();
    }
    if property == "cssFloat" {
        return "float".to_owned();
    }
    let lowered = property.to_ascii_lowercase();
    match lowered.as_str() {
        "font-width" => "font-stretch".to_owned(),
        "color-adjust" => "print-color-adjust".to_owned(),
        "-webkit-align-content" => "align-content".to_owned(),
        "-webkit-align-items" => "align-items".to_owned(),
        "-webkit-align-self" => "align-self".to_owned(),
        "-webkit-animation" => "animation".to_owned(),
        "-webkit-animation-delay" => "animation-delay".to_owned(),
        "-webkit-animation-direction" => "animation-direction".to_owned(),
        "-webkit-animation-duration" => "animation-duration".to_owned(),
        "-webkit-animation-fill-mode" => "animation-fill-mode".to_owned(),
        "-webkit-animation-iteration-count" => "animation-iteration-count".to_owned(),
        "-webkit-animation-name" => "animation-name".to_owned(),
        "-webkit-animation-play-state" => "animation-play-state".to_owned(),
        "-webkit-animation-timing-function" => "animation-timing-function".to_owned(),
        "-webkit-appearance" => "appearance".to_owned(),
        "-webkit-backface-visibility" => "backface-visibility".to_owned(),
        "-webkit-background-clip" => "background-clip".to_owned(),
        "-webkit-background-origin" => "background-origin".to_owned(),
        "-webkit-background-size" => "background-size".to_owned(),
        "-webkit-box-shadow" => "box-shadow".to_owned(),
        "-webkit-border-radius" => "border-radius".to_owned(),
        "-webkit-border-top-left-radius" => "border-top-left-radius".to_owned(),
        "-webkit-border-top-right-radius" => "border-top-right-radius".to_owned(),
        "-webkit-border-bottom-right-radius" => "border-bottom-right-radius".to_owned(),
        "-webkit-border-bottom-left-radius" => "border-bottom-left-radius".to_owned(),
        "-webkit-box-sizing" => "box-sizing".to_owned(),
        "-webkit-flex" => "flex".to_owned(),
        "-webkit-flex-basis" => "flex-basis".to_owned(),
        "-webkit-flex-direction" => "flex-direction".to_owned(),
        "-webkit-flex-flow" => "flex-flow".to_owned(),
        "-webkit-flex-grow" => "flex-grow".to_owned(),
        "-webkit-flex-shrink" => "flex-shrink".to_owned(),
        "-webkit-flex-wrap" => "flex-wrap".to_owned(),
        "-webkit-filter" => "filter".to_owned(),
        "-webkit-justify-content" => "justify-content".to_owned(),
        "-webkit-order" => "order".to_owned(),
        "-webkit-perspective" => "perspective".to_owned(),
        "-webkit-perspective-origin" => "perspective-origin".to_owned(),
        "-webkit-transform" => "transform".to_owned(),
        "-webkit-transform-style" => "transform-style".to_owned(),
        "-webkit-transition" => "transition".to_owned(),
        "-webkit-transition-delay" => "transition-delay".to_owned(),
        "-webkit-transition-duration" => "transition-duration".to_owned(),
        "-webkit-transition-property" => "transition-property".to_owned(),
        "-webkit-transition-timing-function" => "transition-timing-function".to_owned(),
        "-webkit-user-select" => "user-select".to_owned(),
        _ => lowered,
    }
}

pub fn is_cssom_custom_property_name(property: &str) -> bool {
    property.starts_with("--") && property.len() > 2 && !property.chars().any(char::is_whitespace)
}

pub fn serialize_style_property_name(property: &str) -> String {
    if !property.starts_with("--") {
        return property.to_owned();
    }
    let mut output = String::new();
    serialize_identifier(property, &mut output).unwrap_or(());
    output
}

pub fn canonical_style_property_identifier(property: &str) -> String {
    let property = property.trim();
    if property.starts_with("--") {
        return property.to_owned();
    }
    if property == "cssFloat" {
        return "float".to_owned();
    }
    for prefix in ["WebKit", "Webkit", "webkit"] {
        if let Some(rest) = property.strip_prefix(prefix)
            && rest
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_uppercase())
        {
            let property = format!("-webkit-{}", camel_to_kebab(&decapitalize_ascii_head(rest)));
            return canonical_style_property_name(&property);
        }
    }
    let lowered = if property.contains('-') {
        property.to_ascii_lowercase()
    } else {
        camel_to_kebab(property).to_ascii_lowercase()
    };
    canonical_style_property_name(&lowered)
}

pub fn camel_case_style_property_name(name: &str) -> Option<String> {
    if name.starts_with("--") {
        return None;
    }
    if name == "float" {
        return Some("cssFloat".to_owned());
    }
    let mut output = String::with_capacity(name.len());
    let mut uppercase_next = false;
    for ch in name.chars() {
        if ch == '-' {
            uppercase_next = true;
            continue;
        }
        if uppercase_next {
            output.push(ch.to_ascii_uppercase());
            uppercase_next = false;
        } else {
            output.push(ch);
        }
    }
    (!output.is_empty()).then_some(output)
}

pub fn decapitalize_ascii_head(input: &str) -> String {
    let mut chars = input.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::new();
    out.push(first.to_ascii_lowercase());
    out.extend(chars);
    out
}

pub fn camel_to_kebab(property: &str) -> String {
    let mut out = String::new();
    for ch in property.chars() {
        if ch.is_ascii_uppercase() {
            if !out.is_empty() {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn split_important_priority(value: &str) -> (String, bool) {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let Some(important_start) = lower.rfind("important") else {
        return (trimmed.to_owned(), false);
    };
    if important_start + "important".len() != lower.len() {
        return (trimmed.to_owned(), false);
    }
    let before_important = &trimmed[..important_start];
    let before_important_trimmed = before_important.trim_end();
    if let Some(without_bang) = before_important_trimmed.strip_suffix('!') {
        return (without_bang.trim_end().to_owned(), true);
    }
    (trimmed.to_owned(), false)
}

pub fn unquote_css_string(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        if (bytes[0] == b'"' && bytes[trimmed.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[trimmed.len() - 1] == b'\'')
        {
            return trimmed[1..trimmed.len() - 1].to_owned();
        }
    }
    trimmed.to_owned()
}

pub fn escape_top_level_semicolons(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            output.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => {
                output.push(ch);
                escaped = true;
            }
            '"' | '\'' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                }
                output.push(ch);
            }
            '(' if quote.is_none() => {
                depth += 1;
                output.push(ch);
            }
            ')' if quote.is_none() => {
                depth = depth.saturating_sub(1);
                output.push(ch);
            }
            ';' if quote.is_none() && depth == 0 => {
                output.push('\\');
                output.push(ch);
            }
            _ => output.push(ch),
        }
    }
    output
}

pub fn unescape_top_level_semicolons(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if escaped {
            output.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quote.is_none() && depth == 0 && chars.peek() == Some(&';') => {
                let _ = chars.next();
                output.push(';');
            }
            '\\' => {
                output.push(ch);
                escaped = true;
            }
            '"' | '\'' => {
                if quote == Some(ch) {
                    quote = None;
                } else if quote.is_none() {
                    quote = Some(ch);
                }
                output.push(ch);
            }
            '(' if quote.is_none() => {
                depth += 1;
                output.push(ch);
            }
            ')' if quote.is_none() => {
                depth = depth.saturating_sub(1);
                output.push(ch);
            }
            _ => output.push(ch),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{
        camel_case_style_property_name, camel_to_kebab, canonical_style_property_identifier,
        decapitalize_ascii_head, escape_top_level_semicolons, is_cssom_custom_property_name,
        serialize_style_property_name, unescape_top_level_semicolons,
    };

    #[test]
    fn semicolon_round_trip() {
        let escaped = escape_top_level_semicolons("Hello; world!");
        assert_eq!(escaped, "Hello\\; world!");
        let back = unescape_top_level_semicolons(&escaped);
        assert_eq!(back, "Hello; world!");
    }

    #[test]
    fn style_property_identifier_matches_renderer_surface() {
        assert_eq!(canonical_style_property_identifier(" cssFloat "), "float");
        assert_eq!(
            canonical_style_property_identifier("backgroundColor"),
            "background-color"
        );
        assert_eq!(
            canonical_style_property_identifier("colorAdjust"),
            "print-color-adjust"
        );
        assert_eq!(
            canonical_style_property_identifier("WebkitTransition"),
            "transition"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitAlignContent"),
            "align-content"
        );
        assert_eq!(
            canonical_style_property_identifier("-webkit-align-items"),
            "align-items"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitBackgroundSize"),
            "background-size"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitBackfaceVisibility"),
            "backface-visibility"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitBackgroundOrigin"),
            "background-origin"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitBoxShadow"),
            "box-shadow"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitBorderRadius"),
            "border-radius"
        );
        assert_eq!(
            canonical_style_property_identifier("-webkit-border-radius"),
            "border-radius"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitBorderTopLeftRadius"),
            "border-top-left-radius"
        );
        assert_eq!(
            canonical_style_property_identifier("-webkit-border-top-right-radius"),
            "border-top-right-radius"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitBorderBottomRightRadius"),
            "border-bottom-right-radius"
        );
        assert_eq!(
            canonical_style_property_identifier("-webkit-border-bottom-left-radius"),
            "border-bottom-left-radius"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitTransform"),
            "transform"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitFilter"),
            "filter"
        );
        assert_eq!(
            canonical_style_property_identifier("-webkit-filter"),
            "filter"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitPerspective"),
            "perspective"
        );
        assert_eq!(
            canonical_style_property_identifier("-webkit-perspective"),
            "perspective"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitPerspectiveOrigin"),
            "perspective-origin"
        );
        assert_eq!(
            canonical_style_property_identifier("-webkit-perspective-origin"),
            "perspective-origin"
        );
        assert_eq!(
            canonical_style_property_identifier("-Webkit-Box-Sizing"),
            "box-sizing"
        );
        assert_eq!(canonical_style_property_identifier("webkitFlex"), "flex");
        assert_eq!(
            canonical_style_property_identifier("webkitFlexFlow"),
            "flex-flow"
        );
        assert_eq!(
            canonical_style_property_identifier("-webkit-flex-basis"),
            "flex-basis"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitFlexDirection"),
            "flex-direction"
        );
        assert_eq!(
            canonical_style_property_identifier("-webkit-justify-content"),
            "justify-content"
        );
        assert_eq!(canonical_style_property_identifier("webkitOrder"), "order");
        assert_eq!(
            canonical_style_property_identifier("webkitTransitionDuration"),
            "transition-duration"
        );
        assert_eq!(
            canonical_style_property_identifier("-webkit-animation"),
            "animation"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitAnimationTimingFunction"),
            "animation-timing-function"
        );
        assert_eq!(
            canonical_style_property_identifier("webkitTransformStyle"),
            "transform-style"
        );
        assert_eq!(canonical_style_property_identifier("--xY"), "--xY");
    }

    #[test]
    fn style_property_identifier_helpers_convert_names() {
        assert_eq!(
            camel_case_style_property_name("float").as_deref(),
            Some("cssFloat")
        );
        assert_eq!(
            camel_case_style_property_name("-webkit-transition").as_deref(),
            Some("WebkitTransition")
        );
        assert_eq!(camel_case_style_property_name("--custom"), None);
        assert_eq!(camel_to_kebab("WebkitTransition"), "webkit-transition");
        assert_eq!(decapitalize_ascii_head("Transition"), "transition");
        assert_eq!(decapitalize_ascii_head(""), "");
    }

    #[test]
    fn custom_property_names_serialize_as_css_identifiers() {
        assert_eq!(serialize_style_property_name("color"), "color");
        assert_eq!(serialize_style_property_name("--a;b"), r#"--a\;b"#);
        assert_eq!(serialize_style_property_name(r#"--\"#), r#"--\\"#);
        assert_eq!(serialize_style_property_name("--ab"), "--ab");
    }

    #[test]
    fn cssom_custom_property_name_requires_nonempty_suffix_without_whitespace() {
        assert!(is_cssom_custom_property_name("--x"));
        assert!(is_cssom_custom_property_name("---"));
        assert!(is_cssom_custom_property_name("--a;b"));
        assert!(is_cssom_custom_property_name("--\\"));

        assert!(!is_cssom_custom_property_name("--"));
        assert!(!is_cssom_custom_property_name("--x "));
        assert!(!is_cssom_custom_property_name("--x y"));
        assert!(!is_cssom_custom_property_name(" --x"));
    }
}
