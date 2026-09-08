//! Inline containing blocks are bounded by their first and last line, not by
//! the union of every line. The topology is pass-local; geometry is read from
//! the latest accepted layout, including after an enclosing abspos is sized.

use std::{collections::HashMap, fmt::Debug, hash::Hash};

use taffy::{AbsoluteAxis, Direction, Line, Rect, ResolveOrZero, WritingMode, prelude::TaffyZero};

use crate::{LayoutBoxId, LayoutBoxKind, LayoutWorld, PaintRect};

use super::unrounded_global_origin;

#[derive(Clone, Copy, Debug)]
enum FragmentSource {
    Inline(LayoutBoxId),
    BlockInInline(LayoutBoxId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContainingLine {
    Inline { owner: LayoutBoxId, index: usize },
    BlockInInline(LayoutBoxId),
}

#[derive(Default)]
pub(super) struct InlineContainingBlocks {
    sources: HashMap<LayoutBoxId, Vec<FragmentSource>>,
}

impl InlineContainingBlocks {
    pub(super) fn new<N: Copy + Debug + Eq + Hash>(world: &LayoutWorld<N>) -> Self {
        let mut result = Self {
            sources: world
                .boxes
                .iter()
                .filter_map(|layout_box| layout_box.positioned_containing_block)
                .filter(|id| world.boxes[id.index()].inline_flattened)
                .map(|id| (id, Vec::new()))
                .collect(),
        };
        if result.sources.is_empty() {
            return result;
        }
        let mut stack = vec![world.root];
        while let Some(id) = stack.pop() {
            let layout_box = &world.boxes[id.index()];
            stack.extend(layout_box.children.iter().rev().copied());
            if layout_box.inline_flattened && layout_box.style.display().is_inline_flow() {
                let principal = world.principal_inline_box(id);
                if let Some(sources) = result.sources.get_mut(&principal) {
                    sources.push(FragmentSource::Inline(id));
                }
            } else if layout_box.kind == LayoutBoxKind::BlockInInline {
                for principal in world.block_in_inline_ancestors(id) {
                    if let Some(sources) = result.sources.get_mut(&principal) {
                        sources.push(FragmentSource::BlockInInline(id));
                    }
                }
            }
        }
        result
    }

    pub(super) fn rect<N: Copy + Debug + Eq + Hash>(
        &self,
        world: &LayoutWorld<N>,
        principal: LayoutBoxId,
    ) -> Option<PaintRect> {
        let inline_box = &world.boxes[principal.index()];
        let owner = inline_box.inline_context_owner?;
        let container_style = &world.boxes[owner.index()].style;
        let mut geometry = None::<InlineContainingBlockGeometry>;
        let mut include = |rect, line, phantom| {
            if let Some(geometry) = &mut geometry {
                geometry.include(rect, line, phantom);
            } else {
                geometry = Some(InlineContainingBlockGeometry::new(rect, line));
            }
        };
        for source in self.sources.get(&principal)? {
            match *source {
                FragmentSource::Inline(id) => {
                    let owner = world.boxes[id.index()].inline_context_owner?;
                    let owner_box = &world.boxes[owner.index()];
                    let Some(context) = owner_box.inline_layout.as_ref() else {
                        continue;
                    };
                    let origin = unrounded_global_origin(world, owner);
                    let layout = owner_box.unrounded_layout;
                    for fragment in context
                        .fragments
                        .boxes
                        .iter()
                        .filter(|fragment| fragment.box_id == id)
                    {
                        let rect = fragment.box_model.border;
                        include(
                            PaintRect::new(
                                origin.x + layout.border.left + layout.padding.left + rect.x,
                                origin.y + layout.border.top + layout.padding.top + rect.y,
                                rect.width,
                                rect.height,
                            ),
                            ContainingLine::Inline {
                                owner,
                                index: fragment.line_index,
                            },
                            context.fragments.lines[fragment.line_index].phantom,
                        );
                    }
                }
                FragmentSource::BlockInInline(id) => {
                    let origin = unrounded_global_origin(world, id);
                    let size = world.boxes[id.index()].unrounded_layout.size;
                    include(
                        PaintRect::new(origin.x, origin.y, size.width, size.height),
                        ContainingLine::BlockInInline(id),
                        false,
                    );
                }
            }
        }
        let border = inline_box.style.taffy.border.resolve_or_zero(
            Some(world.boxes[owner.index()].unrounded_layout.size.width),
            crate::style::resolve_stylo_calc_value,
        );
        Some(geometry?.padding_rect(
            container_style.writing_mode(),
            container_style.taffy.direction,
            inline_box.style.taffy.direction,
            border,
        ))
    }
}

#[derive(Clone, Copy, Debug)]
struct InlineContainingBlockGeometry {
    first: PaintRect,
    last: PaintRect,
    first_line: ContainingLine,
    last_line: ContainingLine,
}

impl InlineContainingBlockGeometry {
    fn new(rect: PaintRect, line: ContainingLine) -> Self {
        Self {
            first: rect,
            last: rect,
            first_line: line,
            last_line: line,
        }
    }

    fn include(&mut self, rect: PaintRect, line: ContainingLine, phantom: bool) {
        if self.first_line == line {
            self.first = self.first.union(rect);
        }
        if self.last_line == line {
            self.last = self.last.union(rect);
        } else if !phantom {
            // A trailing empty line keeps its static position but does not
            // replace the last real line of the containing block (Blink's
            // GatherInlineContainerFragmentsFromItems).
            self.last = rect;
            self.last_line = line;
        }
    }

    fn padding_rect(
        self,
        mode: WritingMode,
        direction: Direction,
        inline_direction: Direction,
        border: Rect<f32>,
    ) -> PaintRect {
        let same_direction = direction == inline_direction;
        let axis = |axis, first: Line<f32>, last: Line<f32>, border: Line<f32>| {
            let border = if same_direction || mode.block_axis() == axis {
                border
            } else {
                Line::ZERO
            };
            if mode.is_axis_flow_reversed(axis, direction) {
                let start = first.end - border.end;
                Line {
                    start: (last.start + border.start).min(start),
                    end: start,
                }
            } else {
                let start = first.start + border.start;
                Line {
                    start,
                    end: (last.end - border.end).max(start),
                }
            }
        };
        let x = axis(
            AbsoluteAxis::Horizontal,
            Line {
                start: self.first.x,
                end: self.first.right(),
            },
            Line {
                start: self.last.x,
                end: self.last.right(),
            },
            Line {
                start: border.left,
                end: border.right,
            },
        );
        let y = axis(
            AbsoluteAxis::Vertical,
            Line {
                start: self.first.y,
                end: self.first.bottom(),
            },
            Line {
                start: self.last.y,
                end: self.last.bottom(),
            },
            Line {
                start: border.top,
                end: border.bottom,
            },
        );
        PaintRect::new(x.start, y.start, x.end - x.start, y.end - y.start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(index: usize) -> ContainingLine {
        ContainingLine::Inline {
            owner: LayoutBoxId::from_index(0),
            index,
        }
    }

    #[test]
    fn endpoints_unite_bidi_fragments_on_the_same_line_but_not_intermediate_lines() {
        let mut geometry =
            InlineContainingBlockGeometry::new(PaintRect::new(100.0, 10.0, 40.0, 10.0), line(0));
        geometry.include(PaintRect::new(80.0, 10.0, 20.0, 10.0), line(0), false);
        geometry.include(PaintRect::new(-10.0, 30.0, 300.0, 10.0), line(1), false);
        geometry.include(PaintRect::new(0.0, 50.0, 100.0, 10.0), line(2), false);
        geometry.include(PaintRect::new(100.0, 50.0, 40.0, 10.0), line(2), false);
        assert_eq!(
            geometry.padding_rect(
                WritingMode::HorizontalTb,
                Direction::Ltr,
                Direction::Ltr,
                Rect::ZERO
            ),
            PaintRect::new(80.0, 10.0, 60.0, 50.0)
        );
    }

    #[test]
    fn phantom_tail_does_not_replace_a_block_in_inline_fragment() {
        let mut geometry =
            InlineContainingBlockGeometry::new(PaintRect::new(0.0, 0.0, 0.0, 0.0), line(0));
        geometry.include(
            PaintRect::new(0.0, 0.0, 240.0, 30.0),
            ContainingLine::BlockInInline(LayoutBoxId::from_index(1)),
            false,
        );
        geometry.include(PaintRect::new(0.0, 30.0, 0.0, 0.0), line(1), true);
        assert_eq!(
            geometry.padding_rect(
                WritingMode::HorizontalTb,
                Direction::Ltr,
                Direction::Ltr,
                Rect::ZERO
            ),
            PaintRect::new(0.0, 0.0, 240.0, 30.0)
        );
    }

    #[test]
    fn inline_border_insets_follow_the_containers_direction() {
        let mut geometry =
            InlineContainingBlockGeometry::new(PaintRect::new(80.0, 10.0, 110.0, 16.0), line(0));
        geometry.include(PaintRect::new(0.0, 50.0, 150.0, 16.0), line(1), false);
        let border = Rect {
            left: 3.0,
            right: 3.0,
            top: 3.0,
            bottom: 3.0,
        };
        assert_eq!(
            geometry.padding_rect(
                WritingMode::HorizontalTb,
                Direction::Ltr,
                Direction::Ltr,
                border
            ),
            PaintRect::new(83.0, 13.0, 64.0, 50.0)
        );
        assert_eq!(
            geometry.padding_rect(
                WritingMode::HorizontalTb,
                Direction::Ltr,
                Direction::Rtl,
                border
            ),
            PaintRect::new(80.0, 13.0, 70.0, 50.0)
        );
    }

    #[test]
    fn logical_corners_support_vertical_flow_and_clamp_disjoint_endpoints() {
        let mut geometry =
            InlineContainingBlockGeometry::new(PaintRect::new(80.0, 20.0, 10.0, 50.0), line(0));
        geometry.include(PaintRect::new(30.0, 0.0, 10.0, 100.0), line(1), false);
        let border = Rect {
            left: 2.0,
            right: 2.0,
            top: 2.0,
            bottom: 2.0,
        };
        assert_eq!(
            geometry.padding_rect(
                WritingMode::VerticalRl,
                Direction::Ltr,
                Direction::Ltr,
                border
            ),
            PaintRect::new(32.0, 22.0, 56.0, 76.0)
        );
        assert_eq!(
            geometry.padding_rect(
                WritingMode::HorizontalTb,
                Direction::Ltr,
                Direction::Ltr,
                Rect::ZERO
            ),
            PaintRect::new(80.0, 20.0, 0.0, 80.0)
        );
    }
}
