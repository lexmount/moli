use super::*;
use crate::{
    DocumentLayoutServices, LayoutDisplay, LayoutElementCategory, LayoutElementSemantics,
    LayoutError, LayoutNamespace, LayoutSource, LayoutSourceKind, LayoutStyleResolver, LayoutWorld,
    PaintColor, ResolvedLayoutElementStyles, ResolvedLayoutStyle, build_layout_world,
    inline::collect_inline_input,
};

#[test]
fn short_text_preparation_checks_full_style_and_bounded_inputs() {
    let text = "office العربية";
    let base = PreparedStyle::default();
    let owner = LayoutBoxId::from_index(3);
    let mut index = InlinePreparationCache::default();
    index.insert(text, &base, true, owner);
    assert_eq!(index.find(text, &base, true), Some(owner));
    assert_eq!(index.find(text, &base, false), None);
    assert_eq!(index.find("different text", &base, true), None);

    let mut variants = vec![base.clone(); 11];
    variants[0].font_size += 1.0;
    variants[1].font_family = "serif".into();
    variants[2].font_features = "'liga' off".into();
    variants[3].font_variations = "'wght' 600".into();
    variants[4].locale = Some("ar".parse().unwrap());
    variants[5].letter_spacing = 1.0;
    variants[6].word_spacing = 1.0;
    variants[7].line_height = parley::LineHeight::Absolute(40.0);
    variants[8].brush.color = PaintColor::new(1.0, 0.0, 0.0, 1.0);
    variants[9].text_wrap_mode = parley::TextWrapMode::NoWrap;
    variants[10].brush.paint = false;
    for style in variants {
        assert!(index.find(text, &style, true).is_none(), "{style:?}");
    }

    let styles = [base.clone()];
    let runs = [(0..text.len(), 0)];
    assert!(candidate_style(text, &styles, &runs, true).is_some());
    assert!(candidate_style(text, &styles, &runs, false).is_none());
    assert!(candidate_style(text, &styles, &[(1..text.len(), 0)], true).is_none());
    assert!(candidate_style(text, &[base.clone(), base.clone()], &runs, true).is_none());
    assert!(candidate_style(&"a".repeat(MAX_TEXT_BYTES + 1), &styles, &runs, true).is_none());
    let oversized = PreparedStyle {
        font_family: FontFamily::Source("x".repeat(257).into()),
        ..base.clone()
    };
    assert!(candidate_style(text, &[oversized], &runs, true).is_none());

    for i in 0..MAX_ENTRIES + 1 {
        index.insert(&format!("word {i}"), &base, true, owner);
    }
    assert_eq!(index.entries.len(), MAX_ENTRIES);
    assert!(index.find(text, &base, true).is_none());
    assert_eq!(index.find("word 64", &base, true), Some(owner));
}

struct TextRows {
    texts: Vec<String>,
    children: Vec<Vec<usize>>,
}

impl TextRows {
    fn new() -> Self {
        let samples = [
            "Alpha Beta Gamma",
            "office ffi fl",
            "العربية لا",
            "שלום 🙂",
            "e\u{301} A\u{30a}",
            "first\nsecond",
        ];
        let texts = (0..120)
            .map(|i| samples[i % samples.len()].into())
            .collect::<Vec<_>>();
        let mut children = vec![(0..texts.len()).map(|i| i * 2 + 1).collect()];
        for i in 0..texts.len() {
            children.push(vec![i * 2 + 2]);
            children.push(Vec::new());
        }
        Self { texts, children }
    }
}

impl LayoutSource for TextRows {
    type NodeId = usize;
    type ChildIter<'a> = std::iter::Copied<std::slice::Iter<'a, usize>>;

    fn root(&self) -> usize {
        0
    }
    fn flat_parent(&self, node: usize) -> Option<usize> {
        (node != 0).then(|| if node % 2 == 1 { 0 } else { node - 1 })
    }
    fn flat_children(&self, node: usize) -> Self::ChildIter<'_> {
        self.children[node].iter().copied()
    }
    fn node_kind(&self, node: usize) -> LayoutSourceKind {
        if node != 0 && node.is_multiple_of(2) {
            LayoutSourceKind::Text
        } else {
            LayoutSourceKind::Element
        }
    }
    fn element_semantics(&self, node: usize) -> Option<LayoutElementSemantics> {
        (self.node_kind(node) == LayoutSourceKind::Element).then(|| {
            LayoutElementSemantics::new(
                LayoutNamespace::Html,
                "div",
                LayoutElementCategory::Generic,
                None,
            )
        })
    }
    fn text(&self, node: usize) -> Option<&str> {
        (self.node_kind(node) == LayoutSourceKind::Text).then(|| self.texts[node / 2 - 1].as_str())
    }
    fn label(&self, node: usize) -> String {
        format!("node {node}")
    }
}

impl LayoutStyleResolver<usize> for TextRows {
    fn element_styles(
        &mut self,
        node: usize,
    ) -> Result<Option<ResolvedLayoutElementStyles>, LayoutError> {
        Ok(Some(ResolvedLayoutElementStyles::from_primary(
            ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                taffy::Style::default(),
                PaintColor::TRANSPARENT,
            )
            .with_text_metrics(if node % 4 == 1 { 16.0 } else { 20.0 }, 24.0),
        )))
    }
}

fn prepare_rows(
    services: &mut DocumentLayoutServices,
    evict_every: usize,
) -> (LayoutWorld<usize>, usize) {
    let mut world = build_layout_world(&TextRows::new(), &mut TextRows::new()).unwrap();
    world.prepare_inline_storage();
    let owners = world
        .boxes
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            b.inline_formatting_context
                .then_some(LayoutBoxId::from_index(i))
        })
        .collect::<Vec<_>>();
    let mut index = InlinePreparationCache::default();
    for (i, owner) in owners.into_iter().enumerate() {
        if i % evict_every == 0 {
            index.entries.clear();
        }
        let input = collect_inline_input(&mut world, owner);
        let context = input.build(
            &world,
            services.parley_mut(),
            owner,
            (evict_every != 1).then_some(&mut index),
        );
        world.inline_layouts[owner.index()] = Some(context);
    }
    (world, index.hits())
}

#[test]
fn short_text_preparation_matches_uncached_shaping_and_independent_line_breaks() {
    let mut services = DocumentLayoutServices::new();
    let (reference, hits) = prepare_rows(&mut services, 1);
    assert_eq!(hits, 0);
    for evict_every in [7, usize::MAX] {
        let (mut reused, hits) = prepare_rows(&mut services, evict_every);
        assert!(hits > 0, "fixture must exercise actual preparation reuse");
        for (expected, actual) in reference
            .inline_layouts
            .iter()
            .zip(&mut reused.inline_layouts)
        {
            let (Some(expected), Some(actual)) = (expected, actual) else {
                continue;
            };
            // Full data includes glyphs, fonts, style indices, clusters,
            // bidi levels and breaks, not just aggregate width/height.
            assert_eq!(format!("{actual:#?}"), format!("{expected:#?}"));
            for width in [20.0, 400.0, 70.0] {
                let actual = actual.measurement_layout.as_mut().unwrap();
                let mut expected = expected.measurement_layout.as_ref().unwrap().clone();
                actual.break_all_lines(Some(width));
                expected.break_all_lines(Some(width));
                assert_eq!(format!("{actual:#?}"), format!("{expected:#?}"));
            }
        }
    }
}
