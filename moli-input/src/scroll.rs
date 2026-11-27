use crate::{InputModifiers, Point};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollDeltaMode {
    Pixel,
    Line,
    Page,
}

/// One scroll step.
///
/// A scroll run retains its steps in order. Its consumer can coalesce
/// compatible steps before dispatch.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollAction {
    pub position: Point,
    pub delta_x: f64,
    pub delta_y: f64,
    pub delta_mode: ScrollDeltaMode,
    pub modifiers: InputModifiers,
}

impl ScrollAction {
    #[must_use]
    pub const fn pixels(position: Point, delta_x: f64, delta_y: f64) -> Self {
        Self {
            position,
            delta_x,
            delta_y,
            delta_mode: ScrollDeltaMode::Pixel,
            modifiers: InputModifiers::NONE,
        }
    }

    /// Accumulate an adjacent compatible step for a single input dispatch.
    /// The caller checks any document, gesture, and pointer state boundaries.
    pub fn coalesce(&mut self, next: &Self) -> bool {
        if self.position != next.position
            || self.delta_mode != next.delta_mode
            || self.modifiers != next.modifiers
            || !same_scroll_direction(self.delta_x, next.delta_x)
            || !same_scroll_direction(self.delta_y, next.delta_y)
        {
            return false;
        }

        let delta_x = self.delta_x + next.delta_x;
        let delta_y = self.delta_y + next.delta_y;
        if !delta_x.is_finite() || !delta_y.is_finite() {
            return false;
        }
        self.delta_x = delta_x;
        self.delta_y = delta_y;
        true
    }
}

fn same_scroll_direction(previous: f64, next: f64) -> bool {
    previous == 0.0 || next == 0.0 || previous.is_sign_positive() == next.is_sign_positive()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_coalescing_preserves_input_boundaries() {
        let position = Point::new(10.0, 20.0);
        let mut scroll = ScrollAction::pixels(position, 2.0, 10.0);
        let next = ScrollAction::pixels(position, 0.0, 15.0);
        assert!(scroll.coalesce(&next));
        let merged = ScrollAction::pixels(position, 2.0, 25.0);
        assert_eq!(scroll, merged);

        for incompatible in [
            ScrollAction::pixels(position, -1.0, 15.0),
            ScrollAction::pixels(position, 0.0, -5.0),
            ScrollAction::pixels(Point::new(11.0, 20.0), 0.0, 15.0),
            ScrollAction {
                delta_mode: ScrollDeltaMode::Line,
                ..next.clone()
            },
            ScrollAction {
                modifiers: InputModifiers::SHIFT,
                ..next
            },
        ] {
            assert!(!scroll.coalesce(&incompatible));
            assert_eq!(scroll, merged);
        }

        let large = ScrollAction::pixels(position, 0.0, f64::MAX);
        let mut scroll = large.clone();
        assert!(!scroll.coalesce(&large));
        assert_eq!(scroll, large);
    }
}
