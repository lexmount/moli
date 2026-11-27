use icu_locale_core::{Locale, subtags::script};

/// Detects the host language preference and its base language fallback.
pub fn system_languages() -> Option<Vec<String>> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if let Some(languages) = display_languages() {
        return Some(languages);
    }
    environment_languages(|name| std::env::var(name).ok())
}

fn environment_languages(mut get: impl FnMut(&str) -> Option<String>) -> Option<Vec<String>> {
    if let Some(languages) =
        get("LANGUAGE").and_then(|list| list.split(':').find_map(languages_from_locale))
    {
        return Some(languages);
    }

    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(get)
        .find(|value| !value.trim().is_empty())
        .as_deref()
        .and_then(languages_from_locale)
}

#[cfg(target_os = "macos")]
fn display_languages() -> Option<Vec<String>> {
    macos_default_value("AppleLanguages")
        .and_then(|value| {
            value.lines().find_map(|line| {
                languages_from_locale(line.trim().trim_matches(['(', ')', ',', '"']))
            })
        })
        .or_else(|| {
            macos_default_value("AppleLocale")
                .as_deref()
                .and_then(languages_from_locale)
        })
}

#[cfg(target_os = "macos")]
fn macos_default_value(key: &str) -> Option<String> {
    let output = std::process::Command::new("defaults")
        .args(["read", "-g", key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

#[cfg(target_os = "windows")]
fn display_languages() -> Option<Vec<String>> {
    use windows_sys::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};

    let mut count = 0;
    let mut length = 0;
    // SAFETY: The null buffer queries the size, and both output pointers are writable.
    let success = unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            std::ptr::null_mut(),
            &mut length,
        )
    };
    if success == 0 || length == 0 {
        return None;
    }

    let mut buffer = vec![0u16; length as usize];
    // SAFETY: The buffer has `length` UTF-16 code units, as required by the API.
    let success = unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    if success == 0 {
        return None;
    }
    buffer
        .split(|unit| *unit == 0)
        .take(count as usize)
        .filter_map(|units| String::from_utf16(units).ok())
        .find_map(|value| languages_from_locale(&value))
}

fn languages_from_locale(value: &str) -> Option<Vec<String>> {
    let value = value.trim();
    let (name, modifier) = value.split_once('@').unwrap_or((value, ""));
    let tag = name.split_once('.').map_or(name, |(tag, _)| tag).trim();
    if tag.eq_ignore_ascii_case("C") || tag.eq_ignore_ascii_case("POSIX") {
        return None;
    }
    let mut locale = Locale::try_from_str(&tag.replace('_', "-")).ok()?;
    if locale.id.script.is_none() {
        locale.id.script = match modifier {
            "latin" => Some(script!("Latn")),
            "cyrillic" => Some(script!("Cyrl")),
            _ => None,
        };
    }
    let language = locale.id.language.as_str();
    let mut primary = locale.to_string();
    // Keep the browser spelling used for the macOS Simplified Chinese locale.
    if primary == "zh-Hans-CN" {
        primary = "zh-CN".to_owned();
    }
    if primary == language {
        Some(vec![primary])
    } else {
        Some(vec![primary, language.to_owned()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_languages_preserve_scripts_and_regions() {
        let cases: &[(&str, &[&str])] = &[
            ("zh_CN.UTF-8", &["zh-CN", "zh"]),
            ("en_US.UTF-8@euro", &["en-US", "en"]),
            ("es_419", &["es-419", "es"]),
            ("zh-Hans-CN", &["zh-CN", "zh"]),
            ("zh-Hans-HK", &["zh-Hans-HK", "zh"]),
            ("zh-Hant-CN", &["zh-Hant-CN", "zh"]),
            ("zh-Hans", &["zh-Hans", "zh"]),
            ("SR-latn-rs", &["sr-Latn-RS", "sr"]),
            ("sr_RS@latin", &["sr-Latn-RS", "sr"]),
            ("sr_RS.UTF-8@latin", &["sr-Latn-RS", "sr"]),
            ("sr_Cyrl_RS.UTF-8@latin", &["sr-Cyrl-RS", "sr"]),
            ("uz_UZ@cyrillic", &["uz-Cyrl-UZ", "uz"]),
            ("fr", &["fr"]),
        ];
        for &(locale, expected) in cases {
            assert_eq!(languages_from_locale(locale).unwrap(), expected);
        }
        for locale in ["C", "C.UTF-8", "POSIX", "", "en-abcdefghi"] {
            assert_eq!(languages_from_locale(locale), None);
        }
    }

    #[test]
    fn locale_environment_prefers_the_display_language_list() {
        let languages = |values: &[(&str, &str)]| {
            environment_languages(|key| {
                values
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| (*value).to_owned())
            })
        };
        assert_eq!(
            languages(&[("LANGUAGE", "C:zh_CN:en"), ("LANG", "en_US.UTF-8")]).unwrap(),
            ["zh-CN", "zh"]
        );
        assert_eq!(
            languages(&[
                ("LANGUAGE", ":"),
                ("LC_MESSAGES", "sr_Latn_RS"),
                ("LANG", "en_US")
            ])
            .unwrap(),
            ["sr-Latn-RS", "sr"]
        );
        assert_eq!(languages(&[("LC_ALL", "C"), ("LANG", "zh_CN")]), None);
    }
}
