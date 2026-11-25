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
                && layout_box.parent == layout_box.structural_parent
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

/// Refresh the owned formatting tree between publications. Existing boxes
/// with identical inputs keep their storage and numeric results. A changed
/// branch is constructed by the same builder as a fresh layout.
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

    let original_count = world.boxes.len();
    let mut rebuilt_in_place = HashSet::new();
    let mut builder = BoxBuilder::new(source, styles);
    builder.seen_sources.insert(root_source, root_source);
    if builder
        .refresh_box(world, root, root_source, false, &mut rebuilt_in_place)?
        .is_none()
    {
        return Ok(None);
    }
    world.css_image_references = builder.css_image_references;
    let order = world.preorder();
    let retained_count = order
        .iter()
        .filter(|id| id.index() < original_count && !rebuilt_in_place.contains(id))
        .count();
    world.compact_for_reuse(order);
    Ok(can_retain_world(world).then_some(retained_count))
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
        restyled_parent: bool,
        rebuilt_in_place: &mut HashSet<LayoutBoxId>,
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
        let mut replaced_child = restyled_parent;
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
                        !restyled_parent
                            && old.kind == LayoutBoxKind::Text
                            && self.source.text(child) == old.text.as_deref()
                    }
                    _ => false,
                };
                if current {
                    let Some(child_changed) =
                        self.refresh_box(world, previous, child, false, rebuilt_in_place)?
                    else {
                        return Ok(None);
                    };
                    changed |= child_changed;
                    children.push(previous);
                    continue;
                }
                if id == world.root {
                    // The root and body determine viewport overflow policy.
                    return Ok(None);
                }
                if !world.boxes[previous.index()].children.is_empty()
                    && self.restyle_existing_container(world, previous, child)?
                {
                    rebuilt_in_place.insert(previous);
                    if self
                        .refresh_box(world, previous, child, true, rebuilt_in_place)?
                        .is_none()
                    {
                        return Ok(None);
                    }
                    changed = true;
                    children.push(previous);
                    continue;
                }
                world.unmap_box_subtree(previous);
                changed = true;
            }

            let inherited = world.boxes[id.index()].style.clone();
            let first_new = world.boxes.len();
            let mut built = self.build_source_node(world, child, &inherited)?;
            if world.boxes[first_new..].iter().any(|layout_box| {
                layout_box
                    .source
                    .and_then(|source| world.source_box(source))
                    .is_some_and(|existing| existing.index() < first_new)
            }) {
                // A source moved into the rebuilt branch from another parent.
                return Ok(None);
            }
            if let Some(previous) = previous
                && let [new] = built.as_mut_slice()
                && new.index() == first_new
                && world.boxes.len() == first_new + 1
                && world.boxes[previous.index()].children.is_empty()
                && world.boxes[new.index()].children.is_empty()
            {
                world.replace_retained_leaf(previous, child);
                rebuilt_in_place.insert(previous);
                replaced_child = true;
                *new = previous;
            }
            children.append(&mut built);
        }
        if replaced_child || children != world.boxes[id.index()].children {
            if id == world.root {
                return Ok(None);
            }
            let old_children = std::mem::take(&mut world.boxes[id.index()].children);
            let style = world.boxes[id.index()].style.clone();
            if self.attach_children(world, id, node, &style, children, true)? != [id] {
                return Ok(None);
            }
            changed |= world.boxes[id.index()].children != old_children;
        }
        if changed {
            world.clear_layout_caches(id);
        }
        Ok(Some(changed))
    }
    /// A new style on the same principal container invalidates its measured
    /// result. Its children keep their allocations and revalidate their own
    /// styles and constraints.
    fn restyle_existing_container(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        id: LayoutBoxId,
        node: S::NodeId,
    ) -> Result<bool, LayoutError> {
        let old = &world.boxes[id.index()];
        let Some(semantics) = self.source.element_semantics(node) else {
            return Ok(false);
        };
        if old.element_semantics() != Some(&semantics) {
            return Ok(false);
        }
        let Some(styles) = self.styles.element_styles(node)? else {
            return Ok(false);
        };
        let (style, before, after) = styles.into_parts();
        if style.display() != old.style.display() || before.is_some() || after.is_some() {
            return Ok(false);
        }
        let replacement = self.principal_box(
            node,
            self.source.label(node),
            semantics,
            old.kind,
            style,
            None,
        );
        world.replace_retained_box(id, replacement);
        Ok(true)
    }
}
