use super::*;
use crate::{LayoutNamespace, LayoutSourceChange};

fn can_refresh_style(style: &ResolvedLayoutStyle) -> bool {
    style.uses_horizontal_writing_mode()
        && !style.is_out_of_flow()
        && !style.is_floated()
        && style
            .stylo_computed_values()
            .is_none_or(|values| !values.clone_container_type().is_size_container_type())
}

/// Reusable boxes can be refreshed without reconstructing their own principal
/// box. Their child streams may still need to be normalized again.
fn can_refresh_box<N>(layout_box: &crate::LayoutBox<N>) -> bool {
    matches!(
        layout_box.kind,
        LayoutBoxKind::PrincipalBlock
            | LayoutBoxKind::PrincipalFlowRoot
            | LayoutBoxKind::PrincipalInline
            | LayoutBoxKind::PrincipalInlineBlock
            | LayoutBoxKind::PrincipalFlex
            | LayoutBoxKind::PrincipalInlineFlex
            | LayoutBoxKind::PrincipalGrid
            | LayoutBoxKind::PrincipalInlineGrid
            | LayoutBoxKind::ListItem
            | LayoutBoxKind::TableWrapper
            | LayoutBoxKind::InlineTableWrapper
            | LayoutBoxKind::TableCaption
            | LayoutBoxKind::TableRowGroup
            | LayoutBoxKind::TableHeaderGroup
            | LayoutBoxKind::TableFooterGroup
            | LayoutBoxKind::TableRow
            | LayoutBoxKind::TableCell
            | LayoutBoxKind::LineBreak
            | LayoutBoxKind::Text
    ) && layout_box.element_semantics.as_ref().is_none_or(|element| {
        element.namespace == LayoutNamespace::Html
            && element.replaced.is_none()
            && matches!(
                element.category,
                LayoutElementCategory::Generic
                    | LayoutElementCategory::Table(_)
                    | LayoutElementCategory::List(_)
                    | LayoutElementCategory::LineBreak
            )
    }) && can_refresh_style(&layout_box.style)
        && (!crate::table::is_table_root(layout_box.kind)
            || (!layout_box.style.uses_fixed_table_layout()
                && !layout_box.style.table_border_is_collapsed()))
}

pub(crate) fn can_retain_root<N: Copy + std::fmt::Debug + Eq + std::hash::Hash>(
    world: &LayoutWorld<N>,
) -> bool {
    let root = &world.boxes[world.root];
    root.source.is_some()
        && !matches!(
            root.style.display(),
            LayoutDisplay::None | LayoutDisplay::Contents
        )
        && can_refresh_box(root)
}

fn construction_semantics_changed<S: LayoutSource>(
    source: &S,
    node: S::NodeId,
    layout_box: &crate::LayoutBox<S::NodeId>,
) -> bool {
    layout_box.element_semantics().is_some_and(|previous| {
        matches!(
            previous.category,
            LayoutElementCategory::Table(_) | LayoutElementCategory::List(_)
        ) && source.element_semantics(node).as_ref() != Some(previous)
    })
}

fn has_independent_child_stream(display: LayoutDisplay) -> bool {
    matches!(
        display,
        LayoutDisplay::Block | LayoutDisplay::FlowRoot | LayoutDisplay::Flex | LayoutDisplay::Grid
    )
}

struct RestyledChildren {
    before: Option<ResolvedLayoutPseudoStyle>,
    after: Option<ResolvedLayoutPseudoStyle>,
    restore_item_styles: bool,
}

struct RetainedBoxes<N> {
    sources: HashMap<N, LayoutBoxId>,
    css_image_sources: HashSet<N>,
    split_inline_sources: HashSet<N>,
    rebuilt_in_place: HashSet<LayoutBoxId>,
    pruned_sources: bool,
}

struct DirtyBoxPaths<N> {
    paths: HashSet<N>,
    subtrees: HashSet<N>,
}

impl<N: Copy + std::fmt::Debug + Eq + std::hash::Hash> DirtyBoxPaths<N> {
    fn collect(
        source: &impl LayoutSource<NodeId = N>,
        world: &LayoutWorld<N>,
        split_inline_sources: &HashSet<N>,
        changes: &[LayoutSourceChange<N>],
    ) -> Option<Self> {
        let mut dirty = Self {
            paths: HashSet::new(),
            subtrees: HashSet::new(),
        };
        for &change in changes {
            let node = match change {
                LayoutSourceChange::Node(node) => node,
                LayoutSourceChange::Subtree(node) => {
                    dirty.subtrees.insert(node);
                    node
                }
            };
            dirty.mark_path(source, node)?;
        }
        // Controls, markers, replaced content, and special formatting
        // streams still sample pass-local inputs or regenerate their boxes.
        for layout_box in world.boxes.values() {
            if let Some(node) = layout_box.source
                && (!can_refresh_box(layout_box)
                    || layout_box.text_selection.is_some()
                    || layout_box.style.display().is_list_item())
            {
                dirty.mark_path(source, node)?;
            }
        }
        for node in world
            .display_contents_mapping
            .keys()
            .copied()
            .chain(
                world
                    .css_image_references
                    .iter()
                    .map(|reference| reference.source),
            )
            .chain(split_inline_sources.iter().copied())
        {
            dirty.mark_path(source, node)?;
        }
        Some(dirty)
    }

    fn mark_path(&mut self, source: &impl LayoutSource<NodeId = N>, mut node: N) -> Option<()> {
        while self.paths.insert(node) && node != source.root() {
            node = source.flat_parent(node)?;
        }
        Some(())
    }
}

/// Refresh the owned formatting tree between publications. Construction
/// rebuilds a current source index while looking up boxes in the previous one.
/// A subtree that moves to a different owner takes the normal construction path.
pub(crate) fn update_layout_world<S, R>(
    source: &S,
    styles: &mut R,
    world: &mut LayoutWorld<S::NodeId>,
    changes: Option<&[LayoutSourceChange<S::NodeId>]>,
) -> Result<Option<(usize, usize, bool)>, LayoutError>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
{
    let root = world.root;
    let root_source = source.root();
    let root_box = &world.boxes[root];
    if root_box.source != Some(root_source)
        || source.flat_parent(root_source).is_some()
        || source.root_is_document_element() != world.root_is_document_element
        || source.quirks_mode() != world.quirks_mode
        || source.disables_viewport_user_scrolling()
            != world.viewport_scroll_policy.disables_user_scrolling()
        || construction_semantics_changed(source, root_source, root_box)
    {
        return Ok(None);
    }

    let mut builder = BoxBuilder::new(source, styles);
    builder.source_root = Some(root_source);
    builder.root_is_html = root_box
        .element_semantics()
        .is_some_and(|element| element.is_html_element("html"));
    let root_style = if builder
        .styles
        .can_reuse_element_style(root_source, &root_box.style)
    {
        None
    } else {
        if !can_refresh_box(root_box) {
            return Ok(None);
        }
        let Some(restyled) = builder.restyle_existing_container(world, root, root_source)? else {
            return Ok(None);
        };
        Some(restyled)
    };
    builder.seen_sources.insert(root_source, root_source);

    let original_count = world.boxes.len();
    let original_body = world.viewport_scroll_policy.defining_body();
    let mut had_floats = false;
    let mut split_inline_sources = HashSet::new();
    for layout_box in world.boxes.values() {
        had_floats = had_floats || layout_box.style.is_floated();
        if layout_box.kind == LayoutBoxKind::InlineContinuation
            && let Some(owner) = layout_box.owner
        {
            split_inline_sources.insert(owner);
        }
    }
    let dirty = changes
        .and_then(|changes| DirtyBoxPaths::collect(source, world, &split_inline_sources, changes));
    let mut retained = RetainedBoxes {
        sources: std::mem::take(&mut world.source_mapping),
        css_image_sources: world
            .css_image_references
            .drain(..)
            .map(|reference| reference.source)
            .collect(),
        split_inline_sources,
        rebuilt_in_place: HashSet::new(),
        pruned_sources: false,
    };
    if root_style.is_some() {
        retained.rebuilt_in_place.insert(root);
    }
    world.source_mapping.reserve(retained.sources.len());
    world.display_contents_mapping.clear();
    builder.refresh_box(
        world,
        root,
        root_source,
        root_style,
        dirty.as_ref(),
        &mut retained,
        &mut Vec::new(),
    )?;
    let order = world.preorder()?;
    let mut pruned_source_count = 0;
    if retained.pruned_sources {
        // Recreate the live source index during the existing dense box pass.
        // Clean branches never return to the DOM or style resolver.
        let already_mapped = world.source_mapping.len();
        for &id in &order {
            if let Some(node) = world.boxes[id].source {
                world.map_source(node, id);
            }
        }
        pruned_source_count = world.source_mapping.len() - already_mapped;
    }
    // The reused root still needs the policy derived from the current body.
    builder.finalize_viewport_scroll_policy(world);
    world.css_image_references = builder.css_image_references;
    let current_body = world.viewport_scroll_policy.defining_body();
    if original_body != current_body {
        for body in original_body.into_iter().chain(current_body) {
            world.clear_layout_caches(body);
        }
    }

    // Taffy's cache key does not include its shared block float context.
    // Preserve box construction work even when that context forces a cold
    // numeric pass.
    let retained_count = order
        .iter()
        .filter(|id| {
            world.boxes.index_of(*id) < original_count && !retained.rebuilt_in_place.contains(id)
        })
        .count();
    world.compact_for_reuse(order);
    let reuse_numeric_layout = !had_floats
        && !world
            .boxes
            .values()
            .any(|layout_box| layout_box.style.is_floated());
    Ok(Some((
        retained_count,
        pruned_source_count,
        reuse_numeric_layout,
    )))
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
        refreshed: Option<RestyledChildren>,
        dirty: Option<&DirtyBoxPaths<S::NodeId>>,
        retained: &mut RetainedBoxes<S::NodeId>,
        parent_children: &mut Vec<LayoutBoxId>,
    ) -> Result<bool, LayoutError> {
        // A cold rebuild below this retained box spends the same source
        // depth budget as the retained ancestors above it.
        self.enter_source(node)?;
        let result =
            self.refresh_entered_box(world, id, node, refreshed, dirty, retained, parent_children);
        self.active_sources.remove(&node);
        result
    }

    fn refresh_entered_box(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        id: LayoutBoxId,
        node: S::NodeId,
        refreshed: Option<RestyledChildren>,
        dirty: Option<&DirtyBoxPaths<S::NodeId>>,
        retained: &mut RetainedBoxes<S::NodeId>,
        parent_children: &mut Vec<LayoutBoxId>,
    ) -> Result<bool, LayoutError> {
        world.map_source(node, id);
        let restyled = refreshed.is_some();
        let layout_box = &mut world.boxes[id];
        // Labels are also used in diagnostic output after an ID change.
        layout_box.source_label = self.source.label(node);
        if layout_box.kind == LayoutBoxKind::Text {
            let selection = self.source.text_selection(node);
            let changed = layout_box.text_selection != selection;
            layout_box.text_selection = selection;
            parent_children.push(id);
            return Ok(changed);
        }
        layout_box.scroll_offset = self.sample_scroll_offset(node, &layout_box.style);
        if !restyled && retained.css_image_sources.contains(&node) {
            self.collect_css_images(node, &layout_box.style);
        }
        let element = layout_box
            .element_semantics
            .as_ref()
            .expect("a refreshed element has source semantics");
        self.record_viewport_body_candidate(node, element, &layout_box.style);
        if element
            .category
            .child_box_policy(false, layout_box.style.display())
            != ChildBoxPolicy::All
        {
            parent_children.push(id);
            return Ok(false);
        }
        let refreshed_text_style = (restyled && layout_box.inline_layout.is_some())
            .then(|| ResolvedLayoutStyle::text_leaf_from(&layout_box.style));
        let is_list_item = layout_box.style.display().is_list_item();
        let mut children = Vec::with_capacity(layout_box.children.len());
        let (before, after, restore_item_styles) = refreshed
            .map(|styles| (styles.before, styles.after, styles.restore_item_styles))
            .unwrap_or_default();
        if is_list_item {
            // Numbering is finalized later using the current list order.
            children.extend(self.build_list_marker(world, node, Some(id))?);
        }
        children.extend(self.build_pseudo(world, node, LayoutPseudo::Before, before)?);
        let mut changed = false;
        let mut reattach = restyled;
        let dirty = dirty.filter(|dirty| !restyled && !dirty.subtrees.contains(&node));
        for child in self.checked_flat_children(node)? {
            // Normalization may wrap a source child in an anonymous box. Its
            // structural owner still identifies the original child stream.
            let previous = retained
                .sources
                .get(&child)
                .copied()
                .filter(|previous| world.boxes[previous].structural_parent == Some(id));
            if let Some(previous) = previous {
                let old = &world.boxes[previous];
                // A previously split inline's boxes can extend beyond its
                // principal subtree. Rebuild its full contribution to the
                // parent's child stream together.
                if can_refresh_box(old) && !retained.split_inline_sources.contains(&child) {
                    if dirty.is_some_and(|dirty| !dirty.paths.contains(&child)) {
                        if let Some(element) = old.element_semantics() {
                            self.record_viewport_body_candidate(child, element, &old.style);
                        }
                        children.push(previous);
                        retained.pruned_sources = true;
                        continue;
                    }
                    let semantics_changed = construction_semantics_changed(self.source, child, old);
                    let source_kind = self.source.node_kind(child);
                    let current = !semantics_changed
                        && match source_kind {
                            LayoutSourceKind::Element => {
                                // A former grid or flex item may still carry a
                                // blockified numeric style. Reproject it when
                                // moving back into flow layout.
                                (!restore_item_styles || !old.style.display().is_inline_level())
                                    && self.styles.can_reuse_element_style(child, &old.style)
                            }
                            LayoutSourceKind::Text => {
                                (!restyled || refreshed_text_style.is_some())
                                    && old.kind == LayoutBoxKind::Text
                                    && self.source.text(child) == old.text.as_deref()
                            }
                            _ => false,
                        };
                    if current {
                        if source_kind == LayoutSourceKind::Text
                            && let Some(style) = &refreshed_text_style
                        {
                            world.boxes[previous].style = style.clone();
                        }
                        changed |= self.refresh_box(
                            world,
                            previous,
                            child,
                            None,
                            dirty,
                            retained,
                            &mut children,
                        )?;
                        continue;
                    }
                    if !semantics_changed
                        && !old.children.is_empty()
                        && let Some(styles) =
                            self.restyle_existing_container(world, previous, child)?
                    {
                        retained.rebuilt_in_place.insert(previous);
                        self.refresh_box(
                            world,
                            previous,
                            child,
                            Some(styles),
                            dirty,
                            retained,
                            &mut children,
                        )?;
                        changed = true;
                        continue;
                    }
                }
                changed = true;
            }

            let inherited = world.boxes[id].style.clone();
            let first_new = world.boxes.len();
            let may_replace_leaf = previous.is_some_and(|old| world.boxes[old].children.is_empty());
            let mut built = self.build_source_node(world, child, &inherited, may_replace_leaf)?;
            if let Some(previous) = previous
                && let [new] = built.as_mut_slice()
                && world.boxes.index_of(*new) == first_new
                && world.boxes.len() == first_new + 1
                && world.boxes[*new].source == Some(child)
                && world.boxes[previous].children.is_empty()
                && world.boxes[*new].children.is_empty()
            {
                world.replace_retained_leaf(previous, child);
                retained.rebuilt_in_place.insert(previous);
                reattach = true;
                *new = previous;
            }
            children.append(&mut built);
        }
        children.extend(self.build_pseudo(world, node, LayoutPseudo::After, after)?);
        if reattach || children != world.boxes[id].children {
            let old_children = std::mem::take(&mut world.boxes[id].children);
            let style = world.boxes[id].style.clone();
            let was_ifc = world.boxes[id].inline_formatting_context;
            let roots =
                self.attach_children(world, id, node, &style, children, id != world.root)?;
            let structure_changed =
                roots.as_slice() != [id] || world.boxes[id].children != old_children;
            if structure_changed || world.boxes[id].inline_formatting_context != was_ifc {
                world.numeric_tree = None;
            }
            changed |= structure_changed;
            parent_children.extend(roots);
        } else {
            parent_children.push(id);
        }
        if changed {
            world.invalidate_retained_inline(id);
            world.clear_layout_caches(id);
        }
        Ok(changed)
    }

    /// A new style on the same principal container invalidates its measured
    /// result. Its children keep their allocations and revalidate their own
    /// styles and constraints.
    fn restyle_existing_container(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        id: LayoutBoxId,
        node: S::NodeId,
    ) -> Result<Option<RestyledChildren>, LayoutError> {
        let old = &world.boxes[id];
        let Some(semantics) = self.source.element_semantics(node) else {
            return Ok(None);
        };
        if old.element_semantics() != Some(&semantics) {
            return Ok(None);
        }
        let Some(styles) = self.styles.element_styles(node)? else {
            return Ok(None);
        };
        let (style, before, after) = styles.into_parts();
        let display = style.display();
        let previous_display = old.style.display();
        if !can_refresh_style(&style)
            || ((display != previous_display || before.is_some() || after.is_some())
                && !(has_independent_child_stream(previous_display)
                    && has_independent_child_stream(display)))
        {
            return Ok(None);
        }
        let restore_item_styles = (previous_display.is_flex_container()
            || previous_display.is_grid_container())
            && !(display.is_flex_container() || display.is_grid_container());
        let kind = principal_kind(&semantics, &style);
        let replacement =
            self.principal_box(node, self.source.label(node), semantics, kind, style, None);
        world.replace_retained_box(id, replacement);
        Ok(Some(RestyledChildren {
            before,
            after,
            restore_item_styles,
        }))
    }
}
