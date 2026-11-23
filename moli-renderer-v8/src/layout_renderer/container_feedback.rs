use moli_layout::{AutoScrollbars, LayoutError, MAX_RETAINED_LAYOUT_BOXES};
use moli_selector::QueryContainerSizes;

use crate::document_runtime::DomHandle;

// Bound full box-tree rebuild work independently of the number of query
// containers or scrollbar transitions. One large pass consumes more budget
// than one small pass.
const MAX_FEEDBACK_BOXES: usize = 16 * MAX_RETAINED_LAYOUT_BOXES;

#[derive(Default)]
pub(super) struct ContainerQueryFeedback {
    checkpoint: Option<Checkpoint>,
    pub(super) frozen_scrollbars: AutoScrollbars<DomHandle>,
    iterations: usize,
    boxes: usize,
}

struct Checkpoint {
    sizes: QueryContainerSizes,
    scrollbars: AutoScrollbars<DomHandle>,
    changed_axes: AutoScrollbars<DomHandle>,
    window: usize,
    steps: usize,
}

impl Checkpoint {
    fn new(
        sizes: QueryContainerSizes,
        scrollbars: AutoScrollbars<DomHandle>,
        window: usize,
    ) -> Self {
        Self {
            sizes,
            scrollbars,
            changed_axes: AutoScrollbars::default(),
            window,
            steps: 0,
        }
    }
}

impl ContainerQueryFeedback {
    /// With frozen gutters held fixed, the measured size table determines the
    /// next pass. Brent's expanding checkpoint detects exact repeats with a
    /// single retained table. Require a matching scrollbar snapshot too: the
    /// first occurrence of a size table can still carry gutters from the
    /// transient pass that produced it.
    pub(super) fn observe(
        &mut self,
        sizes: QueryContainerSizes,
        scrollbars: AutoScrollbars<DomHandle>,
    ) -> Result<bool, LayoutError> {
        let Some(checkpoint) = self.checkpoint.as_mut() else {
            self.checkpoint = Some(Checkpoint::new(sizes, scrollbars, 1));
            return Ok(false);
        };
        checkpoint.steps += 1;
        checkpoint
            .changed_axes
            .extend_changed_axes(&checkpoint.scrollbars, &scrollbars);
        if checkpoint.sizes == sizes && checkpoint.scrollbars == scrollbars {
            if self
                .frozen_scrollbars
                .freeze_axes(std::mem::take(&mut checkpoint.changed_axes))
            {
                self.checkpoint = None;
                return Ok(true);
            }
            return Err(LayoutError::ContainerQueryFeedbackCycle {
                period: checkpoint.steps,
            });
        }
        if checkpoint.steps == checkpoint.window {
            *checkpoint = Checkpoint::new(sizes, scrollbars, checkpoint.window.saturating_mul(2));
        }
        Ok(false)
    }

    pub(super) fn reserve_rebuild(&mut self, box_count: usize) -> Result<(), LayoutError> {
        self.boxes = self.boxes.saturating_add(box_count);
        if self.boxes > MAX_FEEDBACK_BOXES {
            return Err(LayoutError::ContainerQueryFeedbackBudgetExceeded {
                iterations: self.iterations,
                boxes: self.boxes,
                max_boxes: MAX_FEEDBACK_BOXES,
            });
        }
        self.iterations += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_units::Au;
    use euclid::default::Size2D;

    fn size_table(width: i32) -> QueryContainerSizes {
        [(
            DomHandle::new(1),
            Size2D::new(Some(Au::from_px(width)), None),
        )]
        .into_iter()
        .collect()
    }

    #[test]
    fn container_feedback_checks_actual_repeats_and_rebuild_work_separately() {
        let mut feedback = ContainerQueryFeedback::default();
        for width in 0..128 {
            assert_eq!(
                feedback.observe(size_table(width), AutoScrollbars::default()),
                Ok(false)
            );
            feedback.reserve_rebuild(20).unwrap();
        }

        let mut feedback = ContainerQueryFeedback::default();
        // The repeated tail has three states and contains no scrollbar axes.
        for width in [10, 20, 30, 40, 20, 30] {
            assert_eq!(
                feedback.observe(size_table(width), AutoScrollbars::default()),
                Ok(false)
            );
        }
        assert_eq!(
            feedback.observe(size_table(40), AutoScrollbars::default()),
            Err(LayoutError::ContainerQueryFeedbackCycle { period: 3 }),
        );

        let mut feedback = ContainerQueryFeedback::default();
        feedback.reserve_rebuild(MAX_FEEDBACK_BOXES).unwrap();
        assert_eq!(
            feedback.reserve_rebuild(1),
            Err(LayoutError::ContainerQueryFeedbackBudgetExceeded {
                iterations: 1,
                boxes: MAX_FEEDBACK_BOXES + 1,
                max_boxes: MAX_FEEDBACK_BOXES,
            }),
        );
    }
}
