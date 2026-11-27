use std::cmp::Reverse;

use icu_locale_core::Locale;

use crate::window_surface::DEFAULT_ACCEPT_LANGUAGE;

/// Returns the concrete, positive-weight languages in preference order.
pub fn parse_accept_language(value: &str) -> Vec<String> {
    let mut preferred: Vec<_> = value
        .split(',')
        .filter_map(|entry| parse_range(entry).ok())
        .filter(|(tag, quality)| is_preferred_language(tag, *quality))
        .collect();
    preferred.sort_by_key(|(_, quality)| Reverse(*quality));
    preferred
        .into_iter()
        .map(|(tag, _)| tag.to_owned())
        .collect()
}

pub(super) fn navigator_languages(value: &str) -> Vec<String> {
    let preferred = parse_accept_language(value);
    if preferred.is_empty() {
        parse_accept_language(DEFAULT_ACCEPT_LANGUAGE)
    } else {
        preferred
    }
}

/// Validates an explicitly configured browser language preference.
pub fn validate_accept_language(value: &str) -> Result<(), String> {
    if value.chars().any(|c| c.is_control() && c != '\t') {
        return Err("Accept-Language must not contain control characters".to_owned());
    }
    let mut has_preferred_language = false;
    for entry in value.split(',') {
        let (tag, quality) = parse_range(entry)
            .map_err(|reason| format!("invalid Accept-Language entry {entry:?}: {reason}"))?;
        has_preferred_language |= is_preferred_language(tag, quality);
    }
    if !has_preferred_language {
        return Err(
            "Accept-Language requires at least one language tag with positive weight".to_owned(),
        );
    }
    Ok(())
}

fn is_preferred_language(tag: &str, quality: u16) -> bool {
    tag != "*" && quality > 0
}

fn parse_range(entry: &str) -> Result<(&str, u16), &'static str> {
    let entry = entry.trim_matches([' ', '\t']);
    let (tag, weight) = entry
        .split_once(';')
        .map_or((entry, None), |(tag, weight)| {
            (
                tag.trim_end_matches([' ', '\t']),
                Some(weight.trim_start_matches([' ', '\t'])),
            )
        });
    if tag != "*" && (tag.contains('_') || Locale::try_from_str(tag).is_err()) {
        return Err("expected a language tag such as zh-CN or *");
    }

    let quality = if let Some(weight) = weight {
        weight
            .split_once('=')
            .filter(|(name, _)| name.eq_ignore_ascii_case("q"))
            .and_then(|(_, value)| parse_quality(value))
            .ok_or("expected a single q value from 0 to 1 with at most three decimal digits")?
    } else {
        1000
    };
    Ok((tag, quality))
}

// RFC 9110 section 12.4.2 uses at most three fractional digits.
fn parse_quality(value: &str) -> Option<u16> {
    let (whole, fractional) = value.split_once('.').unwrap_or((value, ""));
    if fractional.len() > 3 || !fractional.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let digits = fractional.parse::<u16>().unwrap_or(0);
    match whole {
        "0" => Some(digits * 10u16.pow(3 - fractional.len() as u32)),
        "1" if digits == 0 => Some(1000),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigator_preferences_sort_by_weight_and_skip_http_only_ranges() {
        let header = "fr;q=0.25,en;q=0,sr-Latn-RS,de;q=0.250,*;q=0.9,zh-CN;q=1.000";
        assert_eq!(validate_accept_language(header), Ok(()));
        assert_eq!(
            parse_accept_language(header),
            ["sr-Latn-RS", "zh-CN", "fr", "de"]
        );
        assert!(parse_accept_language("en;q=0.000,*").is_empty());
    }
}
