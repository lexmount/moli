//! Canonical states for the HTML `<input type>` enumerated attribute.

/// The state selected by an HTML `<input>` element's `type` attribute.
///
/// Missing and invalid attribute values select [`InputType::Text`]. The raw
/// content attribute remains available at the DOM boundary when callers need
/// to distinguish an invalid keyword from the resulting state.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    Hash,
    PartialEq,
    strum::AsRefStr,
    strum::Display,
    strum::EnumIter,
    strum::EnumString,
)]
#[strum(serialize_all = "kebab-case", ascii_case_insensitive)]
pub enum InputType {
    Button,
    Checkbox,
    Color,
    Date,
    DatetimeLocal,
    Email,
    File,
    Hidden,
    Image,
    Month,
    Number,
    Password,
    Radio,
    Range,
    Reset,
    Search,
    Submit,
    Tel,
    #[default]
    Text,
    Time,
    Url,
    Week,
}

impl InputType {
    /// Applies the enumerated-attribute keyword matching and invalid-value
    /// default used by `HTMLInputElement.type`.
    pub fn from_attribute_value(value: Option<&str>) -> Self {
        value
            .and_then(|value| value.parse().ok())
            .unwrap_or_default()
    }

    pub const fn is_checkable(self) -> bool {
        matches!(self, Self::Checkbox | Self::Radio)
    }

    pub const fn is_submit_button(self) -> bool {
        matches!(self, Self::Submit | Self::Image)
    }

    pub const fn supports_value_as_number(self) -> bool {
        matches!(
            self,
            Self::Number
                | Self::Range
                | Self::Date
                | Self::Time
                | Self::DatetimeLocal
                | Self::Month
                | Self::Week
        )
    }

    pub const fn supports_value_as_date(self) -> bool {
        matches!(self, Self::Date | Self::Month | Self::Week | Self::Time)
    }

    pub const fn supports_pattern(self) -> bool {
        matches!(
            self,
            Self::Text | Self::Search | Self::Tel | Self::Url | Self::Email | Self::Password
        )
    }

    pub const fn supports_placeholder(self) -> bool {
        matches!(
            self,
            Self::Text | Self::Search | Self::Url | Self::Tel | Self::Email | Self::Password
        )
    }

    pub const fn supports_required(self) -> bool {
        !matches!(
            self,
            Self::Hidden | Self::Button | Self::Submit | Self::Reset | Self::Image
        )
    }

    pub const fn supports_text_length_validation(self) -> bool {
        matches!(
            self,
            Self::Text | Self::Search | Self::Url | Self::Tel | Self::Email | Self::Password
        )
    }

    pub const fn supports_readonly(self) -> bool {
        matches!(
            self,
            Self::Text
                | Self::Search
                | Self::Url
                | Self::Tel
                | Self::Email
                | Self::Password
                | Self::Date
                | Self::Month
                | Self::Week
                | Self::Time
                | Self::DatetimeLocal
                | Self::Number
        )
    }

    pub const fn supports_variable_length_selection(self) -> bool {
        matches!(
            self,
            Self::Text | Self::Search | Self::Tel | Self::Url | Self::Password
        )
    }

    pub const fn supports_dirname(self) -> bool {
        matches!(
            self,
            Self::Hidden
                | Self::Text
                | Self::Search
                | Self::Tel
                | Self::Url
                | Self::Email
                | Self::Password
        )
    }

    pub const fn uses_value_for_auto_direction(self) -> bool {
        matches!(
            self,
            Self::Hidden
                | Self::Text
                | Self::Search
                | Self::Tel
                | Self::Url
                | Self::Email
                | Self::Password
                | Self::Submit
                | Self::Reset
                | Self::Button
        )
    }
}

impl From<Option<&str>> for InputType {
    fn from(value: Option<&str>) -> Self {
        Self::from_attribute_value(value)
    }
}

#[cfg(test)]
mod tests {
    use super::InputType;
    use strum::IntoEnumIterator;

    #[test]
    fn canonical_keywords_round_trip() {
        for input_type in InputType::iter() {
            let keyword: &str = input_type.as_ref();
            assert_eq!(keyword.parse::<InputType>(), Ok(input_type));
            assert_eq!(input_type.to_string(), keyword);
        }
    }

    #[test]
    fn matching_is_ascii_case_insensitive() {
        assert_eq!("EMAIL".parse::<InputType>(), Ok(InputType::Email));
        assert_eq!(
            "DaTeTiMe-LoCaL".parse::<InputType>(),
            Ok(InputType::DatetimeLocal)
        );
    }

    #[test]
    fn missing_invalid_and_whitespace_padded_values_select_text() {
        assert_eq!(InputType::from_attribute_value(None), InputType::Text);
        for value in [
            "unknown",
            "DatetimeLocal",
            "datetime_local",
            " datetime-local ",
        ] {
            assert!(value.parse::<InputType>().is_err());
            assert_eq!(
                InputType::from_attribute_value(Some(value)),
                InputType::Text
            );
        }
    }

    #[test]
    fn capability_sets_are_defined_on_canonical_states() {
        assert!(InputType::Week.supports_value_as_number());
        assert!(!InputType::Email.supports_value_as_number());
        assert!(InputType::Password.supports_pattern());
        assert!(InputType::Email.supports_placeholder());
        assert!(!InputType::Submit.supports_required());
        assert!(InputType::Email.supports_text_length_validation());
        assert!(InputType::Number.supports_readonly());
        assert!(InputType::Url.supports_variable_length_selection());
        assert!(InputType::Hidden.supports_dirname());
        assert!(InputType::Submit.uses_value_for_auto_direction());
    }
}
