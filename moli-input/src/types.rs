use std::ops::{BitOr, BitOrAssign};

/// A viewport-space input coordinate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Modifier keys active for an input action.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputModifiers(u8);

impl InputModifiers {
    pub const NONE: Self = Self(0);
    pub const ALT: Self = Self(1 << 0);
    pub const CONTROL: Self = Self(1 << 1);
    pub const META: Self = Self(1 << 2);
    pub const SHIFT: Self = Self(1 << 3);

    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for InputModifiers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for InputModifiers {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

/// One complete logical click.
///
/// This must represent the completed press/release gesture. Raw pointer-down
/// and pointer-up events must not be admitted independently, since click
/// compaction intentionally keeps only the latest click in a scope.
#[derive(Clone, Debug, PartialEq)]
pub struct ClickAction {
    pub position: Point,
    pub button: MouseButton,
    pub click_count: u32,
    pub modifiers: InputModifiers,
}

impl ClickAction {
    #[must_use]
    pub const fn new(position: Point, button: MouseButton, click_count: u32) -> Self {
        Self {
            position,
            button,
            click_count,
            modifiers: InputModifiers::NONE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InputModifiers;

    #[test]
    fn modifiers_are_a_composable_bit_set() {
        let modifiers = InputModifiers::CONTROL | InputModifiers::SHIFT;

        assert!(modifiers.contains(InputModifiers::CONTROL));
        assert!(modifiers.contains(InputModifiers::SHIFT));
        assert!(!modifiers.contains(InputModifiers::ALT));
        assert_eq!(modifiers.bits(), 0b1010);
    }
}
