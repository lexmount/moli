use super::numeric::is_valid_number_input_value;
use moli_html_input_temporal::{
    datetime_local_input_milliseconds, datetime_local_input_value_from_milliseconds,
    is_valid_date_input_value, is_valid_month_input_value, is_valid_time_input_value,
    is_valid_week_input_value,
};
use moli_html_input_type::InputType;

pub fn form_control_type_supports_intrinsic_validation(
    local_name: &str,
    input_type: Option<InputType>,
    button_type: Option<&str>,
) -> bool {
    match local_name {
        "input" => !matches!(
            input_type.unwrap_or_default(),
            InputType::Hidden | InputType::Button | InputType::Reset
        ),
        "select" | "textarea" => true,
        "button" => !matches!(
            button_type
                .unwrap_or("submit")
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "button" | "reset"
        ),
        _ => false,
    }
}

pub fn sanitize_input_value_for_type(input_type: InputType, value: &str) -> String {
    sanitize_input_value_for_type_with_multiple(input_type, value, false)
}

pub fn sanitize_input_value_for_type_with_multiple(
    input_type: InputType,
    value: &str,
    multiple: bool,
) -> String {
    match input_type {
        // Text-family — strip newlines / CR but leave whitespace runs alone.
        InputType::Text | InputType::Search | InputType::Tel | InputType::Password => {
            strip_input_value_line_breaks(value)
        }
        // URL — strip newlines AND trim leading/trailing ASCII whitespace.
        InputType::Url => {
            let stripped = strip_input_value_line_breaks(value);
            stripped.trim_matches(is_ascii_whitespace_char).to_owned()
        }
        InputType::Email => sanitize_email_input_value(value, multiple),
        InputType::Number if !is_valid_number_input_value(value) => String::new(),
        InputType::Date if !is_valid_date_input_value(value) => String::new(),
        InputType::Time if !is_valid_time_input_value(value) => String::new(),
        InputType::DatetimeLocal => datetime_local_input_milliseconds(value)
            .and_then(datetime_local_input_value_from_milliseconds)
            .unwrap_or_default(),
        InputType::Month if !is_valid_month_input_value(value) => String::new(),
        InputType::Week if !is_valid_week_input_value(value) => String::new(),
        // Range — sanitization: parse as float, clamp to [min, max], else
        // default to "(min+max)/2". Without min/max context here we use the
        // attribute-less default range 0..=100 (midpoint 50). Callers with
        // attribute context can override.
        InputType::Range => sanitize_range_value(value),
        // HTML-compatible color inputs accept CSS colors, discard alpha, and
        // expose an opaque lowercase sRGB simple color.
        InputType::Color => sanitize_color_value(value),
        // File — IDL value is always the empty string when set
        // programmatically; the user-selected files are the only path to a
        // non-empty file list.
        InputType::File => String::new(),
        _ => value.to_owned(),
    }
}

fn sanitize_email_input_value(value: &str, multiple: bool) -> String {
    let stripped = strip_input_value_line_breaks(value);
    if !multiple {
        return sanitize_email_address(stripped.trim_matches(is_ascii_whitespace_char));
    }
    stripped
        .split(',')
        .map(|address| address.trim_matches(is_ascii_whitespace_char))
        .map(sanitize_email_address)
        .collect::<Vec<String>>()
        .join(",")
}

fn sanitize_email_address(address: &str) -> String {
    let Some((local, domain)) = address.rsplit_once('@') else {
        return address.to_owned();
    };
    let Ok(url::Host::Domain(domain)) = url::Host::parse(domain) else {
        return address.to_owned();
    };
    format!("{local}@{domain}")
}

fn strip_input_value_line_breaks(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !matches!(ch, '\n' | '\r'))
        .collect()
}

fn is_ascii_whitespace_char(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\x0C')
}

fn sanitize_range_value(value: &str) -> String {
    // Default min=0, max=100 per spec — without attribute access here we
    // can only honour that default; element-aware callers should override
    // by computing the suggested default themselves.
    match value.trim_matches(is_ascii_whitespace_char).parse::<f64>() {
        Ok(parsed) if parsed.is_finite() => {
            // Clamp to [0, 100]; preserve integer formatting when the result
            // is an integer to match the HTML serializer for valid floats.
            let clamped = parsed.clamp(0.0, 100.0);
            if clamped == clamped.trunc() {
                (clamped as i64).to_string()
            } else {
                clamped.to_string()
            }
        }
        _ => "50".to_owned(),
    }
}

fn sanitize_color_value(value: &str) -> String {
    moli_css_parse::parse_css_color_to_opaque_srgb_hex(value)
        .unwrap_or_else(|| "#000000".to_owned())
}

pub fn input_type_has_value_sanitization(input_type: InputType) -> bool {
    matches!(
        input_type,
        InputType::Number
            | InputType::Date
            | InputType::Time
            | InputType::DatetimeLocal
            | InputType::Month
            | InputType::Week
    )
}

pub fn input_type_value_mismatch(input_type: InputType, value: &str, multiple: bool) -> bool {
    if value.is_empty() {
        return false;
    }
    match input_type {
        InputType::Email => email_value_type_mismatch(value, multiple),
        InputType::Url => url_value_type_mismatch(value),
        _ => false,
    }
}

pub fn email_value_type_mismatch(value: &str, multiple: bool) -> bool {
    if multiple {
        return value
            .split(',')
            .map(str::trim)
            .any(|address| address.is_empty() || !is_valid_email_address(address));
    }
    !is_valid_email_address(value.trim())
}

pub fn is_valid_email_address(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return false;
    }
    if !local.chars().all(is_email_atext_or_dot) {
        return false;
    }
    domain.split('.').all(is_valid_email_domain_label)
}

fn is_email_atext_or_dot(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || matches!(
            ch,
            '!' | '#'
                | '$'
                | '%'
                | '&'
                | '\''
                | '*'
                | '+'
                | '-'
                | '/'
                | '='
                | '?'
                | '^'
                | '_'
                | '`'
                | '{'
                | '|'
                | '}'
                | '~'
                | '.'
        )
}

fn is_valid_email_domain_label(label: &str) -> bool {
    if label.is_empty() || label.len() > 63 {
        return false;
    }
    let bytes = label.as_bytes();
    bytes[0].is_ascii_alphanumeric()
        && bytes[label.len() - 1].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
}

pub fn url_value_type_mismatch(value: &str) -> bool {
    url::Url::parse(value.trim()).is_err()
}

#[cfg(test)]
mod tests {
    use super::{sanitize_input_value_for_type, sanitize_input_value_for_type_with_multiple};
    use moli_html_input_type::InputType;

    const INITIAL: &str = "  foo\rbar  ";

    #[test]
    fn text_family_strips_cr_but_preserves_surrounding_whitespace() {
        for input_type in [
            InputType::Text,
            InputType::Search,
            InputType::Tel,
            InputType::Password,
        ] {
            assert_eq!(
                sanitize_input_value_for_type(input_type, INITIAL),
                "  foobar  ",
                "{input_type}: surrounding whitespace must survive"
            );
        }
    }

    #[test]
    fn url_email_strip_cr_then_trim_whitespace() {
        for input_type in [InputType::Url, InputType::Email] {
            assert_eq!(
                sanitize_input_value_for_type(input_type, INITIAL),
                "foobar",
                "{input_type}: must trim AND strip CR"
            );
            assert_eq!(sanitize_input_value_for_type(input_type, ""), "");
            // Already-trimmed values pass through unchanged.
            assert_eq!(
                sanitize_input_value_for_type(input_type, "foobar"),
                "foobar"
            );
        }
    }

    #[test]
    fn multiple_email_trims_each_comma_separated_address() {
        assert_eq!(
            sanitize_input_value_for_type_with_multiple(
                InputType::Email,
                "  first@example.com \r, second@example.test\n  ",
                true,
            ),
            "first@example.com,second@example.test"
        );
        assert_eq!(
            sanitize_input_value_for_type_with_multiple(
                InputType::Email,
                " first@example.com , , third@example.test ",
                true,
            ),
            "first@example.com,,third@example.test"
        );
        assert_eq!(
            sanitize_input_value_for_type(InputType::Email, " test@exämle.com "),
            "test@xn--exmle-hra.com"
        );
        assert_eq!(
            sanitize_input_value_for_type_with_multiple(
                InputType::Email,
                " test@exämle.com, user@お.com ",
                true,
            ),
            "test@xn--exmle-hra.com,user@xn--t8j.com"
        );
    }

    #[test]
    fn range_defaults_to_midpoint_for_invalid_input() {
        // "  foo\rbar  " parses as NaN -> default midpoint of 0..=100 = "50".
        assert_eq!(
            sanitize_input_value_for_type(InputType::Range, INITIAL),
            "50"
        );
        // Empty -> default.
        assert_eq!(sanitize_input_value_for_type(InputType::Range, ""), "50");
        // Valid float in-range round-trips and gets integer-serialised when
        // it lands exactly on an integer.
        assert_eq!(sanitize_input_value_for_type(InputType::Range, "42"), "42");
        // Out-of-range -> clamped.
        assert_eq!(
            sanitize_input_value_for_type(InputType::Range, "150"),
            "100"
        );
        assert_eq!(sanitize_input_value_for_type(InputType::Range, "-5"), "0");
    }

    #[test]
    fn color_parses_css_colors_and_defaults_to_black_for_invalid_input() {
        assert_eq!(
            sanitize_input_value_for_type(InputType::Color, INITIAL),
            "#000000"
        );
        assert_eq!(
            sanitize_input_value_for_type(InputType::Color, ""),
            "#000000"
        );
        assert_eq!(
            sanitize_input_value_for_type(InputType::Color, "red"),
            "#ff0000"
        );
        assert_eq!(
            sanitize_input_value_for_type(InputType::Color, "#FFAA00"),
            "#ffaa00"
        );
        assert_eq!(
            sanitize_input_value_for_type(InputType::Color, "#abc"),
            "#aabbcc"
        );
        assert_eq!(
            sanitize_input_value_for_type(InputType::Color, "color(display-p3 .5 0 0)"),
            "#8c0000"
        );
        assert_eq!(
            sanitize_input_value_for_type(InputType::Color, "not-a-color"),
            "#000000"
        );
    }

    #[test]
    fn file_always_yields_empty_string() {
        assert_eq!(sanitize_input_value_for_type(InputType::File, INITIAL), "");
        assert_eq!(
            sanitize_input_value_for_type(InputType::File, "anything"),
            ""
        );
    }

    #[test]
    fn states_without_sanitization_pass_value_through_unchanged() {
        for input_type in [
            InputType::Hidden,
            InputType::Checkbox,
            InputType::Radio,
            InputType::Submit,
            InputType::Reset,
            InputType::Button,
            InputType::Image,
        ] {
            assert_eq!(sanitize_input_value_for_type(input_type, INITIAL), INITIAL);
        }
    }

    #[test]
    fn invalid_temporal_values_collapse_to_empty_string() {
        for input_type in [
            InputType::Number,
            InputType::Date,
            InputType::Time,
            InputType::DatetimeLocal,
            InputType::Month,
            InputType::Week,
        ] {
            assert_eq!(sanitize_input_value_for_type(input_type, INITIAL), "");
        }
    }
}
