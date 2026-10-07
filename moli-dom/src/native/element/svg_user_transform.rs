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
