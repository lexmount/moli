// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Construction decisions are narrowly adapted from DioxusLabs/blitz commit
// d788124ab881f9bb537cb452ec1d837604a374a8,
// packages/blitz-dom/src/layout/{construct,inline,table,list,replaced}.rs. The
// source project is licensed MIT OR Apache-2.0. Anonymous-table insertion is
// aligned with Blink's LayoutObject/LayoutTable/LayoutTableSection/
// LayoutTableRow AddChild paths in the Chromium source pinned by the plan.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use crate::{
    LayoutAnonymousReason, LayoutBoxId, LayoutBoxKind, LayoutCapabilityDiagnostic, LayoutDisplay,
    LayoutElementCategory, LayoutElementSemantics, LayoutError, LayoutPseudo, LayoutSource,
    LayoutSourceKind, LayoutStyleResolver, LayoutWorld, ResolvedLayoutStyle,
    replaced::ReplacedContext, style::InlineWhiteSpaceCollapse,
};
use style::values::generics::image::GenericImage;

/// Constructs a complete pass-local CSS box tree from a borrowed source view.
pub fn build_layout_world<S, R>(
    source: &S,
    styles: &mut R,
) -> Result<LayoutWorld<S::NodeId>, LayoutError>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
{
    BoxBuilder::new(source, styles).build()
}

struct BoxBuilder<'a, S, R>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
{
    source: &'a S,
    styles: &'a mut R,
    active_sources: HashSet<S::NodeId>,
    seen_sources: HashMap<S::NodeId, String>,
}

impl<'a, S, R> BoxBuilder<'a, S, R>
where
    S: LayoutSource,
    R: LayoutStyleResolver<S::NodeId>,
{
    fn new(source: &'a S, styles: &'a mut R) -> Self {
        Self {
            source,
            styles,
            active_sources: HashSet::new(),
            seen_sources: HashMap::new(),
        }
    }

    fn build(mut self) -> Result<LayoutWorld<S::NodeId>, LayoutError> {
        let source_root = self.source.root();
        let source_label = self.source.label(source_root);
        if let Some(parent) = self.source.flat_parent(source_root) {
            return Err(LayoutError::source_contract(
                &source_label,
                format!(
                    "view root must not have a flat parent, got {}",
                    self.source.label(parent)
                ),
            ));
        }
        let source_kind = self.source.node_kind(source_root);
        if source_kind != LayoutSourceKind::Element {
            return Err(LayoutError::source_contract(
                &source_label,
                format!("view root must be an element, got {source_kind:?}"),
            ));
        }
        let root_semantics = self.validated_element_semantics(source_root, source_kind)?;
        let Some(mut root_style) = self.styles.primary_style(source_root)? else {
            return Err(LayoutError::MissingRootStyle { source_label });
        };
        let root_metrics = self.source.replaced_metrics(source_root);
        if root_semantics.is_hidden_input()
            || (root_style.display() == LayoutDisplay::Contents
                && root_semantics.display_contents_is_none())
        {
            // A LayoutWorld always needs a carrier root for Taffy and paint.
            // Preserve that internal carrier but give it the correct no-box
            // used display and never construct source descendants.
            root_style.force_display_none();
        }
        if root_semantics.is_replaced() {
            root_style.mark_replaced(natural_replaced_ratio(&root_semantics, root_metrics));
        } else if matches!(
            root_semantics.category,
            crate::LayoutElementCategory::FormControl(crate::LayoutFormControlKind::Button)
        ) {
            root_style.mark_intrinsic_form_control_container();
        }
        let root_kind = principal_kind(&root_semantics, &root_style);
        let root_generates_principal_box = root_style.display() != LayoutDisplay::None;
        let mut root = self.principal_box(
            source_root,
            source_label,
            root_semantics.clone(),
            root_kind,
            root_style.clone(),
            root_metrics,
        );
        if !root_generates_principal_box {
            // Taffy requires one root node even when the source root generates
            // no CSS box. Keep that carrier internal: exposing its zero-sized
            // fragment would make CSSOM geometry report a rect for
            // `display:none`.
            root.source = None;
        }
        let mut world = LayoutWorld::new(root);
        let root_box = world.root();
        if root_generates_principal_box {
            world.map_source(source_root, root_box);
        }
        self.seen_sources
            .insert(source_root, self.source.label(source_root));

        if !matches!(
            root_style.display(),
            LayoutDisplay::None | LayoutDisplay::Contents
        ) && !is_leaf_element(&root_semantics, root_kind, &root_style)
        {
            self.populate_root(&mut world, root_box, source_root, &root_style)?;
        }

        world.compact_reachable();
        world.validate_invariants()?;
        Ok(world)
    }

    fn populate_root(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        root_box: LayoutBoxId,
        source_node: S::NodeId,
        style: &ResolvedLayoutStyle,
    ) -> Result<(), LayoutError> {
        if !self.active_sources.insert(source_node) {
            return Err(LayoutError::SourceCycle {
                source_label: self.source.label(source_node),
            });
        }
        let result = (|| {
            let children = self.build_element_child_stream(world, source_node, style)?;
            let _ = self.attach_children(world, root_box, source_node, style, children, false)?;
            Ok(())
        })();
        self.active_sources.remove(&source_node);
        result
    }

    fn build_source_node(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        source_node: S::NodeId,
        inherited_style: &ResolvedLayoutStyle,
    ) -> Result<Vec<LayoutBoxId>, LayoutError> {
        let source_kind = self.source.node_kind(source_node);
        match source_kind {
            LayoutSourceKind::Comment | LayoutSourceKind::Other => {
                self.validate_non_element_semantics(source_node, source_kind)?;
                Ok(Vec::new())
            }
            LayoutSourceKind::Text => {
                self.validate_non_element_semantics(source_node, source_kind)?;
                let Some(text) = self.source.text(source_node) else {
                    return Ok(Vec::new());
                };
                let id = world.allocate(LayoutWorld::new_box(
                    Some(source_node),
                    None,
                    None,
                    self.source.label(source_node),
                    None,
                    None,
                    None,
                    LayoutBoxKind::Text,
                    ResolvedLayoutStyle::text_leaf_from(inherited_style),
                    Some(Arc::from(text)),
                    None,
                ));
                world.boxes[id.index()].text_selection = self.source.text_selection(source_node);
                world.map_source(source_node, id);
                Ok(vec![id])
            }
            LayoutSourceKind::Element => self.build_element(world, source_node),
        }
    }

    fn build_element(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        source_node: S::NodeId,
    ) -> Result<Vec<LayoutBoxId>, LayoutError> {
        if !self.active_sources.insert(source_node) {
            return Err(LayoutError::SourceCycle {
                source_label: self.source.label(source_node),
            });
        }

        let result = (|| {
            let semantics =
                self.validated_element_semantics(source_node, self.source.node_kind(source_node))?;
            let Some(mut style) = self.styles.primary_style(source_node)? else {
                return Ok(Vec::new());
            };
            let metrics = self.source.replaced_metrics(source_node);
            if semantics.is_replaced() {
                style.mark_replaced(natural_replaced_ratio(&semantics, metrics));
            } else if matches!(
                semantics.category,
                crate::LayoutElementCategory::FormControl(crate::LayoutFormControlKind::Button)
            ) {
                style.mark_intrinsic_form_control_container();
            }

            if semantics.is_hidden_input()
                || (style.display() == LayoutDisplay::Contents
                    && semantics.display_contents_is_none())
            {
                return Ok(Vec::new());
            }

            match style.display() {
                LayoutDisplay::None => Ok(Vec::new()),
                LayoutDisplay::Contents => {
                    let children = self.build_element_child_stream(world, source_node, &style)?;
                    world.map_display_contents_source(source_node, &children);
                    Ok(children)
                }
                _ => {
                    let kind = principal_kind(&semantics, &style);
                    let box_node = self.principal_box(
                        source_node,
                        self.source.label(source_node),
                        semantics.clone(),
                        kind,
                        style.clone(),
                        metrics,
                    );
                    let id = world.allocate(box_node);
                    world.map_source(source_node, id);

                    if is_leaf_element(&semantics, kind, &style) {
                        return Ok(vec![id]);
                    }

                    let children = self.build_element_child_stream(world, source_node, &style)?;
                    self.attach_children(world, id, source_node, &style, children, true)
                }
            }
        })();

        self.active_sources.remove(&source_node);
        result
    }

    fn principal_box(
        &self,
        source_node: S::NodeId,
        source_label: String,
        semantics: LayoutElementSemantics,
        kind: LayoutBoxKind,
        style: ResolvedLayoutStyle,
        metrics: Option<crate::ReplacedMetrics>,
    ) -> crate::LayoutBox<S::NodeId> {
        let mut layout_box = LayoutWorld::new_box(
            Some(source_node),
            None,
            None,
            source_label,
            None,
            Some(semantics.clone()),
            None,
            kind,
            style.clone(),
            None,
            metrics,
        );
        layout_box.replaced_image = self.source.replaced_image(source_node, &style);
        layout_box.css_images = self.css_image_resources(&style);
        layout_box.scroll_offset = self.source.scroll_offset(source_node);
        layout_box
    }

    fn build_element_child_stream(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        source_node: S::NodeId,
        style: &ResolvedLayoutStyle,
    ) -> Result<Vec<LayoutBoxId>, LayoutError> {
        let mut children = Vec::new();
        if style.display().is_list_item() {
            children.extend(self.build_pseudo(world, source_node, LayoutPseudo::Marker)?);
        }
        children.extend(self.build_pseudo(world, source_node, LayoutPseudo::Before)?);
        for child in self.checked_flat_children(source_node)? {
            children.extend(self.build_source_node(world, child, style)?);
        }
        children.extend(self.build_pseudo(world, source_node, LayoutPseudo::After)?);
        Ok(children)
    }

    fn build_pseudo(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        owner: S::NodeId,
        pseudo: LayoutPseudo,
    ) -> Result<Vec<LayoutBoxId>, LayoutError> {
        let Some(style) = self.styles.pseudo_style(owner, pseudo)? else {
            return Ok(Vec::new());
        };
        let is_marker = pseudo == LayoutPseudo::Marker;
        if style.display() == LayoutDisplay::None || !style.generates_pseudo_box(is_marker) {
            return Ok(Vec::new());
        }
        let generated_child = style
            .generated_text()
            .map(|text| self.allocate_generated_text(world, owner, pseudo, &style, text));

        // `display: contents` suppresses the pseudo's own principal box, but
        // its generated content still participates directly in the parent's
        // child stream. Keep pseudo ownership on that derived text box.
        if style.display() == LayoutDisplay::Contents {
            let Some(child) = generated_child else {
                return Ok(Vec::new());
            };
            if style.has_unsupported_generated_content()
                && let Some(layout_box) = world.box_by_id_mut(child)
            {
                push_diagnostic(
                    &mut layout_box.capability_diagnostics,
                    LayoutCapabilityDiagnostic::GeneratedContentUnsupported,
                );
            }
            return Ok(vec![child]);
        }

        let kind = match pseudo {
            LayoutPseudo::Marker => LayoutBoxKind::PseudoMarker,
            LayoutPseudo::Before => LayoutBoxKind::PseudoBefore,
            LayoutPseudo::After => LayoutBoxKind::PseudoAfter,
        };
        let mut layout_box = LayoutWorld::new_box(
            None,
            Some(owner),
            Some(pseudo),
            format!("{}{}", self.source.label(owner), pseudo.label()),
            Some(self.source.label(owner)),
            None,
            None,
            kind,
            style.clone(),
            None,
            None,
        );
        layout_box.css_images = self.css_image_resources(&style);
        if style.has_unsupported_generated_content() {
            push_diagnostic(
                &mut layout_box.capability_diagnostics,
                LayoutCapabilityDiagnostic::GeneratedContentUnsupported,
            );
        }
        let id = world.allocate(layout_box);
        let children = generated_child.into_iter().collect();
        self.attach_children(world, id, owner, &style, children, false)
    }

    fn allocate_generated_text(
        &self,
        world: &mut LayoutWorld<S::NodeId>,
        owner: S::NodeId,
        pseudo: LayoutPseudo,
        pseudo_style: &ResolvedLayoutStyle,
        text: &str,
    ) -> LayoutBoxId {
        world.allocate(LayoutWorld::new_box(
            None,
            Some(owner),
            Some(pseudo),
            format!("{}{}::text", self.source.label(owner), pseudo.label()),
            Some(self.source.label(owner)),
            None,
            None,
            LayoutBoxKind::Text,
            ResolvedLayoutStyle::text_leaf_from(pseudo_style),
            Some(Arc::from(text)),
            None,
        ))
    }

    fn attach_children(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        box_id: LayoutBoxId,
        owner: S::NodeId,
        parent_style: &ResolvedLayoutStyle,
        children: Vec<LayoutBoxId>,
        allow_inline_split: bool,
    ) -> Result<Vec<LayoutBoxId>, LayoutError> {
        let table_role = self.table_role(world, box_id)?;
        let children = if table_role == Some(TableBoxRole::Column) {
            Vec::new()
        } else if table_role == Some(TableBoxRole::ColumnGroup) {
            self.normalize_table_column_group(world, children)?
        } else if matches!(
            table_role,
            Some(TableBoxRole::Root | TableBoxRole::RowGroup | TableBoxRole::Row)
        ) {
            self.normalize_structural_table_children(
                world,
                owner,
                parent_style,
                table_role.expect("matched structural table role"),
                children,
            )?
        } else {
            self.normalize_missing_table_parents(world, owner, parent_style, children)?
        };

        if allow_inline_split
            && parent_style.display().is_inline_flow()
            && children
                .iter()
                .copied()
                .any(|child| self.is_block_in_flow(world, child))
        {
            return self.split_inline_box(world, box_id, owner, parent_style, children);
        }

        let children = if matches!(
            table_role,
            Some(
                TableBoxRole::Root
                    | TableBoxRole::RowGroup
                    | TableBoxRole::Row
                    | TableBoxRole::ColumnGroup
                    | TableBoxRole::Column
            )
        ) {
            children
        } else if parent_style.display().is_flex_container() {
            self.normalize_item_children(
                world,
                owner,
                parent_style,
                children,
                LayoutBoxKind::AnonymousFlexItem,
            )?
        } else if parent_style.display().is_grid_container() {
            self.normalize_item_children(
                world,
                owner,
                parent_style,
                children,
                LayoutBoxKind::AnonymousGridItem,
            )?
        } else {
            self.normalize_flow_children(world, owner, parent_style, children)?
        };

        self.replace_children_and_mark_context(world, box_id, children)?;
        Ok(vec![box_id])
    }

    fn split_inline_box(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        principal: LayoutBoxId,
        owner: S::NodeId,
        style: &ResolvedLayoutStyle,
        children: Vec<LayoutBoxId>,
    ) -> Result<Vec<LayoutBoxId>, LayoutError> {
        let semantics = world
            .box_by_id(principal)
            .and_then(|layout_box| layout_box.element_semantics.clone());
        let mut output = Vec::new();
        let mut current = Some(principal);
        let mut run = Vec::new();

        for child in children {
            if self.is_block_in_flow(world, child) {
                if let Some(fragment) = current.take() {
                    self.replace_children_and_mark_context(
                        world,
                        fragment,
                        std::mem::take(&mut run),
                    )?;
                    output.push(fragment);
                }
                output.push(child);
                continue;
            }

            if current.is_none() {
                current =
                    Some(self.allocate_inline_continuation(world, owner, style, semantics.clone()));
            }
            run.push(child);
        }

        if let Some(fragment) = current {
            self.replace_children_and_mark_context(world, fragment, run)?;
            output.push(fragment);
        } else {
            let continuation = self.allocate_inline_continuation(world, owner, style, semantics);
            self.replace_children_and_mark_context(world, continuation, Vec::new())?;
            output.push(continuation);
        }
        Ok(output)
    }

    fn allocate_inline_continuation(
        &self,
        world: &mut LayoutWorld<S::NodeId>,
        owner: S::NodeId,
        style: &ResolvedLayoutStyle,
        semantics: Option<LayoutElementSemantics>,
    ) -> LayoutBoxId {
        let mut continuation = LayoutWorld::new_box(
            None,
            Some(owner),
            None,
            format!("continuation({})", self.source.label(owner)),
            Some(self.source.label(owner)),
            semantics,
            Some(LayoutAnonymousReason::InlineSplitContinuation),
            LayoutBoxKind::InlineContinuation,
            style.clone(),
            None,
            None,
        );
        continuation.css_images = self.css_image_resources(style);
        world.allocate(continuation)
    }

    fn css_image_resources(
        &self,
        style: &ResolvedLayoutStyle,
    ) -> crate::source::LayoutCssImageResources {
        let Some(computed) = style.stylo_computed_values() else {
            return crate::source::LayoutCssImageResources::default();
        };
        let sample = |image: &style::values::computed::Image| match image {
            GenericImage::Url(url) => url
                .url()
                .and_then(|url| self.source.css_image_resource(url.as_str())),
            _ => None,
        };
        crate::source::LayoutCssImageResources {
            background: computed
                .get_background()
                .background_image
                .0
                .iter()
                .map(sample)
                .collect(),
            mask: computed.get_svg().mask_image.0.iter().map(sample).collect(),
        }
    }

    fn normalize_flow_children(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        owner: S::NodeId,
        parent_style: &ResolvedLayoutStyle,
        children: Vec<LayoutBoxId>,
    ) -> Result<Vec<LayoutBoxId>, LayoutError> {
        let has_inline = children
            .iter()
            .copied()
            .any(|id| self.is_meaningful_inline_in_flow(world, id));
        let has_block = children
            .iter()
            .copied()
            .any(|id| self.is_block_in_flow(world, id));

        if !has_block {
            if has_inline {
                return Ok(children);
            }
            return Ok(children
                .into_iter()
                .filter(|id| !self.is_ignorable_whitespace_text(world, *id))
                .collect());
        }
        if !has_inline {
            return Ok(children
                .into_iter()
                .filter(|id| !self.is_ignorable_whitespace_text(world, *id))
                .collect());
        }

        let mut output = Vec::new();
        let mut inline_run = Vec::new();
        for child in children {
            if self.is_block_in_flow(world, child) {
                self.flush_inline_run(
                    world,
                    owner,
                    parent_style,
                    LayoutBoxKind::AnonymousBlock,
                    &mut output,
                    &mut inline_run,
                )?;
                output.push(child);
            } else if self.is_inline_in_flow(world, child) {
                inline_run.push(child);
            } else {
                // Floated and absolutely/fixed-positioned children do not vote
                // in flow classification, but they remain direct children and
                // terminate the current anonymous inline run. This matches
                // Blitz's LayoutChildren::push boundary and Blink's anonymous
                // block construction around out-of-flow layout objects.
                self.flush_inline_run(
                    world,
                    owner,
                    parent_style,
                    LayoutBoxKind::AnonymousBlock,
                    &mut output,
                    &mut inline_run,
                )?;
                output.push(child);
            }
        }
        self.flush_inline_run(
            world,
            owner,
            parent_style,
            LayoutBoxKind::AnonymousBlock,
            &mut output,
            &mut inline_run,
        )?;
        Ok(output)
    }

    fn normalize_item_children(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        owner: S::NodeId,
        parent_style: &ResolvedLayoutStyle,
        children: Vec<LayoutBoxId>,
        anonymous_kind: LayoutBoxKind,
    ) -> Result<Vec<LayoutBoxId>, LayoutError> {
        let mut output = Vec::new();
        let mut text_run = Vec::new();
        for child in children {
            if world
                .box_by_id(child)
                .is_some_and(|layout_box| layout_box.kind.is_text())
            {
                text_run.push(child);
                continue;
            }
            self.flush_inline_run(
                world,
                owner,
                parent_style,
                anonymous_kind,
                &mut output,
                &mut text_run,
            )?;
            if let Some(layout_box) = world.box_by_id_mut(child)
                && !layout_box.style.is_out_of_flow()
            {
                layout_box.style.blockify_for_item();
            }
            output.push(child);
        }
        self.flush_inline_run(
            world,
            owner,
            parent_style,
            anonymous_kind,
            &mut output,
            &mut text_run,
        )?;
        Ok(output)
    }

    fn flush_inline_run(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        owner: S::NodeId,
        parent_style: &ResolvedLayoutStyle,
        anonymous_kind: LayoutBoxKind,
        output: &mut Vec<LayoutBoxId>,
        run: &mut Vec<LayoutBoxId>,
    ) -> Result<(), LayoutError> {
        if run.is_empty() {
            return Ok(());
        }
        if run
            .iter()
            .all(|id| self.is_ignorable_whitespace_text(world, *id))
        {
            run.clear();
            return Ok(());
        }
        let (reason, display) = match anonymous_kind {
            LayoutBoxKind::AnonymousBlock => (
                LayoutAnonymousReason::MixedFlowInlineRun,
                LayoutDisplay::Block,
            ),
            LayoutBoxKind::AnonymousFlexItem => {
                (LayoutAnonymousReason::FlexTextRun, LayoutDisplay::Block)
            }
            LayoutBoxKind::AnonymousGridItem => {
                (LayoutAnonymousReason::GridTextRun, LayoutDisplay::Block)
            }
            _ => {
                return Err(LayoutError::source_contract(
                    self.source.label(owner),
                    format!(
                        "box kind {} cannot be constructed as an anonymous inline-run wrapper",
                        anonymous_kind.debug_name()
                    ),
                ));
            }
        };
        let style = self.styles.anonymous_style(owner, parent_style, display)?;
        let mut anonymous = LayoutWorld::new_box(
            None,
            Some(owner),
            None,
            format!("anonymous({})", self.source.label(owner)),
            Some(self.source.label(owner)),
            None,
            Some(reason),
            anonymous_kind,
            style,
            None,
            None,
        );
        anonymous.inline_formatting_context = true;
        let id = world.allocate(anonymous);
        world.replace_children(id, std::mem::take(run))?;
        output.push(id);
        Ok(())
    }

    fn normalize_missing_table_parents(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        owner: S::NodeId,
        parent_style: &ResolvedLayoutStyle,
        children: Vec<LayoutBoxId>,
    ) -> Result<Vec<LayoutBoxId>, LayoutError> {
        let mut output = Vec::new();
        for (index, child) in children.iter().copied().enumerate() {
            // Chromium keeps table parts separated only by collapsible
            // whitespace in the same anonymous table wrapper. A real text run
            // still terminates that wrapper. This mirrors LayoutObject's
            // anonymous-table reuse after whitespace suppression.
            if self.is_ignorable_whitespace_text(world, child)
                && output.last().copied().is_some_and(|last| {
                    world.box_by_id(last).is_some_and(|layout_box| {
                        layout_box.kind == LayoutBoxKind::AnonymousTableWrapper
                            && layout_box.anonymous_reason
                                == Some(LayoutAnonymousReason::MissingTableParent)
                    })
                })
            {
                let next_table_part = children[index + 1..]
                    .iter()
                    .copied()
                    .find(|next| !self.is_ignorable_whitespace_text(world, *next))
                    .map(|next| self.table_role(world, next))
                    .transpose()?
                    .flatten()
                    .is_some_and(TableBoxRole::requires_parent);
                if next_table_part {
                    continue;
                }
            }
            self.insert_flow_child(world, owner, parent_style, &mut output, child)?;
        }
        Ok(output)
    }

    fn insert_flow_child(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        owner: S::NodeId,
        parent_style: &ResolvedLayoutStyle,
        output: &mut Vec<LayoutBoxId>,
        child: LayoutBoxId,
    ) -> Result<(), LayoutError> {
        let child_role = self.table_role(world, child)?;
        if !child_role.is_some_and(TableBoxRole::requires_parent) {
            output.push(child);
            return Ok(());
        }

        // Blink only creates an inline anonymous table when the actual parent
        // is a LayoutInline. Atomic inline-level containers (inline-block,
        // inline-flex/grid/table) establish their own inner formatting
        // context and therefore receive a block-level anonymous table.
        let display = if parent_style.display().is_inline_flow() {
            LayoutDisplay::InlineTable
        } else {
            LayoutDisplay::Table
        };
        let table = self.ensure_anonymous_wrapper(
            world,
            owner,
            parent_style,
            output,
            LayoutBoxKind::AnonymousTableWrapper,
            LayoutAnonymousReason::MissingTableParent,
            display,
        )?;
        self.insert_into_existing_table_parent(world, owner, table, child)
    }

    fn normalize_structural_table_children(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        owner: S::NodeId,
        parent_style: &ResolvedLayoutStyle,
        parent_role: TableBoxRole,
        children: Vec<LayoutBoxId>,
    ) -> Result<Vec<LayoutBoxId>, LayoutError> {
        let mut output = Vec::new();
        for child in children {
            if self.is_ignorable_whitespace_text(world, child) {
                continue;
            }
            self.insert_table_child(world, owner, parent_style, parent_role, &mut output, child)?;
        }
        Ok(output)
    }

    fn normalize_table_column_group(
        &self,
        world: &LayoutWorld<S::NodeId>,
        children: Vec<LayoutBoxId>,
    ) -> Result<Vec<LayoutBoxId>, LayoutError> {
        children
            .into_iter()
            .filter_map(|child| match self.table_role(world, child) {
                Ok(Some(TableBoxRole::Column)) => Some(Ok(child)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    fn insert_into_existing_table_parent(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        owner: S::NodeId,
        parent: LayoutBoxId,
        child: LayoutBoxId,
    ) -> Result<(), LayoutError> {
        let (parent_style, mut children) = {
            let parent_box = world
                .box_by_id(parent)
                .ok_or(LayoutError::InvalidBoxReference {
                    index: parent.index(),
                })?;
            (parent_box.style.clone(), parent_box.children.clone())
        };
        let parent_role = self.table_role(world, parent)?.ok_or_else(|| {
            LayoutError::source_contract(
                self.source.label(owner),
                "non-table box cannot own structural table children",
            )
        })?;
        self.insert_table_child(
            world,
            owner,
            &parent_style,
            parent_role,
            &mut children,
            child,
        )?;
        world.replace_children(parent, children)
    }

    fn insert_table_child(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        owner: S::NodeId,
        parent_style: &ResolvedLayoutStyle,
        parent_role: TableBoxRole,
        output: &mut Vec<LayoutBoxId>,
        child: LayoutBoxId,
    ) -> Result<(), LayoutError> {
        let child_role = self.table_role(world, child)?;
        if parent_role == TableBoxRole::Root {
            if child_role.is_some_and(TableBoxRole::is_direct_table_child) {
                output.push(child);
                return Ok(());
            }
            let row_group = self.ensure_anonymous_wrapper(
                world,
                owner,
                parent_style,
                output,
                LayoutBoxKind::AnonymousTableRowGroup,
                LayoutAnonymousReason::MissingTableRowGroup,
                LayoutDisplay::TableRowGroup,
            )?;
            return self.insert_into_existing_table_parent(world, owner, row_group, child);
        }

        if parent_role == TableBoxRole::RowGroup {
            if child_role == Some(TableBoxRole::Row) {
                output.push(child);
                return Ok(());
            }
            let row = self.ensure_anonymous_wrapper(
                world,
                owner,
                parent_style,
                output,
                LayoutBoxKind::AnonymousTableRow,
                LayoutAnonymousReason::MissingTableRow,
                LayoutDisplay::TableRow,
            )?;
            return self.insert_into_existing_table_parent(world, owner, row, child);
        }

        if parent_role == TableBoxRole::Row {
            if child_role == Some(TableBoxRole::Cell) {
                output.push(child);
                return Ok(());
            }
            let cell = self.ensure_anonymous_wrapper(
                world,
                owner,
                parent_style,
                output,
                LayoutBoxKind::AnonymousTableCell,
                LayoutAnonymousReason::MissingTableCell,
                LayoutDisplay::TableCell,
            )?;
            let (cell_style, mut cell_children) = {
                let cell_box = world
                    .box_by_id(cell)
                    .ok_or(LayoutError::InvalidBoxReference {
                        index: cell.index(),
                    })?;
                (cell_box.style.clone(), cell_box.children.clone())
            };
            self.insert_flow_child(world, owner, &cell_style, &mut cell_children, child)?;
            let cell_children =
                self.normalize_flow_children(world, owner, &cell_style, cell_children)?;
            return self.replace_children_and_mark_context(world, cell, cell_children);
        }

        Err(LayoutError::source_contract(
            self.source.label(owner),
            format!(
                "box kind {} cannot own structural table children",
                parent_role.debug_name()
            ),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn ensure_anonymous_wrapper(
        &mut self,
        world: &mut LayoutWorld<S::NodeId>,
        owner: S::NodeId,
        parent_style: &ResolvedLayoutStyle,
        output: &mut Vec<LayoutBoxId>,
        kind: LayoutBoxKind,
        reason: LayoutAnonymousReason,
        display: LayoutDisplay,
    ) -> Result<LayoutBoxId, LayoutError> {
        if let Some(last) = output.last().copied()
            && world.box_by_id(last).is_some_and(|layout_box| {
                layout_box.kind == kind && layout_box.anonymous_reason == Some(reason)
            })
        {
            return Ok(last);
        }

        let style = self.styles.anonymous_style(owner, parent_style, display)?;
        let id = world.allocate(LayoutWorld::new_box(
            None,
            Some(owner),
            None,
            format!("{}({})", kind.debug_name(), self.source.label(owner)),
            Some(self.source.label(owner)),
            None,
            Some(reason),
            kind,
            style,
            None,
            None,
        ));
        output.push(id);
        Ok(id)
    }

    fn replace_children_and_mark_context(
        &self,
        world: &mut LayoutWorld<S::NodeId>,
        parent: LayoutBoxId,
        children: Vec<LayoutBoxId>,
    ) -> Result<(), LayoutError> {
        let establishes_inline_context = children
            .iter()
            .copied()
            .any(|child| self.is_meaningful_inline_in_flow(world, child))
            && !children
                .iter()
                .copied()
                .any(|child| self.is_block_in_flow(world, child));
        world.replace_children(parent, children)?;
        world
            .box_by_id_mut(parent)
            .ok_or(LayoutError::InvalidBoxReference {
                index: parent.index(),
            })?
            .inline_formatting_context = establishes_inline_context;
        Ok(())
    }

    fn table_role(
        &self,
        world: &LayoutWorld<S::NodeId>,
        id: LayoutBoxId,
    ) -> Result<Option<TableBoxRole>, LayoutError> {
        let layout_box = world
            .box_by_id(id)
            .ok_or(LayoutError::InvalidBoxReference { index: id.index() })?;
        Ok(match layout_box.style.display() {
            LayoutDisplay::Table | LayoutDisplay::InlineTable => Some(TableBoxRole::Root),
            LayoutDisplay::TableCaption => Some(TableBoxRole::Caption),
            LayoutDisplay::TableRowGroup
            | LayoutDisplay::TableHeaderGroup
            | LayoutDisplay::TableFooterGroup => Some(TableBoxRole::RowGroup),
            LayoutDisplay::TableColumnGroup => Some(TableBoxRole::ColumnGroup),
            LayoutDisplay::TableColumn => Some(TableBoxRole::Column),
            LayoutDisplay::TableRow => Some(TableBoxRole::Row),
            LayoutDisplay::TableCell => Some(TableBoxRole::Cell),
            _ => None,
        })
    }

    fn is_inline_in_flow(&self, world: &LayoutWorld<S::NodeId>, id: LayoutBoxId) -> bool {
        world.box_by_id(id).is_some_and(|layout_box| {
            !layout_box.style.is_out_of_flow()
                && (layout_box.kind.is_text() || layout_box.style.display().is_inline_level())
        })
    }

    fn is_meaningful_inline_in_flow(
        &self,
        world: &LayoutWorld<S::NodeId>,
        id: LayoutBoxId,
    ) -> bool {
        self.is_inline_in_flow(world, id) && !self.is_ignorable_whitespace_text(world, id)
    }

    fn is_block_in_flow(&self, world: &LayoutWorld<S::NodeId>, id: LayoutBoxId) -> bool {
        world.box_by_id(id).is_some_and(|layout_box| {
            !layout_box.style.is_out_of_flow()
                && !layout_box.kind.is_text()
                && !layout_box.style.display().is_inline_level()
        })
    }

    fn is_ignorable_whitespace_text(
        &self,
        world: &LayoutWorld<S::NodeId>,
        id: LayoutBoxId,
    ) -> bool {
        world.box_by_id(id).is_some_and(|layout_box| {
            layout_box.kind.is_text()
                && !layout_box
                    .capability_diagnostics
                    .contains(&LayoutCapabilityDiagnostic::GeneratedContentUnsupported)
                && layout_box.text.as_deref().is_none_or(|text| {
                    whitespace_text_is_ignorable(text, layout_box.style.white_space_collapse())
                })
        })
    }

    fn checked_flat_children(&mut self, parent: S::NodeId) -> Result<Vec<S::NodeId>, LayoutError> {
        let parent_label = self.source.label(parent);
        let children = self.source.flat_children(parent).collect::<Vec<_>>();
        for child in &children {
            let child_label = self.source.label(*child);
            if self.active_sources.contains(child) {
                return Err(LayoutError::SourceCycle {
                    source_label: child_label,
                });
            }
            let actual_parent = self.source.flat_parent(*child);
            if actual_parent != Some(parent) {
                let actual = actual_parent
                    .map(|node| self.source.label(node))
                    .unwrap_or_else(|| "<none>".to_owned());
                return Err(LayoutError::source_contract(
                    child_label,
                    format!(
                        "flat_children({parent_label}) disagrees with flat_parent; got {actual}"
                    ),
                ));
            }
            if let Some(first_parent) = self.seen_sources.get(child) {
                return Err(LayoutError::source_contract(
                    child_label,
                    format!(
                        "flat-tree node appears more than once; first parent was {first_parent}, second parent is {parent_label}"
                    ),
                ));
            }
            self.seen_sources.insert(*child, parent_label.clone());
        }
        Ok(children)
    }

    fn validated_element_semantics(
        &self,
        node: S::NodeId,
        kind: LayoutSourceKind,
    ) -> Result<LayoutElementSemantics, LayoutError> {
        let source = self.source.label(node);
        if kind != LayoutSourceKind::Element {
            return Err(LayoutError::source_contract(
                source,
                format!("element semantics requested for {kind:?} source"),
            ));
        }
        let Some(semantics) = self.source.element_semantics(node) else {
            return Err(LayoutError::source_contract(
                source,
                "element source has no element semantics",
            ));
        };
        if semantics.local_name.is_empty() {
            return Err(LayoutError::source_contract(
                source,
                "element local name must not be empty",
            ));
        }
        if semantics.replaced.is_none() && self.source.replaced_metrics(node).is_some() {
            return Err(LayoutError::source_contract(
                source,
                "non-replaced element exposed replaced metrics",
            ));
        }
        Ok(semantics)
    }

    fn validate_non_element_semantics(
        &self,
        node: S::NodeId,
        kind: LayoutSourceKind,
    ) -> Result<(), LayoutError> {
        if self.source.element_semantics(node).is_some() {
            return Err(LayoutError::source_contract(
                self.source.label(node),
                format!("{kind:?} source exposed element semantics"),
            ));
        }
        if self.source.replaced_metrics(node).is_some() {
            return Err(LayoutError::source_contract(
                self.source.label(node),
                format!("{kind:?} source exposed replaced metrics"),
            ));
        }
        Ok(())
    }
}

fn natural_replaced_ratio(
    semantics: &LayoutElementSemantics,
    metrics: Option<crate::ReplacedMetrics>,
) -> Option<f32> {
    let sizing_kind = if matches!(
        semantics.category,
        crate::LayoutElementCategory::FormControl(crate::LayoutFormControlKind::Input(
            crate::LayoutInputControlKind::Image
        ))
    ) {
        crate::LayoutReplacedKind::Image
    } else {
        semantics.replaced?
    };
    ReplacedContext::for_element(sizing_kind, metrics).inherent_ratio()
}

fn whitespace_text_is_ignorable(text: &str, mode: InlineWhiteSpaceCollapse) -> bool {
    if text.is_empty() {
        return true;
    }
    if !text
        .chars()
        .all(|character| matches!(character, ' ' | '\t' | '\n' | '\r' | '\u{000C}'))
    {
        return false;
    }
    match mode {
        InlineWhiteSpaceCollapse::Collapse => true,
        InlineWhiteSpaceCollapse::PreserveBreaks => !text
            .chars()
            .any(|character| matches!(character, '\n' | '\r' | '\u{000C}')),
        InlineWhiteSpaceCollapse::Preserve | InlineWhiteSpaceCollapse::BreakSpaces => false,
    }
}

fn principal_kind(
    semantics: &LayoutElementSemantics,
    style: &ResolvedLayoutStyle,
) -> LayoutBoxKind {
    match semantics.category {
        LayoutElementCategory::LineBreak => return LayoutBoxKind::LineBreak,
        LayoutElementCategory::FormControl(_) => return LayoutBoxKind::FormControl,
        LayoutElementCategory::Generic
        | LayoutElementCategory::Table(_)
        | LayoutElementCategory::List(_) => {}
    }
    if semantics.is_replaced() {
        return LayoutBoxKind::Replaced;
    }
    match style.display() {
        LayoutDisplay::None | LayoutDisplay::Contents | LayoutDisplay::Block => {
            LayoutBoxKind::PrincipalBlock
        }
        LayoutDisplay::FlowRoot => LayoutBoxKind::PrincipalFlowRoot,
        LayoutDisplay::Inline => LayoutBoxKind::PrincipalInline,
        LayoutDisplay::InlineBlock => LayoutBoxKind::PrincipalInlineBlock,
        LayoutDisplay::Flex => LayoutBoxKind::PrincipalFlex,
        LayoutDisplay::InlineFlex => LayoutBoxKind::PrincipalInlineFlex,
        LayoutDisplay::Grid => LayoutBoxKind::PrincipalGrid,
        LayoutDisplay::InlineGrid => LayoutBoxKind::PrincipalInlineGrid,
        LayoutDisplay::BlockListItem => LayoutBoxKind::ListItem,
        LayoutDisplay::InlineListItem => LayoutBoxKind::InlineListItem,
        LayoutDisplay::Table => LayoutBoxKind::TableWrapper,
        LayoutDisplay::InlineTable => LayoutBoxKind::InlineTableWrapper,
        LayoutDisplay::TableCaption => LayoutBoxKind::TableCaption,
        LayoutDisplay::TableRowGroup => LayoutBoxKind::TableRowGroup,
        LayoutDisplay::TableHeaderGroup => LayoutBoxKind::TableHeaderGroup,
        LayoutDisplay::TableFooterGroup => LayoutBoxKind::TableFooterGroup,
        LayoutDisplay::TableColumnGroup => LayoutBoxKind::TableColumnGroup,
        LayoutDisplay::TableColumn => LayoutBoxKind::TableColumn,
        LayoutDisplay::TableRow => LayoutBoxKind::TableRow,
        LayoutDisplay::TableCell => LayoutBoxKind::TableCell,
    }
}

fn is_leaf_element(
    semantics: &LayoutElementSemantics,
    kind: LayoutBoxKind,
    style: &ResolvedLayoutStyle,
) -> bool {
    semantics.is_replaced()
        || semantics.category == LayoutElementCategory::LineBreak
        || kind == LayoutBoxKind::TableColumn
        || style.display() == LayoutDisplay::TableColumn
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TableBoxRole {
    Root,
    Caption,
    RowGroup,
    ColumnGroup,
    Column,
    Row,
    Cell,
}

impl TableBoxRole {
    const fn requires_parent(self) -> bool {
        !matches!(self, Self::Root)
    }

    const fn is_direct_table_child(self) -> bool {
        matches!(
            self,
            Self::Caption | Self::RowGroup | Self::ColumnGroup | Self::Column
        )
    }

    const fn debug_name(self) -> &'static str {
        match self {
            Self::Root => "table-root",
            Self::Caption => "table-caption",
            Self::RowGroup => "table-row-group",
            Self::ColumnGroup => "table-column-group",
            Self::Column => "table-column",
            Self::Row => "table-row",
            Self::Cell => "table-cell",
        }
    }
}

fn push_diagnostic(
    diagnostics: &mut Vec<LayoutCapabilityDiagnostic>,
    diagnostic: LayoutCapabilityDiagnostic,
) {
    if !diagnostics.contains(&diagnostic) {
        diagnostics.push(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use super::{InlineWhiteSpaceCollapse, whitespace_text_is_ignorable};

    #[test]
    fn only_collapsible_whitespace_is_ignorable_during_box_construction() {
        assert!(whitespace_text_is_ignorable(
            " \t\n",
            InlineWhiteSpaceCollapse::Collapse,
        ));
        assert!(whitespace_text_is_ignorable(
            " \t",
            InlineWhiteSpaceCollapse::PreserveBreaks,
        ));
        assert!(!whitespace_text_is_ignorable(
            "\n",
            InlineWhiteSpaceCollapse::PreserveBreaks,
        ));
        assert!(!whitespace_text_is_ignorable(
            "\n",
            InlineWhiteSpaceCollapse::Preserve,
        ));
        assert!(!whitespace_text_is_ignorable(
            " ",
            InlineWhiteSpaceCollapse::BreakSpaces,
        ));
        assert!(!whitespace_text_is_ignorable(
            "\u{00a0}",
            InlineWhiteSpaceCollapse::Collapse,
        ));
    }
}
