//! Canonical classic-scrollbar geometry shared by layout, paint, and input.
//!
//! Chromium's `ScrollbarTheme` computes the control parts once and uses the
//! same result for painting and hit testing. This follows Blitz's useful
//! shared-geometry model while freezing the result outside its DOM-bound
//! overlay-scrollbar implementation.

use crate::{LayoutPoint, LayoutRect, LayoutTransform2D, PaintColor};

pub(crate) const DEFAULT_SCROLLBAR_THICKNESS: f32 = 15.0;
pub(crate) const THIN_SCROLLBAR_THICKNESS: f32 = 10.0;
const FLUENT_SCROLLBAR_BUTTON_LENGTH: f32 = 18.0;
const FLUENT_SCROLLBAR_THUMB_THICKNESS: f32 = 9.0;
const FLUENT_SCROLLBAR_MINIMUM_THUMB_LENGTH: f32 = 17.0;
const LINE_SCROLL_STEP: f32 = 40.0;

/// Computed `scrollbar-width` projected at the layout boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutScrollbarWidth {
    #[default]
    Auto,
    Thin,
    None,
}

impl LayoutScrollbarWidth {
    pub(crate) const fn thickness(self) -> f32 {
        match self {
            Self::Auto => DEFAULT_SCROLLBAR_THICKNESS,
            Self::Thin => THIN_SCROLLBAR_THICKNESS,
            Self::None => 0.0,
        }
    }
}

/// Computed `scrollbar-gutter` projected at the layout boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutScrollbarGutter {
    #[default]
    Auto,
    Stable,
    StableBothEdges,
}

/// Colors used for one author-colored classic scrollbar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutScrollbarColors {
    pub thumb: PaintColor,
    pub track: PaintColor,
}

impl LayoutScrollbarColors {
    pub const fn new(thumb: PaintColor, track: PaintColor) -> Self {
        Self { thumb, track }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutScrollbarAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutScrollbarPart {
    BackButton,
    BackTrack,
    Thumb,
    ForwardTrack,
    ForwardButton,
}

/// Input hit resolved from the same frozen geometry paint consumes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutScrollbarHit<N> {
    pub source: N,
    pub scrollbar: LayoutScrollbarGeometry,
    pub part: LayoutScrollbarPart,
    pub local_point: LayoutPoint,
    pub viewport_to_local: LayoutTransform2D,
}

/// One scrollbar in its owning box's local coordinate space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutScrollbarGeometry {
    pub axis: LayoutScrollbarAxis,
    pub frame: LayoutRect,
    pub track: LayoutRect,
    /// Full-cross-axis thumb geometry used by Chromium's track hit testing.
    pub thumb: LayoutRect,
    /// Fluent's centered, pill-shaped visual nested inside `thumb`.
    pub painted_thumb: LayoutRect,
    pub back_button: LayoutRect,
    pub forward_button: LayoutRect,
    pub minimum_offset: f32,
    pub maximum_offset: f32,
    pub current_offset: f32,
}

impl LayoutScrollbarGeometry {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        axis: LayoutScrollbarAxis,
        frame: LayoutRect,
        minimum_offset: f32,
        maximum_offset: f32,
        current_offset: f32,
        visible_length: f32,
        total_length: f32,
    ) -> Self {
        let frame_length = axis_length(axis, frame).max(0.0);
        // Fluent keeps both buttons present on a short scrollbar by splitting
        // the available length evenly between them.  Using the full thickness
        // for each would make the controls overlap and leave the whole frame
        // owned by the back button during hit testing.
        let cross_length = axis_cross_length(axis, frame).max(0.0);
        let proportion = cross_length / DEFAULT_SCROLLBAR_THICKNESS;
        let desired_button_length = (FLUENT_SCROLLBAR_BUTTON_LENGTH * proportion).round();
        let button_length = desired_button_length.min(frame_length / 2.0).floor();
        let (back_button, forward_button, track) = axis_parts(axis, frame, button_length);
        let track_length = axis_length(axis, track).max(0.0);
        let proportional = if total_length > 0.0 {
            track_length * (visible_length / total_length).clamp(0.0, 1.0)
        } else {
            track_length
        };
        // NativeThemeFluent reports a 17px minimum auto thumb. The thin
        // variant scales by 2/3 and Chromium's integer return truncates it.
        let minimum_thumb_length = (FLUENT_SCROLLBAR_MINIMUM_THUMB_LENGTH * proportion).floor();
        let thumb_length = proportional
            .round()
            .max(minimum_thumb_length.min(track_length))
            .min(track_length);
        let range = maximum_offset - minimum_offset;
        let progress = if range > f32::EPSILON {
            ((current_offset - minimum_offset) / range).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let raw_thumb_position = (track_length - thumb_length) * progress;
        // `ScrollbarTheme::ThumbPosition` keeps a non-zero subpixel move
        // visible as one pixel, then stores all later positions as integers.
        let thumb_position = if raw_thumb_position > 0.0 && raw_thumb_position < 1.0 {
            1.0
        } else {
            raw_thumb_position.trunc()
        };
        let thumb_start = axis_start(axis, track) + thumb_position;
        let thumb = axis_rect(axis, track, thumb_start, thumb_length);
        let mut painted_thumb_thickness = (FLUENT_SCROLLBAR_THUMB_THICKNESS * proportion).round();
        let thickness_difference = cross_length.round() - painted_thumb_thickness;
        if thickness_difference.rem_euclid(2.0) >= 1.0 {
            painted_thumb_thickness = (painted_thumb_thickness - 1.0).max(0.0);
        }
        let painted_thumb = inset_cross_axis(
            axis,
            thumb,
            ((cross_length - painted_thumb_thickness) / 2.0).max(0.0),
        );
        Self {
            axis,
            frame,
            track,
            thumb,
            painted_thumb,
            back_button,
            forward_button,
            minimum_offset,
            maximum_offset,
            current_offset,
        }
    }

    pub fn part_at(self, point: LayoutPoint) -> Option<LayoutScrollbarPart> {
        if !self.frame.contains(point) {
            return None;
        }
        if self.back_button.contains(point) {
            return Some(LayoutScrollbarPart::BackButton);
        }
        if self.forward_button.contains(point) {
            return Some(LayoutScrollbarPart::ForwardButton);
        }
        if self.thumb.contains(point) {
            return Some(LayoutScrollbarPart::Thumb);
        }
        let coordinate = axis_coordinate(self.axis, point);
        if coordinate < axis_start(self.axis, self.thumb) {
            Some(LayoutScrollbarPart::BackTrack)
        } else {
            Some(LayoutScrollbarPart::ForwardTrack)
        }
    }

    /// CSS scroll units advanced by one local CSS pixel of thumb motion.
    pub fn drag_ratio(self) -> f32 {
        let thumb_travel = axis_length(self.axis, self.track) - axis_length(self.axis, self.thumb);
        if thumb_travel <= f32::EPSILON {
            0.0
        } else {
            (self.maximum_offset - self.minimum_offset) / thumb_travel
        }
    }

    pub fn line_target(self, forward: bool) -> f32 {
        let delta = if forward {
            LINE_SCROLL_STEP
        } else {
            -LINE_SCROLL_STEP
        };
        (self.current_offset + delta).clamp(self.minimum_offset, self.maximum_offset)
    }

    /// Chromium's Linux classic scrollbar advances seven eighths of the
    /// visible scrollport for a track click.
    pub fn page_target(self, forward: bool) -> f32 {
        let step = (axis_length(self.axis, self.frame) * 0.875).max(1.0);
        let delta = if forward { step } else { -step };
        (self.current_offset + delta).clamp(self.minimum_offset, self.maximum_offset)
    }

    pub fn local_axis_coordinate(self, point: LayoutPoint) -> f32 {
        axis_coordinate(self.axis, point)
    }
}

fn inset_cross_axis(axis: LayoutScrollbarAxis, rect: LayoutRect, inset: f32) -> LayoutRect {
    match axis {
        LayoutScrollbarAxis::Horizontal => LayoutRect::new(
            rect.x,
            rect.y + inset,
            rect.width,
            (rect.height - inset * 2.0).max(0.0),
        ),
        LayoutScrollbarAxis::Vertical => LayoutRect::new(
            rect.x + inset,
            rect.y,
            (rect.width - inset * 2.0).max(0.0),
            rect.height,
        ),
    }
}

fn axis_parts(
    axis: LayoutScrollbarAxis,
    frame: LayoutRect,
    button_length: f32,
) -> (LayoutRect, LayoutRect, LayoutRect) {
    let start = axis_start(axis, frame);
    let length = axis_length(axis, frame);
    let back = axis_rect(axis, frame, start, button_length);
    let forward = axis_rect(
        axis,
        frame,
        start + (length - button_length).max(0.0),
        button_length,
    );
    let track = axis_rect(
        axis,
        frame,
        start + button_length,
        (length - button_length * 2.0).max(0.0),
    );
    (back, forward, track)
}

fn axis_rect(
    axis: LayoutScrollbarAxis,
    cross_axis: LayoutRect,
    start: f32,
    length: f32,
) -> LayoutRect {
    match axis {
        LayoutScrollbarAxis::Horizontal => LayoutRect::new(
            start,
            cross_axis.y,
            length.max(0.0),
            cross_axis.height.max(0.0),
        ),
        LayoutScrollbarAxis::Vertical => LayoutRect::new(
            cross_axis.x,
            start,
            cross_axis.width.max(0.0),
            length.max(0.0),
        ),
    }
}

pub(crate) fn axis_coordinate(axis: LayoutScrollbarAxis, point: LayoutPoint) -> f32 {
    match axis {
        LayoutScrollbarAxis::Horizontal => point.x,
        LayoutScrollbarAxis::Vertical => point.y,
    }
}

pub(crate) fn axis_start(axis: LayoutScrollbarAxis, rect: LayoutRect) -> f32 {
    match axis {
        LayoutScrollbarAxis::Horizontal => rect.x,
        LayoutScrollbarAxis::Vertical => rect.y,
    }
}

pub(crate) fn axis_length(axis: LayoutScrollbarAxis, rect: LayoutRect) -> f32 {
    match axis {
        LayoutScrollbarAxis::Horizontal => rect.width,
        LayoutScrollbarAxis::Vertical => rect.height,
    }
}

fn axis_cross_length(axis: LayoutScrollbarAxis, rect: LayoutRect) -> f32 {
    match axis {
        LayoutScrollbarAxis::Horizontal => rect.height,
        LayoutScrollbarAxis::Vertical => rect.width,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_geometry_and_parts_share_one_chromium_style_model() {
        let bar = LayoutScrollbarGeometry::new(
            LayoutScrollbarAxis::Vertical,
            LayoutRect::new(185.0, 0.0, 15.0, 100.0),
            0.0,
            300.0,
            150.0,
            100.0,
            400.0,
        );
        assert_eq!(bar.back_button, LayoutRect::new(185.0, 0.0, 15.0, 18.0));
        assert_eq!(bar.forward_button, LayoutRect::new(185.0, 82.0, 15.0, 18.0));
        assert_eq!(bar.track, LayoutRect::new(185.0, 18.0, 15.0, 64.0));
        assert_eq!(bar.thumb, LayoutRect::new(185.0, 41.0, 15.0, 17.0));
        assert_eq!(bar.painted_thumb, LayoutRect::new(188.0, 41.0, 9.0, 17.0));
        assert_eq!(
            bar.part_at(LayoutPoint::new(190.0, 5.0)),
            Some(LayoutScrollbarPart::BackButton)
        );
        assert_eq!(
            bar.part_at(LayoutPoint::new(190.0, 25.0)),
            Some(LayoutScrollbarPart::BackTrack)
        );
        assert_eq!(
            bar.part_at(LayoutPoint::new(190.0, 45.0)),
            Some(LayoutScrollbarPart::Thumb)
        );
        assert_eq!(
            bar.part_at(LayoutPoint::new(190.0, 75.0)),
            Some(LayoutScrollbarPart::ForwardTrack)
        );
        assert_eq!(
            bar.part_at(LayoutPoint::new(190.0, 90.0)),
            Some(LayoutScrollbarPart::ForwardButton)
        );
        assert!((bar.drag_ratio() - 300.0 / 47.0).abs() < f32::EPSILON);
    }

    #[test]
    fn rtl_range_places_zero_offset_at_the_forward_end() {
        let bar = LayoutScrollbarGeometry::new(
            LayoutScrollbarAxis::Horizontal,
            LayoutRect::new(0.0, 85.0, 200.0, 15.0),
            -200.0,
            0.0,
            0.0,
            200.0,
            400.0,
        );
        assert_eq!(bar.thumb.right(), bar.track.right());
        assert_eq!(bar.line_target(false), -40.0);
        assert_eq!(bar.line_target(true), 0.0);
    }

    #[test]
    fn short_scrollbar_splits_its_frame_between_non_overlapping_buttons() {
        let bar = LayoutScrollbarGeometry::new(
            LayoutScrollbarAxis::Vertical,
            LayoutRect::new(0.0, 0.0, 15.0, 20.0),
            0.0,
            100.0,
            0.0,
            20.0,
            120.0,
        );
        assert_eq!(bar.back_button, LayoutRect::new(0.0, 0.0, 15.0, 10.0));
        assert_eq!(bar.forward_button, LayoutRect::new(0.0, 10.0, 15.0, 10.0));
        assert_eq!(bar.track, LayoutRect::new(0.0, 10.0, 15.0, 0.0));
        assert_eq!(
            bar.part_at(LayoutPoint::new(7.5, 5.0)),
            Some(LayoutScrollbarPart::BackButton)
        );
        assert_eq!(
            bar.part_at(LayoutPoint::new(7.5, 15.0)),
            Some(LayoutScrollbarPart::ForwardButton)
        );
    }

    #[test]
    fn thin_fluent_parts_scale_before_integer_geometry_is_frozen() {
        let bar = LayoutScrollbarGeometry::new(
            LayoutScrollbarAxis::Vertical,
            LayoutRect::new(190.0, 0.0, 10.0, 100.0),
            0.0,
            900.0,
            0.0,
            100.0,
            1_000.0,
        );
        assert_eq!(bar.back_button, LayoutRect::new(190.0, 0.0, 10.0, 12.0));
        assert_eq!(bar.forward_button, LayoutRect::new(190.0, 88.0, 10.0, 12.0));
        assert_eq!(bar.track, LayoutRect::new(190.0, 12.0, 10.0, 76.0));
        assert_eq!(bar.thumb, LayoutRect::new(190.0, 12.0, 10.0, 11.0));
        assert_eq!(bar.painted_thumb, LayoutRect::new(192.0, 12.0, 6.0, 11.0));

        let first_subpixel_move = LayoutScrollbarGeometry::new(
            LayoutScrollbarAxis::Vertical,
            bar.frame,
            0.0,
            900.0,
            1.0,
            100.0,
            1_000.0,
        );
        assert_eq!(first_subpixel_move.thumb.y, 13.0);
    }

    #[test]
    fn track_click_uses_chromiums_linux_page_step() {
        let bar = LayoutScrollbarGeometry::new(
            LayoutScrollbarAxis::Vertical,
            LayoutRect::new(0.0, 0.0, 15.0, 200.0),
            0.0,
            1_000.0,
            400.0,
            200.0,
            1_200.0,
        );
        assert_eq!(bar.page_target(true), 575.0);
        assert_eq!(bar.page_target(false), 225.0);
    }
}
