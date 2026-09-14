use crate::conn::{
    EmulatedGeolocationOverride, EmulatedGeolocationOverrideState, EmulatedMediaOverrides,
};
use std::str::FromStr;

use super::params::{SetEmulatedMediaParams, SetGeolocationOverrideParams};

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString)]
#[strum(serialize_all = "kebab-case")]
enum EmulatedMediaFeatureName {
    PrefersColorScheme,
    PrefersReducedMotion,
    PrefersReducedTransparency,
    ForcedColors,
    PrefersContrast,
}

impl EmulatedMediaFeatureName {
    fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    fn apply_to_overrides(self, overrides: &mut EmulatedMediaOverrides, value: Option<String>) {
        match self {
            Self::PrefersColorScheme => overrides.color_scheme = value,
            Self::PrefersReducedMotion => overrides.reduced_motion = value,
            Self::PrefersReducedTransparency => overrides.reduced_transparency = value,
            Self::ForcedColors => overrides.forced_colors = value,
            Self::PrefersContrast => overrides.contrast = value,
        }
    }
}

pub(super) fn emulated_media_overrides_from_params(
    params: SetEmulatedMediaParams,
) -> EmulatedMediaOverrides {
    let mut overrides = EmulatedMediaOverrides {
        media: params.media.filter(|value| !value.is_empty()),
        ..Default::default()
    };
    for feature in params.features.unwrap_or_default() {
        let value = Some(feature.value).filter(|value| !value.is_empty());
        if let Some(name) = EmulatedMediaFeatureName::parse(&feature.name) {
            name.apply_to_overrides(&mut overrides, value);
        }
    }
    overrides
}

pub(super) fn geolocation_override_from_params(
    params: SetGeolocationOverrideParams,
) -> Result<EmulatedGeolocationOverrideState, ()> {
    if let Some(latitude) = params.latitude
        && (!latitude.is_finite() || !(-90.0..=90.0).contains(&latitude))
    {
        return Err(());
    }
    if let Some(longitude) = params.longitude
        && (!longitude.is_finite() || !(-180.0..=180.0).contains(&longitude))
    {
        return Err(());
    }
    if let Some(accuracy) = params.accuracy
        && (!accuracy.is_finite() || accuracy < 0.0)
    {
        return Err(());
    }
    if let Some(altitude) = params.altitude
        && !altitude.is_finite()
    {
        return Err(());
    }
    if let Some(altitude_accuracy) = params.altitude_accuracy
        && (!altitude_accuracy.is_finite() || altitude_accuracy < 0.0)
    {
        return Err(());
    }
    if let Some(heading) = params.heading
        && (!heading.is_finite() || !(0.0..360.0).contains(&heading))
    {
        return Err(());
    }
    if let Some(speed) = params.speed
        && (!speed.is_finite() || speed < 0.0)
    {
        return Err(());
    }

    let (Some(latitude), Some(longitude), Some(accuracy)) =
        (params.latitude, params.longitude, params.accuracy)
    else {
        return Ok(EmulatedGeolocationOverrideState::PositionUnavailable);
    };

    Ok(EmulatedGeolocationOverrideState::Position(
        EmulatedGeolocationOverride {
            latitude,
            longitude,
            accuracy,
            altitude: params.altitude,
            altitude_accuracy: params.altitude_accuracy,
            heading: params.heading,
            speed: params.speed,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::EmulatedMediaFeatureName;
    use crate::conn::EmulatedMediaOverrides;

    #[test]
    fn emulated_media_feature_name_parses_supported_tokens() {
        for (raw, expected) in [
            (
                "prefers-color-scheme",
                EmulatedMediaFeatureName::PrefersColorScheme,
            ),
            (
                "prefers-reduced-motion",
                EmulatedMediaFeatureName::PrefersReducedMotion,
            ),
            (
                "prefers-reduced-transparency",
                EmulatedMediaFeatureName::PrefersReducedTransparency,
            ),
            ("forced-colors", EmulatedMediaFeatureName::ForcedColors),
            (
                "prefers-contrast",
                EmulatedMediaFeatureName::PrefersContrast,
            ),
        ] {
            assert_eq!(EmulatedMediaFeatureName::parse(raw), Some(expected));
        }
        assert!(EmulatedMediaFeatureName::parse("prefers-color").is_none());
        assert!(EmulatedMediaFeatureName::parse("Prefers-Color-Scheme").is_none());
    }

    #[test]
    fn emulated_media_feature_name_applies_to_matching_override_slot() {
        let mut overrides = EmulatedMediaOverrides::default();
        EmulatedMediaFeatureName::PrefersColorScheme
            .apply_to_overrides(&mut overrides, Some("dark".to_owned()));
        EmulatedMediaFeatureName::PrefersReducedMotion
            .apply_to_overrides(&mut overrides, Some("reduce".to_owned()));
        EmulatedMediaFeatureName::PrefersReducedTransparency
            .apply_to_overrides(&mut overrides, Some("reduce".to_owned()));
        EmulatedMediaFeatureName::ForcedColors
            .apply_to_overrides(&mut overrides, Some("active".to_owned()));
        EmulatedMediaFeatureName::PrefersContrast
            .apply_to_overrides(&mut overrides, Some("more".to_owned()));

        assert_eq!(overrides.color_scheme.as_deref(), Some("dark"));
        assert_eq!(overrides.reduced_motion.as_deref(), Some("reduce"));
        assert_eq!(overrides.reduced_transparency.as_deref(), Some("reduce"));
        assert_eq!(overrides.forced_colors.as_deref(), Some("active"));
        assert_eq!(overrides.contrast.as_deref(), Some("more"));
    }
}
