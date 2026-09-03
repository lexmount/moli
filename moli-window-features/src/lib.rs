//! Parsing for the HTML `window.open()` features argument.

use indexmap::IndexMap;
use style::attr::parse_integer;

/// The ordered map produced by tokenizing a `window.open()` features argument.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WindowFeatures {
    values: IndexMap<String, String>,
    entries: Vec<(String, String)>,
}

impl WindowFeatures {
    /// Returns the normalized value of `name`, if the feature was present.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    /// Returns whether the named feature is present and parses as true.
    pub fn boolean(&self, name: &str) -> bool {
        self.get(name).is_some_and(parse_boolean_feature_value)
    }

    /// Returns whether the tokenized feature map is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Iterates over normalized feature names and values in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

/// Parsed `window.open()` features consumed by the browser runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowOpenFeatures {
    x: Option<i32>,
    y: Option<i32>,
    width: Option<i32>,
    height: Option<i32>,
    menu_bar: bool,
    status_bar: bool,
    tool_bar: bool,
    scrollbars: bool,
    resizable: bool,
    is_popup: bool,
    noopener: bool,
    noreferrer: bool,
    background: bool,
    persistent: bool,
}

impl Default for WindowOpenFeatures {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: None,
            height: None,
            menu_bar: true,
            status_bar: true,
            tool_bar: true,
            scrollbars: true,
            resizable: true,
            is_popup: false,
            noopener: false,
            noreferrer: false,
            background: false,
            persistent: false,
        }
    }
}

impl WindowOpenFeatures {
    /// Parses the HTML `window.open()` features argument.
    pub fn parse(feature_string: &str) -> Self {
        let tokenized = tokenize_window_features(feature_string);
        let mut features = Self::default();
        if tokenized.entries.is_empty() {
            return features;
        }

        let mut ui_features_were_disabled = false;
        let mut explicit_popup = None;
        for (name, value) in &tokenized.entries {
            let value = feature_integer_value(value);
            if !ui_features_were_disabled
                && !matches!(name.as_str(), "noopener" | "noreferrer" | "attributionsrc")
            {
                ui_features_were_disabled = true;
                features.menu_bar = false;
                features.status_bar = false;
                features.tool_bar = false;
                features.scrollbars = false;
            }

            match name.as_str() {
                "left" => features.x = Some(value),
                "top" => features.y = Some(value),
                "width" => features.width = Some(value),
                "height" => features.height = Some(value),
                "popup" => explicit_popup = Some(value != 0),
                "menubar" => features.menu_bar = value != 0,
                "toolbar" | "location" => features.tool_bar |= value != 0,
                "status" => features.status_bar = value != 0,
                "scrollbars" => features.scrollbars = value != 0,
                "resizable" => features.resizable = value != 0,
                "noopener" => features.noopener = value != 0,
                "noreferrer" => features.noreferrer = value != 0,
                "background" => features.background = true,
                "persistent" => features.persistent = true,
                _ => {}
            }
        }

        if features.noreferrer {
            features.noopener = true;
        }
        features.is_popup = explicit_popup.unwrap_or(
            !features.tool_bar
                || !features.menu_bar
                || !features.scrollbars
                || !features.status_bar
                || !features.resizable,
        );
        features
    }

    /// Returns whether the parsed features suppress the opener relationship.
    pub fn suppresses_opener(&self) -> bool {
        self.noopener
    }

    /// Returns the normalized enabled features used by the popup runtime.
    pub fn enabled_feature_strings(&self) -> Vec<String> {
        let mut enabled = Vec::new();
        if let Some(x) = self.x {
            enabled.push(format!("left={x}"));
        }
        if let Some(y) = self.y {
            enabled.push(format!("top={y}"));
        }
        if let Some(width) = self.width {
            enabled.push(format!("width={width}"));
        }
        if let Some(height) = self.height {
            enabled.push(format!("height={height}"));
        }
        if !self.is_popup {
            enabled.push("menubar".to_owned());
            enabled.push("toolbar".to_owned());
            enabled.push("status".to_owned());
            enabled.push("scrollbars".to_owned());
        }
        if self.resizable {
            enabled.push("resizable".to_owned());
        }
        if self.noopener {
            enabled.push("noopener".to_owned());
        }
        if self.background {
            enabled.push("background".to_owned());
        }
        if self.persistent {
            enabled.push("persistent".to_owned());
        }
        enabled
    }
}

/// Tokenizes a `window.open()` features argument according to the HTML Standard.
pub fn tokenize_window_features(features: &str) -> WindowFeatures {
    let mut values = IndexMap::new();
    let mut entries = Vec::new();
    let mut input = features.chars().peekable();

    while input.peek().is_some() {
        while input.peek().is_some_and(|&c| is_feature_separator(c)) {
            input.next();
        }

        let mut name = String::new();
        while let Some(&c) = input.peek() {
            if is_feature_separator(c) {
                break;
            }
            name.push(c.to_ascii_lowercase());
            input.next();
        }
        let name = normalize_feature_name(name);

        while let Some(&c) = input.peek() {
            if c == '=' || c == ',' || !is_feature_separator(c) {
                break;
            }
            input.next();
        }

        let mut value = String::new();
        if input.peek().is_some_and(|&c| is_feature_separator(c)) {
            while let Some(&c) = input.peek() {
                if c == ',' || !is_feature_separator(c) {
                    break;
                }
                input.next();
            }
            while let Some(&c) = input.peek() {
                if is_feature_separator(c) {
                    break;
                }
                value.push(c.to_ascii_lowercase());
                input.next();
            }
        }

        if !name.is_empty() {
            entries.push((name.clone(), value.clone()));
            values.insert(name, value);
        }
    }

    WindowFeatures { values, entries }
}

/// Parses the value of a boolean window feature according to the HTML Standard.
pub fn parse_boolean_feature_value(value: &str) -> bool {
    feature_integer_value(value) != 0
}

fn feature_integer_value(value: &str) -> i32 {
    if value.is_empty() || matches!(value, "yes" | "true") {
        return 1;
    }
    parse_integer(value.chars()).unwrap_or(0)
}

fn is_feature_separator(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}' | '=' | ',')
}

fn normalize_feature_name(name: String) -> String {
    match name.as_str() {
        "screenx" => "left".to_owned(),
        "screeny" => "top".to_owned(),
        "innerwidth" => "width".to_owned(),
        "innerheight" => "height".to_owned(),
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::{WindowOpenFeatures, parse_boolean_feature_value, tokenize_window_features};

    #[test]
    fn tokenizes_names_values_and_ascii_case() {
        let features = tokenize_window_features(
            " NoOpEnEr = YES, popup=true,custom=value,feature-without-value ",
        );

        assert_eq!(features.get("noopener"), Some("yes"));
        assert_eq!(features.get("popup"), Some("true"));
        assert_eq!(features.get("custom"), Some("value"));
        assert_eq!(features.get("feature-without-value"), Some(""));
        assert_eq!(features.get("NoOpEnEr"), None);
    }

    #[test]
    fn normalizes_legacy_geometry_aliases() {
        let features =
            tokenize_window_features("screenX=1,screenY=2,innerWidth=3,innerHeight=4,left=5");

        assert_eq!(features.get("left"), Some("5"));
        assert_eq!(features.get("top"), Some("2"));
        assert_eq!(features.get("width"), Some("3"));
        assert_eq!(features.get("height"), Some("4"));
        assert_eq!(features.get("screenx"), None);
        assert_eq!(
            features.iter().collect::<Vec<_>>(),
            vec![("left", "5"), ("top", "2"), ("width", "3"), ("height", "4")]
        );
    }

    #[test]
    fn later_duplicate_values_replace_earlier_values() {
        let features = tokenize_window_features("noopener=1,noopener=0,noreferrer=0,noreferrer");

        assert!(!features.boolean("noopener"));
        assert!(features.boolean("noreferrer"));
        assert_eq!(features.iter().count(), 2);
    }

    #[test]
    fn uses_only_html_ascii_whitespace_as_separators() {
        let features =
            tokenize_window_features("left\t=1\n,top\r=2\u{000c},width =3,noopener\u{000b}=0");

        assert_eq!(features.get("left"), Some("1"));
        assert_eq!(features.get("top"), Some("2"));
        assert_eq!(features.get("width"), Some("3"));
        assert_eq!(features.get("noopener"), None);
        assert_eq!(features.get("noopener\u{000b}"), Some("0"));
    }

    #[test]
    fn parses_boolean_feature_values_with_html_integer_rules() {
        for value in ["", "yes", "true", "1", "+1", "-1", "1.5", "-1.5"] {
            assert!(parse_boolean_feature_value(value), "{value:?}");
        }
        for value in ["no", "false", "0", "+0", "-0", "0.5", "error", "+"] {
            assert!(!parse_boolean_feature_value(value), "{value:?}");
        }
    }

    #[test]
    fn empty_and_separator_only_inputs_produce_empty_maps() {
        assert!(tokenize_window_features("").is_empty());
        assert!(tokenize_window_features(" \t\n\r\u{000c}=,,").is_empty());
    }

    #[test]
    fn empty_features_match_chromium_enabled_window_features() {
        let features = WindowOpenFeatures::parse("");
        assert_eq!(
            features.enabled_feature_strings(),
            ["menubar", "toolbar", "status", "scrollbars", "resizable"]
        );
        assert!(!features.suppresses_opener());
    }

    #[test]
    fn dimensions_aliases_and_boolean_features_match_chromium_shape() {
        let features = WindowOpenFeatures::parse(
            "screenX=12, screenY=24, innerWidth=640, innerHeight=480, \
             location=yes, status=0, scrollbars=1, resizable=no, noopener",
        );
        assert_eq!(
            features.enabled_feature_strings(),
            ["left=12", "top=24", "width=640", "height=480", "noopener",]
        );
        assert!(features.suppresses_opener());
    }

    #[test]
    fn popup_override_and_loose_integers_match_chromium_shape() {
        let features = WindowOpenFeatures::parse("width=640px,popup=0,background=0,persistent=no");
        assert_eq!(
            features.enabled_feature_strings(),
            [
                "width=640",
                "menubar",
                "toolbar",
                "status",
                "scrollbars",
                "resizable",
                "background",
                "persistent",
            ]
        );
    }

    #[test]
    fn noreferrer_implies_noopener_and_last_value_wins() {
        assert!(WindowOpenFeatures::parse("noreferrer=0,noreferrer=1").suppresses_opener());
        assert!(!WindowOpenFeatures::parse("noreferrer=1,noreferrer=0").suppresses_opener());
        assert_eq!(
            WindowOpenFeatures::parse("noreferrer")
                .enabled_feature_strings()
                .last()
                .map(String::as_str),
            Some("noopener")
        );
    }
}
