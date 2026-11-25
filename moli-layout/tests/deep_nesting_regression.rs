use std::collections::{HashMap, HashSet};

use moli_layout::{
    DocumentLayoutServices, EmbeddedFrameRenderer, EmbeddedFrameSnapshot, LayoutDisplay,
    LayoutElementCategory, LayoutElementSemantics, LayoutError, LayoutFlushReason, LayoutListRole,
    LayoutNamespace, LayoutPassRequest, LayoutPosition, LayoutSource, LayoutSourceChange,
    LayoutSourceKind, LayoutStyleResolver, LayoutViewport, PaintColor, PaintRect, PaintViewport,
    ResolvedLayoutElementStyles, ResolvedLayoutStyle, ScreenshotLayoutRequest, build_layout_pass,
    build_screenshot_snapshot, finish_prepared_layout_pass, prepare_layout_pass_reusing,
};
use taffy::{Clear, Dimension, Display, Float, Rect, Size, Style, style_helpers::length};

const RED: PaintColor = PaintColor::new(1.0, 0.0, 0.0, 1.0);
const BLUE: PaintColor = PaintColor::new(0.0, 0.0, 1.0, 1.0);
const GREEN: PaintColor = PaintColor::new(0.0, 1.0, 0.0, 1.0);

#[derive(Clone, Debug)]
struct DeepNode {
    label: String,
    kind: LayoutSourceKind,
    semantics: Option<LayoutElementSemantics>,
    text: Option<String>,
    parent: Option<usize>,
    children: Vec<usize>,
}

#[derive(Debug)]
struct DeepSource {
    nodes: Vec<DeepNode>,
}

impl DeepSource {
    fn new_root(local_name: &str) -> Self {
        Self {
            nodes: vec![DeepNode {
                label: local_name.to_owned(),
                kind: LayoutSourceKind::Element,
                semantics: Some(LayoutElementSemantics::new(
                    LayoutNamespace::Html,
                    local_name,
                    LayoutElementCategory::Generic,
                    None,
                )),
                text: None,
                parent: None,
                children: Vec::new(),
            }],
        }
    }

    fn push_element(
        &mut self,
        parent: usize,
        local_name: &str,
        category: LayoutElementCategory,
    ) -> usize {
        let id = self.nodes.len();
        self.nodes.push(DeepNode {
            label: format!("{local_name}#{id}"),
            kind: LayoutSourceKind::Element,
            semantics: Some(LayoutElementSemantics::new(
                LayoutNamespace::Html,
                local_name,
                category,
                None,
            )),
            text: None,
            parent: Some(parent),
            children: Vec::new(),
        });
        self.nodes[parent].children.push(id);
        id
    }

    fn push_text(&mut self, parent: usize, text: &str) -> usize {
        let id = self.nodes.len();
        self.nodes.push(DeepNode {
            label: format!("text#{id}"),
            kind: LayoutSourceKind::Text,
            semantics: None,
            text: Some(text.to_owned()),
            parent: Some(parent),
            children: Vec::new(),
        });
        self.nodes[parent].children.push(id);
        id
    }
}

impl LayoutSource for DeepSource {
    type NodeId = usize;
    type ChildIter<'a> = std::iter::Copied<std::slice::Iter<'a, usize>>;

    fn root(&self) -> Self::NodeId {
        0
    }

    fn flat_parent(&self, node: Self::NodeId) -> Option<Self::NodeId> {
        self.nodes[node].parent
    }

    fn flat_children(&self, node: Self::NodeId) -> Self::ChildIter<'_> {
        self.nodes[node].children.iter().copied()
    }

    fn node_kind(&self, node: Self::NodeId) -> LayoutSourceKind {
        self.nodes[node].kind
    }

    fn element_semantics(&self, node: Self::NodeId) -> Option<LayoutElementSemantics> {
        self.nodes[node].semantics.clone()
    }

    fn text(&self, node: Self::NodeId) -> Option<&str> {
        self.nodes[node].text.as_deref()
    }

    fn label(&self, node: Self::NodeId) -> String {
        self.nodes[node].label.clone()
    }
}

#[derive(Default)]
struct DeepStyles {
    default_block: Option<ResolvedLayoutStyle>,
    overrides: HashMap<usize, ResolvedLayoutStyle>,
    reuse_clean: bool,
    dirty_nodes: HashSet<usize>,
}

impl LayoutStyleResolver<usize> for DeepStyles {
    fn element_styles(
        &mut self,
        node: usize,
    ) -> Result<Option<ResolvedLayoutElementStyles>, LayoutError> {
        let style = self
            .overrides
            .get(&node)
            .cloned()
            .or_else(|| self.default_block.clone())
            .unwrap_or_else(|| {
                ResolvedLayoutStyle::synthetic(
                    LayoutDisplay::Block,
                    Style::default(),
                    PaintColor::TRANSPARENT,
                )
            });
        Ok(Some(ResolvedLayoutElementStyles::from_primary(style)))
    }

    fn can_reuse_element_style(&mut self, node: usize, _previous: &ResolvedLayoutStyle) -> bool {
        self.reuse_clean && !self.dirty_nodes.contains(&node)
    }

    fn anonymous_style(
        &mut self,
        context: moli_layout::LayoutAnonymousStyleContext<'_, usize>,
        display: LayoutDisplay,
    ) -> Result<ResolvedLayoutStyle, LayoutError> {
        Ok(ResolvedLayoutStyle::anonymous_from(
            context.parent_style(),
            display,
        ))
    }
}

struct NoTestFrames;

impl EmbeddedFrameRenderer<usize> for NoTestFrames {
    fn render_embedded_frame(
        &mut self,
        _frame: usize,
        _viewport: LayoutViewport,
    ) -> Result<Option<EmbeddedFrameSnapshot<usize>>, LayoutError> {
        Ok(None)
    }
}

fn run_on_bounded_stack<F>(name: &str, f: F)
where
    F: FnOnce() + Send + 'static,
{
    // Use a 2 MiB stack (tighter than render_runtime's 8 MiB stack) so any
    // depth-proportional recursion in box construction, numeric layout,
    // rounding, coordinate resolution, clip projection, or paint ordering
    // fails deterministically instead of passing by luck on a huge test stack.
    let handle = std::thread::Builder::new()
        .name(name.to_owned())
        .stack_size(2 * 1024 * 1024)
        .spawn(f)
        .expect("spawn bounded-stack test thread");
    handle.join().expect("bounded-stack test thread panicked");
}

fn find_solid_rect(snapshot: &moli_layout::PaintSnapshot, color: PaintColor) -> PaintRect {
    snapshot
        .fragments
        .iter()
        .find_map(|fragment| {
            fragment
                .solid_fill_in_surface()
                .filter(|(_, actual)| *actual == color)
                .map(|(rect, _)| rect)
        })
        .unwrap_or_else(|| panic!("missing {color:?} rect in paint snapshot"))
}

#[test]
fn deeply_nested_block_divs_layout_and_screenshot_without_stack_overflow() {
    run_on_bounded_stack("deep_divs_2000", || {
        let depth = 2000usize;
        let mut source = DeepSource::new_root("div");
        let mut current = 0usize;
        let mut styles = DeepStyles::default();
        let pad_every = 100usize;
        let padded_levels = depth / pad_every;
        for level in 1..=depth {
            current = source.push_element(current, "div", LayoutElementCategory::Generic);
            if level % pad_every == 0 {
                styles.overrides.insert(
                    current,
                    ResolvedLayoutStyle::synthetic(
                        LayoutDisplay::Block,
                        Style {
                            padding: Rect {
                                left: length(1.0),
                                right: length(0.0),
                                top: length(1.0),
                                bottom: length(0.0),
                            },
                            ..Style::default()
                        },
                        PaintColor::TRANSPARENT,
                    ),
                );
            }
        }
        let leaf = source.push_element(current, "div", LayoutElementCategory::Generic);
        styles.overrides.insert(
            leaf,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: Dimension::length(40.0),
                        height: Dimension::length(20.0),
                    },
                    ..Style::default()
                },
                RED,
            ),
        );

        let viewport = PaintViewport::new(400, 300, 1.0);
        let pass = build_layout_pass(
            &source,
            &mut styles,
            &mut DocumentLayoutServices::new(),
            LayoutPassRequest::with_paint(viewport, LayoutFlushReason::Screenshot),
        )
        .expect("deep block layout pass should succeed");

        let expected_offset = padded_levels as f32;
        let leaf_rects = pass.tree.client_rects_for_source(leaf);
        assert_eq!(leaf_rects.len(), 1, "innermost leaf must produce one rect");
        let leaf_doc_rect = leaf_rects[0].bounding_rect();
        assert!(
            (leaf_doc_rect.x - expected_offset).abs() < 0.05,
            "{leaf_doc_rect:?}"
        );
        assert!(
            (leaf_doc_rect.y - expected_offset).abs() < 0.05,
            "{leaf_doc_rect:?}"
        );
        assert!(
            (leaf_doc_rect.width - 40.0).abs() < 0.05,
            "{leaf_doc_rect:?}"
        );
        assert!(
            (leaf_doc_rect.height - 20.0).abs() < 0.05,
            "{leaf_doc_rect:?}"
        );

        let snapshot = pass
            .into_paint_snapshot()
            .expect("paint snapshot should be produced");
        let rect = find_solid_rect(&snapshot, RED);
        assert!((rect.x - expected_offset).abs() < 0.05, "{rect:?}");
        assert!((rect.y - expected_offset).abs() < 0.05, "{rect:?}");
        assert!((rect.width - 40.0).abs() < 0.05, "{rect:?}");
        assert!((rect.height - 20.0).abs() < 0.05, "{rect:?}");
    });
}

#[test]
fn deeply_nested_5000_divs_with_text_layout_and_paint() {
    run_on_bounded_stack("deep_divs_5000", || {
        let depth = 5000usize;
        let mut source = DeepSource::new_root("div");
        let mut current = 0usize;
        for _ in 0..depth {
            current = source.push_element(current, "div", LayoutElementCategory::Generic);
        }
        let leaf = current;
        let _text = source.push_text(leaf, "x");
        let mut styles = DeepStyles::default();
        let viewport = PaintViewport::new(400, 300, 1.0);
        let pass = build_layout_pass(
            &source,
            &mut styles,
            &mut DocumentLayoutServices::new(),
            LayoutPassRequest::with_paint(viewport, LayoutFlushReason::Screenshot),
        )
        .expect("5000-deep div pass should succeed");
        let leaf_rects = pass.tree.client_rects_for_source(leaf);
        assert_eq!(leaf_rects.len(), 1, "deepest div must exist in frozen tree");
        let leaf_rect = leaf_rects[0].bounding_rect();
        assert!(leaf_rect.width > 0.0);
        assert!(leaf_rect.height > 0.0);
    });
}

#[test]
fn deeply_nested_inline_spans_lists_contents_flex_grid_and_stacking_contexts() {
    run_on_bounded_stack("deep_siblings", || {
        let viewport = PaintViewport::new(400, 300, 1.0);

        // 1. Deeply nested inline spans (inline::collect_box + builder + stacking)
        {
            let mut source = DeepSource::new_root("div");
            let mut current = 0usize;
            let mut styles = DeepStyles::default();
            let inline_style = ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Inline,
                Style::default(),
                PaintColor::TRANSPARENT,
            );
            for _ in 0..2000 {
                current = source.push_element(current, "span", LayoutElementCategory::Generic);
                styles.overrides.insert(current, inline_style.clone());
            }
            source.push_text(current, "hello");
            let snapshot = build_screenshot_snapshot(
                &source,
                &mut styles,
                &mut DocumentLayoutServices::new(),
                ScreenshotLayoutRequest::new(viewport),
            )
            .expect("2000-deep inline spans should succeed");
            assert!(snapshot.content_size.height > 0.0);
        }

        // 2. Deeply nested list container -> divs -> li (list::collect_container_items)
        {
            let mut source = DeepSource::new_root("ul");
            source.nodes[0].semantics = Some(LayoutElementSemantics::new(
                LayoutNamespace::Html,
                "ul",
                LayoutElementCategory::List(LayoutListRole::Container),
                None,
            ));
            let mut current = 0usize;
            for _ in 0..2000 {
                current = source.push_element(current, "div", LayoutElementCategory::Generic);
            }
            let item = source.push_element(
                current,
                "li",
                LayoutElementCategory::List(LayoutListRole::Item),
            );
            source.push_text(item, "item");
            let mut styles = DeepStyles::default();
            styles.overrides.insert(
                item,
                ResolvedLayoutStyle::synthetic(
                    LayoutDisplay::BlockListItem,
                    Style::default(),
                    PaintColor::TRANSPARENT,
                ),
            );
            let snapshot = build_screenshot_snapshot(
                &source,
                &mut styles,
                &mut DocumentLayoutServices::new(),
                ScreenshotLayoutRequest::new(viewport),
            )
            .expect("2000-deep list item collection should succeed");
            assert!(snapshot.content_size.height > 0.0);
        }

        // 3. Deeply nested display:contents lifting (builder.rs)
        {
            let mut source = DeepSource::new_root("div");
            let mut current = 0usize;
            let mut styles = DeepStyles::default();
            let contents_style = ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Contents,
                Style::default(),
                PaintColor::TRANSPARENT,
            );
            for _ in 0..2000 {
                current = source.push_element(current, "div", LayoutElementCategory::Generic);
                styles.overrides.insert(current, contents_style.clone());
            }
            let leaf = source.push_element(current, "div", LayoutElementCategory::Generic);
            styles.overrides.insert(
                leaf,
                ResolvedLayoutStyle::synthetic(
                    LayoutDisplay::Block,
                    Style {
                        size: Size {
                            width: Dimension::length(30.0),
                            height: Dimension::length(15.0),
                        },
                        ..Style::default()
                    },
                    RED,
                ),
            );
            let snapshot = build_screenshot_snapshot(
                &source,
                &mut styles,
                &mut DocumentLayoutServices::new(),
                ScreenshotLayoutRequest::new(viewport),
            )
            .expect("2000-deep display:contents should succeed");
            let rect = find_solid_rect(&snapshot, RED);
            assert!((rect.width - 30.0).abs() < 0.05);
            assert!((rect.height - 15.0).abs() < 0.05);
        }

        // 4. Deeply nested stacking contexts + relative positioning + flex/grid
        {
            let mut source = DeepSource::new_root("div");
            let mut current = 0usize;
            let mut styles = DeepStyles::default();
            let flex_sc = ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Flex,
                Style {
                    display: Display::Flex,
                    ..Style::default()
                },
                PaintColor::TRANSPARENT,
            )
            .with_position(LayoutPosition::Relative)
            .with_z_index(1);
            let grid_rel = ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Grid,
                Style {
                    display: Display::Grid,
                    ..Style::default()
                },
                PaintColor::TRANSPARENT,
            )
            .with_position(LayoutPosition::Relative);
            for level in 0..1600 {
                current = source.push_element(current, "div", LayoutElementCategory::Generic);
                styles.overrides.insert(
                    current,
                    if level % 2 == 0 {
                        flex_sc.clone()
                    } else {
                        grid_rel.clone()
                    },
                );
            }
            let leaf = source.push_element(current, "div", LayoutElementCategory::Generic);
            styles.overrides.insert(
                leaf,
                ResolvedLayoutStyle::synthetic(
                    LayoutDisplay::Block,
                    Style {
                        size: Size {
                            width: Dimension::length(25.0),
                            height: Dimension::length(12.0),
                        },
                        ..Style::default()
                    },
                    RED,
                ),
            );
            let snapshot = build_screenshot_snapshot(
                &source,
                &mut styles,
                &mut DocumentLayoutServices::new(),
                ScreenshotLayoutRequest::new(viewport),
            )
            .expect("1600-deep flex/grid/stacking contexts should succeed");
            let rect = find_solid_rect(&snapshot, RED);
            assert!((rect.width - 25.0).abs() < 0.05);
            assert!((rect.height - 12.0).abs() < 0.05);
        }
    });
}

#[test]
fn deeply_nested_normal_flow_text_wraps_around_earlier_float_across_trampoline_boundary() {
    run_on_bounded_stack("deep_bfc_earlier_float", || {
        fn run_at_depth(depth: usize) -> (PaintRect, PaintRect, f32) {
            let mut source = DeepSource::new_root("div");
            let mut styles = DeepStyles::default();
            styles.overrides.insert(
                0,
                ResolvedLayoutStyle::synthetic(
                    LayoutDisplay::Block,
                    Style {
                        size: Size {
                            width: Dimension::length(220.0),
                            height: Dimension::auto(),
                        },
                        ..Style::default()
                    },
                    PaintColor::TRANSPARENT,
                ),
            );
            let float_box = source.push_element(0, "div", LayoutElementCategory::Generic);
            styles.overrides.insert(
                float_box,
                ResolvedLayoutStyle::synthetic(
                    LayoutDisplay::Block,
                    Style {
                        size: Size {
                            width: Dimension::length(100.0),
                            height: Dimension::length(80.0),
                        },
                        ..Style::default()
                    },
                    BLUE,
                )
                .with_float(Float::Left, Clear::None),
            );

            let mut current = 0usize;
            for _ in 0..depth {
                current = source.push_element(current, "div", LayoutElementCategory::Generic);
            }
            let text_container =
                source.push_element(current, "div", LayoutElementCategory::Generic);
            let inline_marker =
                source.push_element(text_container, "span", LayoutElementCategory::Generic);
            styles.overrides.insert(
                inline_marker,
                ResolvedLayoutStyle::synthetic(
                    LayoutDisplay::InlineBlock,
                    Style {
                        size: Size {
                            width: Dimension::length(60.0),
                            height: Dimension::length(20.0),
                        },
                        ..Style::default()
                    },
                    RED,
                ),
            );
            source.push_text(
                text_container,
                "one two three four five six seven eight nine ten eleven twelve thirteen fourteen",
            );

            let viewport = PaintViewport::new(300, 400, 1.0);
            let pass = build_layout_pass(
                &source,
                &mut styles,
                &mut DocumentLayoutServices::new(),
                LayoutPassRequest::with_paint(viewport, LayoutFlushReason::Screenshot),
            )
            .expect("layout pass should succeed");
            let text_box_rect =
                pass.tree.client_rects_for_source(text_container)[0].bounding_rect();
            let snapshot = pass
                .into_paint_snapshot()
                .expect("paint snapshot should be produced");
            let marker_rect = find_solid_rect(&snapshot, RED);
            let float_rect = find_solid_rect(&snapshot, BLUE);
            (float_rect, marker_rect, text_box_rect.height)
        }

        let (shallow_float, shallow_marker, shallow_text_height) = run_at_depth(4);
        let (deep_float, deep_marker, deep_text_height) = run_at_depth(120);

        assert_eq!(deep_float, shallow_float);
        assert!(
            (deep_marker.x - 100.0).abs() < 0.05,
            "deep inline content must be pushed right of the 100px float: {deep_marker:?}"
        );
        assert_eq!(
            deep_marker, shallow_marker,
            "deep inline marker across trampoline must match shallow BFC placement"
        );
        assert!(
            (deep_text_height - shallow_text_height).abs() < 0.05,
            "deep wrapped text height ({deep_text_height}) must match shallow ({shallow_text_height})"
        );
    });
}

#[test]
fn deeply_nested_float_propagates_to_outer_bfc_siblings_across_trampoline_boundary() {
    run_on_bounded_stack("deep_bfc_propagate_float", || {
        fn run_at_depth(depth: usize) -> (PaintRect, PaintRect, PaintRect, f32) {
            let mut source = DeepSource::new_root("div");
            let mut styles = DeepStyles::default();
            styles.overrides.insert(
                0,
                ResolvedLayoutStyle::synthetic(
                    LayoutDisplay::Block,
                    Style {
                        size: Size {
                            width: Dimension::length(220.0),
                            height: Dimension::auto(),
                        },
                        ..Style::default()
                    },
                    PaintColor::TRANSPARENT,
                ),
            );

            let mut current = 0usize;
            for _ in 0..depth {
                current = source.push_element(current, "div", LayoutElementCategory::Generic);
            }
            let deep_float = source.push_element(current, "div", LayoutElementCategory::Generic);
            styles.overrides.insert(
                deep_float,
                ResolvedLayoutStyle::synthetic(
                    LayoutDisplay::Block,
                    Style {
                        size: Size {
                            width: Dimension::length(100.0),
                            height: Dimension::length(80.0),
                        },
                        ..Style::default()
                    },
                    BLUE,
                )
                .with_float(Float::Left, Clear::None),
            );

            // Outer sibling with inline-block that must shift right of deep_float.
            let sibling_ifc = source.push_element(0, "div", LayoutElementCategory::Generic);
            let sibling_marker =
                source.push_element(sibling_ifc, "span", LayoutElementCategory::Generic);
            styles.overrides.insert(
                sibling_marker,
                ResolvedLayoutStyle::synthetic(
                    LayoutDisplay::InlineBlock,
                    Style {
                        size: Size {
                            width: Dimension::length(60.0),
                            height: Dimension::length(20.0),
                        },
                        ..Style::default()
                    },
                    RED,
                ),
            );

            // Outer sibling with clear: left that must be pushed below y=80.
            let sibling_cleared = source.push_element(0, "div", LayoutElementCategory::Generic);
            styles.overrides.insert(
                sibling_cleared,
                ResolvedLayoutStyle::synthetic(
                    LayoutDisplay::Block,
                    Style {
                        size: Size {
                            width: Dimension::length(50.0),
                            height: Dimension::length(16.0),
                        },
                        ..Style::default()
                    },
                    GREEN,
                )
                .with_float(Float::None, Clear::Left),
            );

            let viewport = PaintViewport::new(300, 400, 1.0);
            let pass = build_layout_pass(
                &source,
                &mut styles,
                &mut DocumentLayoutServices::new(),
                LayoutPassRequest::with_paint(viewport, LayoutFlushReason::Screenshot),
            )
            .expect("layout pass should succeed");
            let root_height = pass.tree.client_rects_for_source(0)[0]
                .bounding_rect()
                .height;
            let snapshot = pass
                .into_paint_snapshot()
                .expect("paint snapshot should be produced");
            (
                find_solid_rect(&snapshot, BLUE),
                find_solid_rect(&snapshot, RED),
                find_solid_rect(&snapshot, GREEN),
                root_height,
            )
        }

        let (shallow_float, shallow_marker, shallow_cleared, shallow_root_h) = run_at_depth(4);
        let (deep_float, deep_marker, deep_cleared, deep_root_h) = run_at_depth(120);

        assert_eq!(deep_float, shallow_float);
        assert!(
            (deep_marker.x - 100.0).abs() < 0.05,
            "outer sibling must avoid propagated deep float: {deep_marker:?}"
        );
        assert_eq!(deep_marker, shallow_marker);
        assert!(
            (deep_cleared.y - 80.0).abs() < 0.05,
            "outer cleared sibling must clear below propagated deep float: {deep_cleared:?}"
        );
        assert_eq!(deep_cleared, shallow_cleared);
        assert!(
            (deep_root_h - shallow_root_h).abs() < 0.05,
            "root BFC height ({deep_root_h}) must match shallow ({shallow_root_h})"
        );
    });
}

#[test]
fn deeply_nested_2000_divs_with_floats_layout_without_stack_overflow() {
    run_on_bounded_stack("deep_divs_2000_with_floats", || {
        let mut source = DeepSource::new_root("div");
        let mut styles = DeepStyles::default();
        styles.overrides.insert(
            0,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: Dimension::length(300.0),
                        height: Dimension::auto(),
                    },
                    ..Style::default()
                },
                PaintColor::TRANSPARENT,
            ),
        );
        let early_float = source.push_element(0, "div", LayoutElementCategory::Generic);
        styles.overrides.insert(
            early_float,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: Dimension::length(80.0),
                        height: Dimension::length(60.0),
                    },
                    ..Style::default()
                },
                BLUE,
            )
            .with_float(Float::Left, Clear::None),
        );

        let mut current = 0usize;
        for _ in 0..2000 {
            current = source.push_element(current, "div", LayoutElementCategory::Generic);
        }
        let deep_float = source.push_element(current, "div", LayoutElementCategory::Generic);
        styles.overrides.insert(
            deep_float,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: Dimension::length(70.0),
                        height: Dimension::length(90.0),
                    },
                    ..Style::default()
                },
                RED,
            )
            .with_float(Float::Left, Clear::None),
        );
        let cleared = source.push_element(0, "div", LayoutElementCategory::Generic);
        styles.overrides.insert(
            cleared,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: Dimension::length(50.0),
                        height: Dimension::length(20.0),
                    },
                    ..Style::default()
                },
                GREEN,
            )
            .with_float(Float::None, Clear::Left),
        );

        let viewport = PaintViewport::new(320, 400, 1.0);
        let snapshot = build_screenshot_snapshot(
            &source,
            &mut styles,
            &mut DocumentLayoutServices::new(),
            ScreenshotLayoutRequest::new(viewport),
        )
        .expect("2000-deep divs with floats should succeed on 2 MiB stack");

        let early_rect = find_solid_rect(&snapshot, BLUE);
        let deep_rect = find_solid_rect(&snapshot, RED);
        let cleared_rect = find_solid_rect(&snapshot, GREEN);
        assert!((early_rect.x - 0.0).abs() < 0.05, "{early_rect:?}");
        assert!(
            (deep_rect.x - 80.0).abs() < 0.05,
            "deep float at depth 2000 must sit next to early 80px float: {deep_rect:?}"
        );
        assert!(
            (cleared_rect.y - 90.0).abs() < 0.05,
            "outer cleared block must clear below 90px tall deep float: {cleared_rect:?}"
        );
    });
}

#[test]
fn deeply_nested_2000_divs_retained_world_update_without_stack_overflow() {
    run_on_bounded_stack("deep_divs_2000_retained_update", || {
        let mut source = DeepSource::new_root("div");
        let mut styles = DeepStyles::default();
        let mut current = 0usize;
        let mut mid_container = 0usize;
        let mut branch_parent = 0usize;
        for depth in 1..=2000 {
            current = source.push_element(current, "div", LayoutElementCategory::Generic);
            if depth == 1000 {
                mid_container = current;
            } else if depth == 1500 {
                branch_parent = current;
            }
        }
        let leaf = current;
        styles.overrides.insert(
            mid_container,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    padding: Rect {
                        left: length(5.0),
                        right: length(5.0),
                        top: length(5.0),
                        bottom: length(5.0),
                    },
                    ..Style::default()
                },
                PaintColor::TRANSPARENT,
            ),
        );
        styles.overrides.insert(
            leaf,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: Dimension::length(100.0),
                        height: Dimension::length(20.0),
                    },
                    ..Style::default()
                },
                RED,
            ),
        );

        let viewport = PaintViewport::new(320, 240, 1.0);
        let request = LayoutPassRequest::new(viewport, LayoutFlushReason::SynchronousGeometry)
            .with_retained_layout_world();
        let mut services = DocumentLayoutServices::new();

        // Pass 1: cold build producing a retained layout world.
        let prepared1 = prepare_layout_pass_reusing(&source, &mut styles, request, None, None)
            .expect("initial retained pass should prepare");
        let mut pass1 =
            finish_prepared_layout_pass(prepared1, &source, &mut services, &mut NoTestFrames)
                .expect("initial retained pass should finish");
        let retained1 = pass1
            .take_retained_layout()
            .expect("retained layout world should be returned");
        assert!(retained1.can_reuse_layout());

        // Pass 2: unchanged warm update must traverse the full 2,000-deep chain
        // iteratively and retain all boxes.
        styles.reuse_clean = true;
        let prepared2 =
            prepare_layout_pass_reusing(&source, &mut styles, request, Some(retained1), None)
                .expect("unchanged retained update should prepare");
        let mut pass2 =
            finish_prepared_layout_pass(prepared2, &source, &mut services, &mut NoTestFrames)
                .expect("unchanged retained update should finish");
        assert!(
            pass2.metrics.retained_box_count >= 2000,
            "expected >= 2000 retained boxes, got {}",
            pass2.metrics.retained_box_count
        );
        assert_eq!(
            pass2.tree.client_rects_for_source(leaf),
            pass1.tree.client_rects_for_source(leaf)
        );
        let retained2 = pass2
            .take_retained_layout()
            .expect("second retained layout world should be returned");

        // Pass 3: exercise in-place container restyle (depth 1000), in-place
        // leaf replacement (depth 2000), and iterative build_source_node for a
        // newly inserted 250-deep subtree under depth 1500.
        styles.overrides.insert(
            mid_container,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    padding: Rect {
                        left: length(15.0),
                        right: length(15.0),
                        top: length(15.0),
                        bottom: length(15.0),
                    },
                    ..Style::default()
                },
                PaintColor::TRANSPARENT,
            ),
        );
        styles.overrides.insert(
            leaf,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: Dimension::length(100.0),
                        height: Dimension::length(50.0),
                    },
                    ..Style::default()
                },
                BLUE,
            ),
        );
        let mut inserted = branch_parent;
        for _ in 0..250 {
            inserted = source.push_element(inserted, "div", LayoutElementCategory::Generic);
        }
        let inserted_leaf = inserted;
        styles.overrides.insert(
            inserted_leaf,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: Dimension::length(60.0),
                        height: Dimension::length(30.0),
                    },
                    ..Style::default()
                },
                GREEN,
            ),
        );
        styles.dirty_nodes.insert(mid_container);
        styles.dirty_nodes.insert(leaf);

        let prepared3 =
            prepare_layout_pass_reusing(&source, &mut styles, request, Some(retained2), None)
                .expect("mutated retained update should prepare");
        let mut pass3 =
            finish_prepared_layout_pass(prepared3, &source, &mut services, &mut NoTestFrames)
                .expect("mutated retained update should finish");
        assert!(
            pass3.metrics.retained_box_count >= 1900,
            "expected >= 1900 retained boxes after local mutation, got {}",
            pass3.metrics.retained_box_count
        );

        let cold = build_layout_pass(
            &source,
            &mut styles,
            &mut DocumentLayoutServices::new(),
            LayoutPassRequest::new(viewport, LayoutFlushReason::SynchronousGeometry),
        )
        .expect("cold reference pass should succeed");
        for node in [0, mid_container, branch_parent, leaf, inserted_leaf] {
            assert_eq!(
                pass3.tree.client_rects_for_source(node),
                cold.tree.client_rects_for_source(node),
                "retained update geometry diverged from cold build at node {node}"
            );
        }

        // Pass 4: exercise dirty-path pruning on a deep branch so the untouched
        // 250-deep sibling subtree under depth 1500 is skipped cleanly.
        let retained3 = pass3
            .take_retained_layout()
            .expect("third retained layout world should be returned");
        styles.dirty_nodes.clear();
        styles.overrides.insert(
            leaf,
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                Style {
                    size: Size {
                        width: Dimension::length(100.0),
                        height: Dimension::length(70.0),
                    },
                    ..Style::default()
                },
                RED,
            ),
        );
        styles.dirty_nodes.insert(leaf);
        let changes = [LayoutSourceChange::Node(leaf)];
        let prepared4 = prepare_layout_pass_reusing(
            &source,
            &mut styles,
            request,
            Some(retained3),
            Some(&changes),
        )
        .expect("dirty-pruned retained update should prepare");
        let pass4 =
            finish_prepared_layout_pass(prepared4, &source, &mut services, &mut NoTestFrames)
                .expect("dirty-pruned retained update should finish");
        assert!(
            pass4.metrics.pruned_source_count >= 250,
            "expected >= 250 pruned clean sources in untouched sibling branch, got {}",
            pass4.metrics.pruned_source_count
        );
        let cold4 = build_layout_pass(
            &source,
            &mut styles,
            &mut DocumentLayoutServices::new(),
            LayoutPassRequest::new(viewport, LayoutFlushReason::SynchronousGeometry),
        )
        .expect("cold reference pass after dirty-pruned update should succeed");
        for node in [0, mid_container, branch_parent, leaf, inserted_leaf] {
            assert_eq!(
                pass4.tree.client_rects_for_source(node),
                cold4.tree.client_rects_for_source(node),
                "dirty-pruned retained geometry diverged from cold build at node {node}"
            );
        }
    });
}
