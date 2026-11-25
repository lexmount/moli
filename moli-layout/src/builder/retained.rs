use super::*;
use crate::{LayoutElementCategory, LayoutNamespace};

/// The reusable tree has one box per source. Formatting that creates boxes
/// with additional identities still uses the normal construction path.
pub(crate) fn can_retain_world<N: Copy + std::fmt::Debug + Eq + std::hash::Hash>(
    world: &LayoutWorld<N>,
) -> bool {
    world.display_contents_mapping.is_empty()
        && world.css_image_references.is_empty()
        && world.boxes.iter().all(|layout_box| {
            layout_box.source.is_some()
                && layout_box.pseudo.is_none()
                && matches!(
                    layout_box.kind,
                    LayoutBoxKind::PrincipalBlock
                        | LayoutBoxKind::PrincipalInline
                        | LayoutBoxKind::PrincipalInlineBlock
                        | LayoutBoxKind::PrincipalFlex
                        | LayoutBoxKind::PrincipalInlineFlex
                        | LayoutBoxKind::LineBreak
                        | LayoutBoxKind::Text
                )
                && layout_box.element_semantics.as_ref().is_none_or(|element| {
                    element.namespace == LayoutNamespace::Html
                        && element.replaced.is_none()
                        && matches!(
                            element.category,
                            LayoutElementCategory::Generic | LayoutElementCategory::LineBreak
                        )
                })
                && layout_box.style.uses_horizontal_writing_mode()
                && !layout_box.style.is_out_of_flow()
                && !layout_box.style.is_floated()
                && layout_box.size_query_axes().is_none()
        })
}

/// Refresh an owned tree whose visible source topology is unchanged.
/// Reuse the normal box constructor for a changed leaf.
pub(crate) fn update_layout_world<S, R>(
    source: &S,
    styles: &mut R,
    world: &mut LayoutWorld<S::NodeId>,
) -> Result<Option<usize>, LayoutError>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
{
    let root = world.root;
    let root_source = source.root();
    let scroll_policy = world.viewport_scroll_policy;
    if world.boxes[root.index()].source != Some(root_source)
        || source.flat_parent(root_source).is_some()
        || source.root_is_document_element() != world.root_is_document_element
        || source.quirks_mode() != world.quirks_mode
        || source.disables_viewport_user_scrolling() != scroll_policy.disables_user_scrolling()
        || !styles.can_reuse_element_style(root_source, &world.boxes[root.index()].style)
    {
        return Ok(None);
    }

    let mut rebuilt = 0;
    let mut builder = BoxBuilder::new(source, styles);
    builder.seen_sources.insert(root_source, root_source);
    if builder
        .refresh_box(world, root, root_source, &mut rebuilt)?
        .is_none()
    {
        return Ok(None);
    }
    world.css_image_references = builder.css_image_references;
    Ok(can_retain_world(world).then_some(world.boxes.len() - rebuilt))
}

impl<S, R> BoxBuilder<'_, S, R>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
{
    fn refresh_box(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        id: LayoutBoxId,
        node: S::NodeId,
        rebuilt: &mut usize,
    ) -> Result<Option<bool>, LayoutError> {
        let layout_box = &mut world.boxes[id.index()];
        // Labels are also used in diagnostic output after an ID change.
        layout_box.source_label = self.source.label(node);
        if layout_box.kind == LayoutBoxKind::Text {
            layout_box.text_selection = self.source.text_selection(node);
            return Ok(Some(false));
        }
        layout_box.scroll_offset = self.sample_scroll_offset(node, &layout_box.style);
        let Some(element) = layout_box.element_semantics.as_ref() else {
            return Ok(None);
        };
        if element
            .category
            .child_box_policy(false, layout_box.style.display())
            != ChildBoxPolicy::All
        {
            return Ok(Some(false));
        }
        let mut children = Vec::with_capacity(layout_box.children.len());
        let mut changed = false;
        for child in self.checked_flat_children(node)? {
            let previous = world.source_box(child);
            if let Some(previous) = previous {
                let old = &world.boxes[previous.index()];
                if old.parent != Some(id) {
                    // The new parent's construction and inheritance may
                    // both change. Rebuild cross-parent moves as one pass.
                    return Ok(None);
                }
                let current = match self.source.node_kind(child) {
                    LayoutSourceKind::Element => {
                        self.styles.can_reuse_element_style(child, &old.style)
                    }
                    LayoutSourceKind::Text => {
                        old.kind == LayoutBoxKind::Text
                            && self.source.text(child) == old.text.as_deref()
                    }
                    _ => false,
                };
                if current {
                    let Some(child_changed) = self.refresh_box(world, previous, child, rebuilt)?
                    else {
                        return Ok(None);
                    };
                    changed |= child_changed;
                    children.push(previous);
                    continue;
                }
                if id == world.root || !old.children.is_empty() {
                    // The root and body determine viewport overflow policy.
                    return Ok(None);
                }
                world.source_mapping.remove(&child);
                changed = true;
            }

            let inherited = world.boxes[id.index()].style.clone();
            let first_new = world.boxes.len();
            let built = self.build_source_node(world, child, &inherited)?;
            match (previous, built.as_slice()) {
                (Some(previous), &[new])
                    if new.index() == first_new
                        && world.boxes.len() == first_new + 1
                        && world.boxes[new.index()].children.is_empty()
                        && world.boxes[previous.index()].kind == world.boxes[new.index()].kind =>
                {
                    world.replace_retained_leaf(previous, child);
                    *rebuilt += 1;
                    children.push(previous);
                }
                (None, []) => {}
                _ => return Ok(None),
            }
        }
        if children != world.boxes[id.index()].children {
            return Ok(None);
        }
        if changed {
            world.clear_layout_caches(id);
        }
        Ok(Some(changed))
    }
}
