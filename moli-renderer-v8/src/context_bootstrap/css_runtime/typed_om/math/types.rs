use style::values::specified::length::LengthUnit;

pub(super) const BASE_NAMES: [&str; 7] = [
    "length",
    "angle",
    "time",
    "frequency",
    "resolution",
    "flex",
    "percent",
];
const PERCENT: usize = 6;

// Stylo's NumericType currently keeps both the exponent map and percent hint
// private and has no inversion API. The DOM needs all three for type() and
// CSSMathInvert. Keep the complete Typed OM type here, without converting JS
// doubles to Stylo's f32 quantities or inspecting its private representation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct NumericType {
    pub powers: [i32; 7],
    pub hint: Option<usize>,
}

impl NumericType {
    pub fn from_unit(unit: &str) -> Option<Self> {
        let base = match unit {
            "number" => return Some(Self::default()),
            "percent" => PERCENT,
            "deg" | "grad" | "rad" | "turn" => 1,
            "s" | "ms" => 2,
            "hz" | "khz" => 3,
            "dpi" | "dpcm" | "dppx" | "x" => 4,
            "fr" => 5,
            _ if LengthUnit::from_str(unit).is_ok() => 0,
            _ => return None,
        };
        let mut result = Self::default();
        result.powers[base] = 1;
        Some(result)
    }

    fn apply_hint(mut self, hint: usize) -> Option<Self> {
        self.hint = Some(hint);
        if hint != PERCENT {
            self.powers[hint] = self.powers[hint].checked_add(self.powers[PERCENT])?;
            self.powers[PERCENT] = 0;
        }
        Some(self)
    }

    fn common_hint(mut self, mut other: Self) -> Option<(Self, Self)> {
        match (self.hint, other.hint) {
            (Some(a), Some(b)) if a != b => return None,
            (Some(hint), None) => other = other.apply_hint(hint)?,
            (None, Some(hint)) => self = self.apply_hint(hint)?,
            _ => {}
        }
        Some((self, other))
    }

    // https://drafts.css-houdini.org/css-typed-om/#cssnumericvalue-add-two-types
    pub fn add(self, other: Self) -> Option<Self> {
        let (a, b) = self.common_hint(other)?;
        if a.powers == b.powers {
            return Some(a);
        }
        if (a.powers[PERCENT] != 0 || b.powers[PERCENT] != 0)
            && (a.powers[..PERCENT].iter().any(|&p| p != 0)
                || b.powers[..PERCENT].iter().any(|&p| p != 0))
        {
            for hint in 0..PERCENT {
                let x = a.apply_hint(hint)?;
                let y = b.apply_hint(hint)?;
                if x.powers == y.powers {
                    return Some(x);
                }
            }
        }
        None
    }

    pub fn multiply(self, other: Self) -> Option<Self> {
        let (mut a, b) = self.common_hint(other)?;
        for (power, other) in a.powers.iter_mut().zip(b.powers) {
            *power = power.checked_add(other)?;
        }
        Some(a)
    }

    pub fn invert(mut self) -> Option<Self> {
        for power in &mut self.powers {
            *power = power.checked_neg()?;
        }
        Some(self)
    }
}
