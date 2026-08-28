#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WindowOpenFeatures {
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
    pub(super) fn parse(feature_string: &str) -> Self {
        let mut features = Self::default();
        if feature_string.is_empty() {
            return features;
        }

        let buffer = feature_string.to_ascii_lowercase();
        let bytes = buffer.as_bytes();
        let mut ui_features_were_disabled = false;
        let mut explicit_popup = None;
        let mut index = 0;
        while index < bytes.len() {
            while index < bytes.len() && is_separator(bytes[index]) {
                index += 1;
            }
            let key_begin = index;
            while index < bytes.len() && !is_separator(bytes[index]) {
                index += 1;
            }
            let key_end = index;

            while index < bytes.len() && bytes[index] != b'=' {
                if bytes[index] == b',' || !is_separator(bytes[index]) {
                    break;
                }
                index += 1;
            }

            let (value_begin, value_end) = if index < bytes.len() && is_separator(bytes[index]) {
                while index < bytes.len() && is_separator(bytes[index]) {
                    if bytes[index] == b',' {
                        break;
                    }
                    index += 1;
                }
                let value_begin = index;
                while index < bytes.len() && !is_separator(bytes[index]) {
                    index += 1;
                }
                (value_begin, index)
            } else {
                (index, index)
            };

            if key_begin == key_end {
                continue;
            }

            let key = &buffer[key_begin..key_end];
            let value = feature_value(&buffer[value_begin..value_end]);
            if !ui_features_were_disabled
                && !matches!(key, "noopener" | "noreferrer" | "attributionsrc")
            {
                ui_features_were_disabled = true;
                features.menu_bar = false;
                features.status_bar = false;
                features.tool_bar = false;
                features.scrollbars = false;
            }

            match key {
                "left" | "screenx" => features.x = Some(value),
                "top" | "screeny" => features.y = Some(value),
                "width" | "innerwidth" => features.width = Some(value),
                "height" | "innerheight" => features.height = Some(value),
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

    pub(super) fn suppresses_opener(&self) -> bool {
        self.noopener
    }

    pub(super) fn suppresses_referrer(&self) -> bool {
        self.noreferrer
    }

    pub(super) fn enabled_feature_strings(&self) -> Vec<String> {
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

fn is_separator(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'=' | b',' | 0x0c)
}

fn feature_value(value: &str) -> i32 {
    if value.is_empty() || matches!(value, "yes" | "true") {
        return 1;
    }

    let bytes = value.as_bytes();
    let mut end = usize::from(matches!(bytes.first().copied(), Some(b'+' | b'-')));
    let digit_begin = end;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    if end == digit_begin {
        return 0;
    }
    value[..end].parse::<i32>().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::WindowOpenFeatures;

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
        assert!(!WindowOpenFeatures::parse("noopener").suppresses_referrer());
        assert!(WindowOpenFeatures::parse("noreferrer").suppresses_referrer());
    }
}
