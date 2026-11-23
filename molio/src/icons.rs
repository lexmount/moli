use std::{f32::consts::TAU, sync::OnceLock, time::Instant};

use gpui::{IntoElement, PathBuilder, Rgba, canvas, point, prelude::*, px};

#[derive(Clone, Copy)]
pub enum Icon {
    Back,
    Forward,
    Reload,
    Stop,
    Close,
    Add,
    Remove,
    Site,
    Menu,
    Page,
    Download,
    Loading,
}

pub fn glyph(icon: Icon, color: Rgba) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let p = |x, y| point(bounds.origin.x + px(x), bounds.origin.y + px(y));
            let mut path = PathBuilder::stroke(px(1.5));
            let mut lines = |segments: &[&[(f32, f32)]]| {
                for segment in segments {
                    let Some((&(x, y), rest)) = segment.split_first() else {
                        continue;
                    };
                    path.move_to(p(x, y));
                    for &(x, y) in rest {
                        path.line_to(p(x, y));
                    }
                }
            };
            match icon {
                Icon::Back => {
                    lines(&[&[(8.5, 3.), (3.5, 8.), (8.5, 13.)], &[(3.5, 8.), (13., 8.)]])
                }
                Icon::Forward => lines(&[
                    &[(7.5, 3.), (12.5, 8.), (7.5, 13.)],
                    &[(3., 8.), (12.5, 8.)],
                ]),
                Icon::Stop => lines(&[&[(4., 4.), (12., 4.), (12., 12.), (4., 12.), (4., 4.)]]),
                Icon::Close => lines(&[&[(4.5, 4.5), (11.5, 11.5)], &[(4.5, 11.5), (11.5, 4.5)]]),
                Icon::Add => lines(&[&[(3., 8.), (13., 8.)], &[(8., 3.), (8., 13.)]]),
                Icon::Remove => lines(&[&[(3., 8.), (13., 8.)]]),
                Icon::Site => lines(&[
                    &[(2.5, 5.), (13.5, 5.)],
                    &[(6., 3.), (6., 7.)],
                    &[(2.5, 11.), (13.5, 11.)],
                    &[(10., 9.), (10., 13.)],
                ]),
                Icon::Menu => lines(&[
                    &[(4., 7.25), (4., 8.75)],
                    &[(8., 7.25), (8., 8.75)],
                    &[(12., 7.25), (12., 8.75)],
                ]),
                Icon::Page => lines(&[
                    &[
                        (4., 2.5),
                        (9., 2.5),
                        (12., 5.5),
                        (12., 13.5),
                        (4., 13.5),
                        (4., 2.5),
                    ],
                    &[(9., 2.5), (9., 5.5), (12., 5.5)],
                ]),
                Icon::Download => lines(&[
                    &[(8., 2.5), (8., 10.)],
                    &[(4.75, 7.), (8., 10.25), (11.25, 7.)],
                    &[(3., 11.5), (3., 13.5), (13., 13.5), (13., 11.5)],
                ]),
                Icon::Reload => {
                    path.move_to(p(12.75, 5.4));
                    path.arc_to(point(px(5.2), px(5.2)), px(0.), true, false, p(13.2, 9.2));
                    path.move_to(p(9.6, 5.4));
                    path.line_to(p(12.75, 5.4));
                    path.line_to(p(12.75, 2.25));
                }
                Icon::Loading => {
                    static START: OnceLock<Instant> = OnceLock::new();
                    let phase = START.get_or_init(Instant::now).elapsed().as_secs_f32() * TAU * 1.4;
                    let endpoint = |angle: f32| p(8. + 5.2 * angle.cos(), 8. + 5.2 * angle.sin());
                    path.move_to(endpoint(phase));
                    path.arc_to(
                        point(px(5.2), px(5.2)),
                        px(0.),
                        true,
                        true,
                        endpoint(phase + TAU * 0.72),
                    );
                    window.request_animation_frame();
                }
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .size(px(16.))
    .flex_shrink_0()
}
