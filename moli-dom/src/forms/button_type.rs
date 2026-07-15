/// The state selected by an HTML `<button>` element's `type` attribute.
///
/// Missing and invalid values select the Auto state. Whether Auto makes the
/// element a submit button depends on the element's command attributes and
/// parent node, so callers must not treat it as an alias for Submit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, strum::EnumString)]
#[strum(serialize_all = "kebab-case", ascii_case_insensitive)]
pub enum ButtonTypeState {
    #[default]
    #[strum(disabled)]
    Auto,
    Submit,
    Reset,
    Button,
}

impl ButtonTypeState {
    pub fn from_attribute_value(value: Option<&str>) -> Self {
        value
            .and_then(|value| value.parse().ok())
            .unwrap_or_default()
    }

    pub const fn reflected_keyword(self, is_submit_button: bool) -> &'static str {
        match self {
            Self::Auto if is_submit_button => "submit",
            Self::Auto | Self::Button => "button",
            Self::Submit => "submit",
            Self::Reset => "reset",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ButtonTypeState;

    #[test]
    fn keywords_are_ascii_case_insensitive_without_whitespace_trimming() {
        assert_eq!(
            ButtonTypeState::from_attribute_value(Some("SuBmIt")),
            ButtonTypeState::Submit
        );
        assert_eq!(
            ButtonTypeState::from_attribute_value(Some("RESET")),
            ButtonTypeState::Reset
        );
        assert_eq!(
            ButtonTypeState::from_attribute_value(Some("button")),
            ButtonTypeState::Button
        );
        for value in [None, Some(""), Some("auto"), Some(" submit ")] {
            assert_eq!(
                ButtonTypeState::from_attribute_value(value),
                ButtonTypeState::Auto
            );
        }
    }

    #[test]
    fn auto_reflection_uses_the_derived_submit_button_state() {
        assert_eq!(ButtonTypeState::Auto.reflected_keyword(true), "submit");
        assert_eq!(ButtonTypeState::Auto.reflected_keyword(false), "button");
        assert_eq!(ButtonTypeState::Submit.reflected_keyword(false), "submit");
    }
}
