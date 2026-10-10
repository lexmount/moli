/// SVG fragment magnification and the coordinates of its live currentTranslate.
/// This state is independent of the transform and viewBox attributes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SvgUserTransform {
    pub scale: f32,
    pub point: [f64; 4],
}

impl Default for SvgUserTransform {
    fn default() -> Self {
        Self {
            scale: 1.0,
            point: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

impl SvgUserTransform {
    pub(in crate::native) fn is_default(&self) -> bool {
        // Numeric equality would collapse -0 to +0. DOMPoint and SVG float
        // getters must retain the sign even when every other field is default.
        self.scale.to_bits() == 1.0_f32.to_bits()
            && self
                .point
                .into_iter()
                .zip(Self::default().point)
                .all(|(value, default)| value.to_bits() == default.to_bits())
    }

    pub(super) fn clear_translation(&mut self) {
        self.point[0] = 0.0;
        self.point[1] = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_transform_releases_rare_state_but_preserves_signed_zero() {
        let mut element = super::super::ElementRareData::default();
        assert!(element.svg_user_transform().is_default());
        let mut value = SvgUserTransform::default();
        value.point[0] = -0.0;
        element.set_svg_user_transform(value);
        assert_eq!(
            element.svg_user_transform().point[0].to_bits(),
            (-0.0_f64).to_bits()
        );
        assert!(!element.svg_user_transform().is_default());
        value.scale = 2.0;
        value.point = [10.0, 20.0, 3.0, 4.0];
        element.set_svg_user_transform(value);
        assert_eq!(element.svg_user_transform(), value);
        value.clear_translation();
        assert_eq!(value.point, [0.0, 0.0, 3.0, 4.0]);
        element.set_svg_user_transform(SvgUserTransform::default());
        assert!(element.svg_user_transform().is_default());
    }
}
