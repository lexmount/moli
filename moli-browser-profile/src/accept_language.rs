use std::fmt::Write;

use icu_locale_core::Locale;

/// Extracts languages from an explicitly supplied HTTP header in wire order.
pub fn parse_accept_language(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter_map(|entry| {
            let tag = entry.split(';').next()?.trim();
            (!tag.is_empty()).then(|| tag.to_owned())
        })
        .collect()
}

pub(super) fn navigator_languages_from_header(value: &str) -> Vec<String> {
    let mut languages = parse_accept_language(value);
    languages.retain(|tag| tag != "*");
    if languages.is_empty() {
        default_languages()
    } else {
        languages
    }
}

pub(super) fn default_languages() -> Vec<String> {
    ["en-US", "en"].into_iter().map(str::to_owned).collect()
}

pub(super) fn validate_languages(languages: Vec<String>) -> Result<Vec<String>, String> {
    let mut normalized = Vec::with_capacity(languages.len());
    for tag in &languages {
        let tag = tag.trim_matches([' ', '\t']);
        let invalid_tag = || format!("invalid language tag {tag:?}");
        if tag.contains('_') {
            return Err(invalid_tag());
        }
        let locale = Locale::try_from_str(tag).map_err(|_| invalid_tag())?;
        let tag = locale.to_string();
        if !normalized.contains(&tag) {
            normalized.push(tag);
        }
    }
    if normalized.is_empty() {
        return Err("expected at least one language tag".to_owned());
    }
    Ok(normalized)
}

pub(super) fn accept_language_from_languages(languages: &[String]) -> String {
    generate_accept_language(expand_languages(languages))
}

fn base_language(tag: &str) -> &str {
    tag.split_once('-').map_or(tag, |(base, _)| base)
}

fn expand_languages(languages: &[String]) -> Vec<&str> {
    let mut expanded = Vec::new();
    for (index, tag) in languages.iter().map(String::as_str).enumerate() {
        if !expanded.contains(&tag) {
            expanded.push(tag);
        }
        let base = base_language(tag);
        let next_base = languages.get(index + 1).map(|next| base_language(next));
        if next_base != Some(base) && !matches!(base, "x" | "i") && !expanded.contains(&base) {
            expanded.push(base);
        }
    }
    expanded
}

pub(super) fn generate_accept_language<'a>(languages: impl IntoIterator<Item = &'a str>) -> String {
    let mut header = String::new();
    for (index, tag) in languages.into_iter().enumerate() {
        if index == 0 {
            header.push_str(tag);
        } else {
            let quality = 10_usize.saturating_sub(index).max(1);
            write!(header, ",{tag};q=0.{quality}").expect("writing to a String is infallible");
        }
    }
    header
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_base_languages_after_each_consecutive_language_family() {
        let languages = validate_languages(
            ["en-us", "en-GB", "fr", "zh-TW", "en"]
                .map(str::to_owned)
                .to_vec(),
        )
        .unwrap();
        assert_eq!(languages, ["en-US", "en-GB", "fr", "zh-TW", "en"]);
        assert_eq!(
            accept_language_from_languages(&languages),
            "en-US,en-GB;q=0.9,en;q=0.8,fr;q=0.7,zh-TW;q=0.6,zh;q=0.5"
        );
    }

    #[test]
    fn generated_quality_stays_at_one_tenth_after_the_tenth_language() {
        assert_eq!(
            generate_accept_language([
                "en", "fr", "es", "de", "it", "pt", "zh", "ja", "ko", "ru", "sv"
            ]),
            "en,fr;q=0.9,es;q=0.8,de;q=0.7,it;q=0.6,pt;q=0.5,zh;q=0.4,ja;q=0.3,ko;q=0.2,ru;q=0.1,sv;q=0.1"
        );
    }
}
