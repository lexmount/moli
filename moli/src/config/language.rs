use icu_locale_core::{Locale, subtags::script};
use moli_browser_profile::DEFAULT_ACCEPT_LANGUAGE;

pub(super) fn system_accept_language() -> String {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if let Some(language) = display_accept_language() {
        return language;
    }
    environment_accept_language(|name| std::env::var(name).ok())
        .unwrap_or_else(|| DEFAULT_ACCEPT_LANGUAGE.to_owned())
}

fn environment_accept_language(mut get: impl FnMut(&str) -> Option<String>) -> Option<String> {
    if let Some(language) =
        get("LANGUAGE").and_then(|list| list.split(':').find_map(accept_language_from_locale))
    {
        return Some(language);
    }

    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(get)
        .find(|value| !value.trim().is_empty())
        .as_deref()
        .and_then(accept_language_from_locale)
}

#[cfg(target_os = "macos")]
fn display_accept_language() -> Option<String> {
    macos_default_value("AppleLanguages")
        .and_then(|value| {
            value.lines().find_map(|line| {
                accept_language_from_locale(line.trim().trim_matches(['(', ')', ',', '"']))
            })
        })
        .or_else(|| {
            macos_default_value("AppleLocale")
                .as_deref()
                .and_then(accept_language_from_locale)
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
fn display_accept_language() -> Option<String> {
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
        .find_map(|value| accept_language_from_locale(&value))
}

fn accept_language_from_locale(value: &str) -> Option<String> {
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
        Some(primary)
    } else {
        Some(format!("{primary},{language};q=0.9"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_accept_language_preserves_scripts_and_regions() {
        for (locale, expected) in [
            ("zh_CN.UTF-8", "zh-CN,zh;q=0.9"),
            ("en_US.UTF-8@euro", "en-US,en;q=0.9"),
            ("es_419", "es-419,es;q=0.9"),
            ("zh-Hans-CN", "zh-CN,zh;q=0.9"),
            ("zh-Hans-HK", "zh-Hans-HK,zh;q=0.9"),
            ("zh-Hant-CN", "zh-Hant-CN,zh;q=0.9"),
            ("zh-Hans", "zh-Hans,zh;q=0.9"),
            ("SR-latn-rs", "sr-Latn-RS,sr;q=0.9"),
            ("sr_RS@latin", "sr-Latn-RS,sr;q=0.9"),
            ("sr_RS.UTF-8@latin", "sr-Latn-RS,sr;q=0.9"),
            ("sr_Cyrl_RS.UTF-8@latin", "sr-Cyrl-RS,sr;q=0.9"),
            ("uz_UZ@cyrillic", "uz-Cyrl-UZ,uz;q=0.9"),
            ("fr", "fr"),
        ] {
            assert_eq!(
                accept_language_from_locale(locale).as_deref(),
                Some(expected)
            );
        }
        for locale in ["C", "C.UTF-8", "POSIX", "", "en-abcdefghi"] {
            assert_eq!(accept_language_from_locale(locale), None);
        }
    }

    #[test]
    fn locale_environment_prefers_the_display_language_list() {
        let accept_language = |values: &[(&str, &str)]| {
            environment_accept_language(|key| {
                values
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| (*value).to_owned())
            })
        };
        assert_eq!(
            accept_language(&[("LANGUAGE", "C:zh_CN:en"), ("LANG", "en_US.UTF-8")]).as_deref(),
            Some("zh-CN,zh;q=0.9")
        );
        assert_eq!(
            accept_language(&[
                ("LANGUAGE", ":"),
                ("LC_MESSAGES", "sr_Latn_RS"),
                ("LANG", "en_US")
            ])
            .as_deref(),
            Some("sr-Latn-RS,sr;q=0.9")
        );
        assert_eq!(accept_language(&[("LC_ALL", "C"), ("LANG", "zh_CN")]), None);
    }
}
