use std::collections::HashMap;

use moli_layout::{
    DocumentLayoutServices, LayoutDisplay, LayoutElementCategory, LayoutElementSemantics,
    LayoutError, LayoutFlushReason, LayoutListRole, LayoutNamespace, LayoutPassRequest,
    LayoutPosition, LayoutSource, LayoutSourceKind, LayoutStyleResolver, PaintColor, PaintRect,
    PaintViewport, ResolvedLayoutElementStyles, ResolvedLayoutStyle, ScreenshotLayoutRequest,
    build_layout_pass, build_screenshot_snapshot,
};
use taffy::{Dimension, Display, Rect, Size, Style, style_helpers::length};

const RED: PaintColor = PaintColor::new(1.0, 0.0, 0.0, 1.0);

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
