use super::*;
use crate::{
    DocumentLayoutServices, LayoutFlushReason, LayoutPassRequest, LayoutTextSelection,
    LayoutViewport, PaintColor, ResolvedLayoutElementStyles, SystemFontPolicy, build_layout_pass,
};

struct Paragraph {
    text: &'static str,
    selection: LayoutTextSelection,
}

impl LayoutSource for Paragraph {
    type NodeId = u8;
    type ChildIter<'a> = std::option::IntoIter<u8>;

    fn root(&self) -> u8 {
        0
    }

    fn flat_parent(&self, node: u8) -> Option<u8> {
        (node == 1).then_some(0)
    }

    fn flat_children(&self, node: u8) -> Self::ChildIter<'_> {
        (node == 0).then_some(1).into_iter()
    }

    fn node_kind(&self, node: u8) -> LayoutSourceKind {
        if node == 0 {
            LayoutSourceKind::Element
        } else {
            LayoutSourceKind::Text
        }
    }

    fn element_semantics(&self, node: u8) -> Option<LayoutElementSemantics> {
        (node == 0).then(|| {
            LayoutElementSemantics::new(
                LayoutNamespace::Html,
                "div",
                LayoutElementCategory::Generic,
                None,
            )
        })
    }

    fn text(&self, node: u8) -> Option<&str> {
        (node == 1).then_some(self.text)
    }

    fn text_selection(&self, node: u8) -> Option<LayoutTextSelection> {
        (node == 1).then_some(self.selection)
    }

    fn label(&self, node: u8) -> String {
        format!("paragraph-{node}")
    }
}

struct ParagraphStyle;

impl LayoutStyleResolver<u8> for ParagraphStyle {
    fn element_styles(
        &mut self,
        node: u8,
    ) -> Result<Option<ResolvedLayoutElementStyles>, LayoutError> {
        Ok((node == 0).then(|| {
            ResolvedLayoutElementStyles::from_primary(ResolvedLayoutStyle::synthetic(
                LayoutDisplay::Block,
                taffy::Style::default(),
                PaintColor::TRANSPARENT,
            ))
        }))
    }

    fn can_reuse_element_style(&mut self, _node: u8, _previous: &ResolvedLayoutStyle) -> bool {
        true
    }
}

#[test]
fn text_edit_preserves_shared_order_and_numeric_tree() {
    let before = Paragraph {
        text: "first paragraph",
        selection: LayoutTextSelection::new(0, 5),
    };
    let after = Paragraph {
        text: "longer paragraph after an edit",
        selection: LayoutTextSelection::new(7, 16),
    };
    let mut styles = ParagraphStyle;
    let mut services = DocumentLayoutServices::with_system_font_policy(SystemFontPolicy::Disabled);
    let request =
        LayoutPassRequest::new(LayoutViewport::new(320, 200, 1.0), LayoutFlushReason::Test)
            .with_retained_layout_world()
            .with_screen_paint(true);
    let mut pass = build_layout_pass(&before, &mut styles, &mut services, request).unwrap();
    let mut retained = pass.take_retained_layout().unwrap();
    retained.discard_paint();
    let world = &mut retained.world;
    let id = world.source_box(1).unwrap();
    let storage_revision = world.boxes.storage_revision();
    assert!(world.numeric_tree.is_some());

    let (_, _, reuse_numeric) = update_layout_world(
        &after,
        &mut styles,
        world,
        Some(&[LayoutSourceChange::Node(1)]),
    )
    .unwrap()
    .unwrap();

    assert_eq!(world.source_box(1), Some(id));
    assert_eq!(world.boxes[id].text(), Some(after.text));
    assert_eq!(world.boxes[id].text_selection, Some(after.selection));
    assert_eq!(world.boxes.storage_revision(), storage_revision);
    assert!(reuse_numeric && world.numeric_tree.is_some());
    world.validate_invariants().unwrap();
}
