//! Canvas compositing in premultiplied coordinates, independent of storage.
//!
//! Porter-Duff and blend formulas follow CSS Compositing and Blending Level 2.
//! Plus-darker uses the premultiplied Quartz definition; the current CSS draft
//! omits the outer inversion in its displayed formula.
//! See https://developer.apple.com/documentation/coregraphics/cgblendmode/plusdarker.
//! Clipping is applied to the result of this operation, rather than to source
//! alpha: a transparent source can still replace or erase destination pixels.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "kebab-case")]
pub enum CompositeOperation {
    Clear,
    #[default]
    SourceOver,
    SourceIn,
    SourceOut,
    SourceAtop,
    DestinationOver,
    DestinationIn,
    DestinationOut,
    DestinationAtop,
    Lighter,
    Copy,
    Xor,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    PlusDarker,
    PlusLighter,
}

impl CompositeOperation {
    pub fn parse(value: &str) -> Option<Self> {
        value.parse().ok()
    }

    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// These operators also affect the transparent part of the infinite
    /// source image, including pixels outside a drawn image's destination.
    pub const fn affects_transparent_source(self) -> bool {
        matches!(
            self,
            Self::Clear
                | Self::SourceIn
                | Self::SourceOut
                | Self::DestinationIn
                | Self::DestinationAtop
                | Self::Copy
                | Self::PlusDarker
        )
    }

    pub(crate) fn apply(self, destination: [f32; 4], source: [f32; 4]) -> [f32; 4] {
        let sa = source[3];
        let da = destination[3];
        let (fa, fb) = match self {
            Self::Clear => (0.0, 0.0),
            Self::SourceOver => (1.0, 1.0 - sa),
            Self::SourceIn => (da, 0.0),
            Self::SourceOut => (1.0 - da, 0.0),
            Self::SourceAtop => (da, 1.0 - sa),
            Self::DestinationOver => (1.0 - da, 1.0),
            Self::DestinationIn => (0.0, sa),
            Self::DestinationOut => (0.0, 1.0 - sa),
            Self::DestinationAtop => (1.0 - da, sa),
            Self::Copy => (1.0, 0.0),
            Self::Xor => (1.0 - da, 1.0 - sa),
            Self::Lighter => {
                let mut result = std::array::from_fn(|i| source[i] + destination[i]);
                result[3] = result[3].min(1.0);
                return result;
            }
            Self::PlusLighter => {
                return std::array::from_fn(|i| (source[i] + destination[i]).min(1.0));
            }
            Self::PlusDarker => {
                return std::array::from_fn(|i| (source[i] + destination[i] - 1.0).max(0.0));
            }
            _ => return self.blend(destination, source),
        };
        // An excluded contribution is zero even if raw float16 storage holds
        // infinities or NaNs. In particular copy/clear must not read the old
        // destination through an infinity-times-zero expression.
        match (fa == 0.0, fb == 0.0) {
            (true, true) => [0.0; 4],
            (true, false) => destination.map(|v| v * fb),
            (false, true) => source.map(|v| v * fa),
            (false, false) => std::array::from_fn(|i| source[i] * fa + destination[i] * fb),
        }
    }

    fn blend(self, destination: [f32; 4], source: [f32; 4]) -> [f32; 4] {
        let sa = source[3];
        let da = destination[3];
        let backdrop = std::array::from_fn(|i| if da == 0.0 { 0.0 } else { destination[i] / da });
        let foreground = std::array::from_fn(|i| if sa == 0.0 { 0.0 } else { source[i] / sa });
        let mixed = match self {
            Self::Hue => set_lum(set_sat(foreground, sat(backdrop)), lum(backdrop)),
            Self::Saturation => set_lum(set_sat(backdrop, sat(foreground)), lum(backdrop)),
            Self::Color => set_lum(foreground, lum(backdrop)),
            Self::Luminosity => set_lum(backdrop, lum(foreground)),
            _ => std::array::from_fn(|i| self.channel(backdrop[i], foreground[i])),
        };
        let mut result = [0.0; 4];
        for i in 0..3 {
            result[i] = source[i] * (1.0 - da) + destination[i] * (1.0 - sa) + mixed[i] * sa * da;
        }
        result[3] = sa + da * (1.0 - sa);
        result
    }

    fn channel(self, b: f32, s: f32) -> f32 {
        match self {
            Self::Multiply => b * s,
            Self::Screen => b + s - b * s,
            Self::Overlay => hard_light(s, b),
            Self::Darken => b.min(s),
            Self::Lighten => b.max(s),
            Self::ColorDodge if b == 0.0 => 0.0,
            Self::ColorDodge if s >= 1.0 => 1.0,
            Self::ColorDodge => (b / (1.0 - s)).min(1.0),
            Self::ColorBurn if b >= 1.0 => 1.0,
            Self::ColorBurn if s <= 0.0 => 0.0,
            Self::ColorBurn => 1.0 - ((1.0 - b) / s).min(1.0),
            Self::HardLight => hard_light(b, s),
            Self::SoftLight if s <= 0.5 => b - (1.0 - 2.0 * s) * b * (1.0 - b),
            Self::SoftLight => {
                let d = if b <= 0.25 {
                    ((16.0 * b - 12.0) * b + 4.0) * b
                } else {
                    b.sqrt()
                };
                b + (2.0 * s - 1.0) * (d - b)
            }
            Self::Difference => (b - s).abs(),
            Self::Exclusion => b + s - 2.0 * b * s,
            _ => unreachable!("only separable blend modes have a channel formula"),
        }
    }
}

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        2.0 * b * s
    } else {
        1.0 - 2.0 * (1.0 - b) * (1.0 - s)
    }
}

fn lum(c: [f32; 3]) -> f32 {
    c[0] * 0.3 + c[1] * 0.59 + c[2] * 0.11
}

fn sat(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn set_lum(mut c: [f32; 3], target: f32) -> [f32; 3] {
    let delta = target - lum(c);
    for value in &mut c {
        *value += delta;
    }
    let l = lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    if n < 0.0 {
        for value in &mut c {
            *value = l + (*value - l) * l / (l - n);
        }
    }
    if x > 1.0 {
        for value in &mut c {
            *value = l + (*value - l) * (1.0 - l) / (x - l);
        }
    }
    c
}

fn set_sat(mut c: [f32; 3], target: f32) -> [f32; 3] {
    let mut order = [0, 1, 2];
    order.sort_by(|&a, &b| c[a].total_cmp(&c[b]));
    let [min, mid, max] = order;
    if c[max] > c[min] {
        c[mid] = (c[mid] - c[min]) * target / (c[max] - c[min]);
        c[max] = target;
    } else {
        c[mid] = 0.0;
        c[max] = 0.0;
    }
    c[min] = 0.0;
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: [f32; 4], expected: [f32; 4]) {
        for (a, b) in actual.into_iter().zip(expected) {
            assert!((a - b).abs() < 0.00001, "{actual:?} != {expected:?}");
        }
    }

    #[test]
    fn porter_duff_compositing_uses_premultiplied_alpha() {
        use CompositeOperation::*;
        let source = [0.125, 0.0, 0.25, 0.25];
        let destination = [0.0, 0.375, 0.125, 0.75];
        for (op, expected) in [
            (Clear, [0.0; 4]),
            (SourceOver, [0.125, 0.28125, 0.34375, 0.8125]),
            (SourceIn, [0.09375, 0.0, 0.1875, 0.1875]),
            (SourceOut, [0.03125, 0.0, 0.0625, 0.0625]),
            (SourceAtop, [0.09375, 0.28125, 0.28125, 0.75]),
            (DestinationOver, [0.03125, 0.375, 0.1875, 0.8125]),
            (DestinationIn, [0.0, 0.09375, 0.03125, 0.1875]),
            (DestinationOut, [0.0, 0.28125, 0.09375, 0.5625]),
            (DestinationAtop, [0.03125, 0.09375, 0.09375, 0.25]),
            (Lighter, [0.125, 0.375, 0.375, 1.0]),
            (Copy, source),
            (Xor, [0.03125, 0.28125, 0.15625, 0.625]),
        ] {
            close(op.apply(destination, source), expected);
        }
    }

    #[test]
    fn separable_blends_use_unpremultiplied_colors_and_preserve_alpha() {
        use CompositeOperation::*;
        for (op, expected) in [
            (Multiply, [0.24, 0.14, 0.24, 1.0]),
            (Screen, [0.86, 0.76, 0.76, 1.0]),
            (Overlay, [0.48, 0.52, 0.48, 1.0]),
            (Darken, [0.3, 0.2, 0.4, 1.0]),
            (Lighten, [0.8, 0.7, 0.6, 1.0]),
            (ColorDodge, [1.0, 0.875, 1.0, 1.0]),
            (ColorBurn, [0.125, 0.0, 0.0, 1.0]),
            (HardLight, [0.72, 0.28, 0.52, 1.0]),
            (SoftLight, [0.44863355, 0.574, 0.4464911, 1.0]),
            (Difference, [0.5, 0.5, 0.2, 1.0]),
            (Exclusion, [0.62, 0.62, 0.52, 1.0]),
        ] {
            close(
                op.apply([0.3, 0.7, 0.4, 1.0], [0.8, 0.2, 0.6, 1.0]),
                expected,
            );
            close(
                op.apply([0.0; 4], [0.2, 0.05, 0.15, 0.25]),
                [0.2, 0.05, 0.15, 0.25],
            );
        }
    }

    #[test]
    fn nonseparable_blends_preserve_specified_luminance_and_saturation() {
        use CompositeOperation::*;
        let b = [0.3, 0.7, 0.4];
        let s = [0.8, 0.2, 0.6];
        for (op, luminance) in [
            (Hue, lum(b)),
            (Saturation, lum(b)),
            (Color, lum(b)),
            (Luminosity, lum(s)),
        ] {
            let c = op.apply([b[0], b[1], b[2], 1.0], [s[0], s[1], s[2], 1.0]);
            assert!((lum([c[0], c[1], c[2]]) - luminance).abs() < 0.00001);
            assert!(c.iter().all(|v| (0.0..=1.0).contains(v)));
        }
    }

    #[test]
    fn additive_compositing_retains_hdr_colors_and_bounds_alpha() {
        close(
            CompositeOperation::Lighter.apply([2.0, -1.0, 0.5, 0.75], [4.0, 0.25, -0.5, 0.5]),
            [6.0, -0.75, 0.0, 1.0],
        );
        close(
            CompositeOperation::PlusLighter.apply([2.0, 0.5, 0.0, 0.75], [4.0, 0.25, 0.5, 0.5]),
            [1.0, 0.75, 0.5, 1.0],
        );
        close(
            CompositeOperation::PlusDarker.apply([0.4, 0.5, 0.9, 1.0], [0.8, 0.7, 0.6, 1.0]),
            [0.2, 0.2, 0.5, 1.0],
        );
    }

    #[test]
    fn zero_porter_duff_factors_ignore_nonfinite_storage_channels() {
        let old = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0];
        let source = [0.25, 0.5, 0.75, 1.0];
        assert_eq!(CompositeOperation::Copy.apply(old, source), source);
        assert_eq!(CompositeOperation::SourceOver.apply(old, source), source);
        assert_eq!(CompositeOperation::Clear.apply(old, source), [0.0; 4]);
        assert_eq!(
            CompositeOperation::DestinationIn.apply(old, [0.0; 4]),
            [0.0; 4]
        );
    }
}
