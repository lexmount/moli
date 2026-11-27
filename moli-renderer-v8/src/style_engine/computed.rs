mod marker;
mod resolution_pass;

use std::collections::HashSet;

use dom::ElementState as StyloElementState;
use style::{
    computed_value_flags::ComputedValueFlags,
    context::QuirksMode,
    data::{ElementData, ElementStyles},
    dom::{TElement, TNode},
    invalidation::element::restyle_hints::RestyleHint,
    properties::{
        ComputedValues, Importance, PropertyDeclarationBlock, PropertyId, ShorthandId,
        longhands::{
            text_wrap_mode::computed_value::T as StyloTextWrapMode,
            visibility::computed_value::T as ComputedVisibility,
            white_space_collapse::computed_value::T as StyloWhiteSpaceCollapse,
        },
        parse_style_attribute,
    },
    selector_parser::PseudoElement,
    servo::restyle_damage::ServoRestyleDamage,
    servo_arc::Arc as ServoArc,
    stylesheets::{CssRuleType, UrlExtraData},
    values::{
        AtomIdent, resolved,
        specified::{
            box_::{ContentVisibility, Display, DisplayInside, DisplayOutside},
            text::TextTransformCase,
        },
    },
};

pub(crate) use marker::{MarkerStyleDecision, marker_style_decision};
pub(crate) use resolution_pass::StyleResolutionPass;
pub(super) use resolution_pass::with_resolution_context_and_snapshots;
pub(super) use resolution_pass::with_style_resolution_pass;
use resolution_pass::{with_resolution_context, with_style_resolution_pass_in_world};

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Node},
};

use moli_selector::InlineStyleSnapshot;

use super::{
    FullStyleWorldSnapshot, MoliStyleEngine, PreparedStyleWorldUpdate, StyleViewport,
    StyleWorldUpdate, StyleWorldUpdatePlan,
    cache::ComputedElementStyleCacheKey,
    document_world::DocumentStyleWorld,
    source_document::StyleSourceDocumentContext,
    world_key::StyleWorldKey,
    world_lifecycle::{
        ensure_retained_style_system, ensure_retained_style_system_incrementally,
        retained_style_world_update_plan,
    },
};

#[cfg(test)]
thread_local! {
    static STYLE_RESOLUTION_CONTEXTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn take_style_resolution_context_count_for_test() -> usize {
    STYLE_RESOLUTION_CONTEXTS.replace(0)
}

#[derive(Clone)]
pub(crate) struct StyloComputedStyleSnapshot {
    primary: ServoArc<ComputedValues>,
    before: Option<ServoArc<ComputedValues>>,
    after: Option<ServoArc<ComputedValues>>,
}

pub(crate) enum StyleObservationSnapshot {
    Current(Option<StyloComputedStyleSnapshot>),
    NeedsStyleWorldUpdate(StyleWorldUpdatePlan),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComputedDisplayKind {
    None,
    Contents,
    Inline,
    InlineAtomic,
    Block,
    Table,
    InlineTable,
    TableInternal,
    TableRow,
    TableCell,
    ListItem,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComputedTextTransformKind {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComputedWhiteSpaceCollapseKind {
    Collapse,
    Preserve,
    PreserveBreaks,
    BreakSpaces,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComputedTextWrapModeKind {
    Wrap,
    NoWrap,
    Other,
}

/// Servo precomputed pseudo used to derive an anonymous box style.
///
/// This is an owner-local style operation inside one synchronous observation
/// scope. It is intentionally not cached and has no generation/fence model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StyloAnonymousBoxKind {
    Generic,
    Table,
    TableRow,
    TableCell,
}

impl StyloAnonymousBoxKind {
    const fn pseudo(self) -> PseudoElement {
        match self {
            Self::Generic => PseudoElement::ServoAnonymousBox,
            Self::Table => PseudoElement::ServoAnonymousTable,
            Self::TableRow => PseudoElement::ServoAnonymousTableRow,
            Self::TableCell => PseudoElement::ServoAnonymousTableCell,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ComputedRenderedStyleFacts {
    pub(crate) display: ComputedDisplayKind,
    pub(crate) hides_contents: bool,
    pub(crate) visibility_visible: bool,
    pub(crate) opacity_zero: bool,
    pub(crate) text_transform: ComputedTextTransformKind,
    pub(crate) white_space_collapse: ComputedWhiteSpaceCollapseKind,
    pub(crate) text_wrap_mode: ComputedTextWrapModeKind,
}

fn layout_damage_for_style_change(
    old: Option<&ServoArc<ComputedValues>>,
    new: Option<&ServoArc<ComputedValues>>,
) -> ServoRestyleDamage {
    match (old, new) {
        (Some(old), Some(new))
            if old.effective_zoom != new.effective_zoom || old.writing_mode != new.writing_mode =>
        {
            ServoRestyleDamage::RELAYOUT
        }
        (Some(_), None) | (None, Some(_)) => ServoRestyleDamage::reconstruct(),
        _ => ServoRestyleDamage::empty(),
    }
}

impl StyloComputedStyleSnapshot {
    fn from_primary(primary: ServoArc<ComputedValues>) -> Self {
        Self {
            primary,
            before: None,
            after: None,
        }
    }

    pub(super) fn from_element_styles(styles: &ElementStyles) -> Self {
        Self {
            primary: styles.primary().clone(),
            before: styles.pseudos.get(&PseudoElement::Before).cloned(),
            after: styles.pseudos.get(&PseudoElement::After).cloned(),
        }
    }

    pub(crate) fn computed_values(&self) -> ServoArc<ComputedValues> {
        self.primary.clone()
    }

    pub(crate) fn into_element_computed_values(
        self,
    ) -> (
        ServoArc<ComputedValues>,
        Option<ServoArc<ComputedValues>>,
        Option<ServoArc<ComputedValues>>,
    ) {
        (self.primary, self.before, self.after)
    }

    pub(crate) fn hides_contents(&self) -> bool {
        content_visibility_applies(self.primary.clone_display())
            && self.primary.clone_content_visibility() == ContentVisibility::Hidden
    }

    pub(crate) fn rendered_style_facts(&self) -> ComputedRenderedStyleFacts {
        let display = self.primary.clone_display();
        let display = if display.is_none() {
            ComputedDisplayKind::None
        } else if display.is_contents() {
            ComputedDisplayKind::Contents
        } else if display.is_list_item() {
            ComputedDisplayKind::ListItem
        } else if display.inside() == DisplayInside::TableRow {
            ComputedDisplayKind::TableRow
        } else if display.inside() == DisplayInside::TableCell {
            ComputedDisplayKind::TableCell
        } else if matches!(
            display.inside(),
            DisplayInside::TableRowGroup
                | DisplayInside::TableColumn
                | DisplayInside::TableColumnGroup
                | DisplayInside::TableHeaderGroup
                | DisplayInside::TableFooterGroup
        ) {
            ComputedDisplayKind::TableInternal
        } else if display.inside() == DisplayInside::Table {
            if display.outside() == DisplayOutside::Inline {
                ComputedDisplayKind::InlineTable
            } else {
                ComputedDisplayKind::Table
            }
        } else if display.outside() == DisplayOutside::Inline {
            if display.is_inline_flow() {
                ComputedDisplayKind::Inline
            } else {
                ComputedDisplayKind::InlineAtomic
            }
        } else if matches!(
            display.outside(),
            DisplayOutside::Block | DisplayOutside::TableCaption
        ) {
            ComputedDisplayKind::Block
        } else {
            ComputedDisplayKind::Other
        };
        let text_transform = match self.primary.clone_text_transform().case() {
            TextTransformCase::None => ComputedTextTransformKind::None,
            TextTransformCase::Uppercase => ComputedTextTransformKind::Uppercase,
            TextTransformCase::Lowercase => ComputedTextTransformKind::Lowercase,
            TextTransformCase::Capitalize => ComputedTextTransformKind::Capitalize,
            #[allow(unreachable_patterns)]
            _ => ComputedTextTransformKind::Other,
        };
        let white_space_collapse = match self.primary.clone_white_space_collapse() {
            StyloWhiteSpaceCollapse::Collapse => ComputedWhiteSpaceCollapseKind::Collapse,
            StyloWhiteSpaceCollapse::Preserve => ComputedWhiteSpaceCollapseKind::Preserve,
            StyloWhiteSpaceCollapse::PreserveBreaks => {
                ComputedWhiteSpaceCollapseKind::PreserveBreaks
            }
            StyloWhiteSpaceCollapse::BreakSpaces => ComputedWhiteSpaceCollapseKind::BreakSpaces,
            #[allow(unreachable_patterns)]
            _ => ComputedWhiteSpaceCollapseKind::Other,
        };
        let text_wrap_mode = match self.primary.clone_text_wrap_mode() {
            StyloTextWrapMode::Wrap => ComputedTextWrapModeKind::Wrap,
            StyloTextWrapMode::Nowrap => ComputedTextWrapModeKind::NoWrap,
            #[allow(unreachable_patterns)]
            _ => ComputedTextWrapModeKind::Other,
        };
        ComputedRenderedStyleFacts {
            display,
            hides_contents: self.hides_contents(),
            visibility_visible: self.primary.clone_visibility() == ComputedVisibility::Visible,
            opacity_zero: self.primary.clone_opacity() == 0.0,
            text_transform,
            white_space_collapse,
            text_wrap_mode,
        }
    }

    pub(crate) fn accessibility_style(&self) -> moli_dom::accessibility::AccessibilityStyle {
        let display = self.primary.clone_display();
        moli_dom::accessibility::AccessibilityStyle {
            display_none: display.is_none(),
            visibility_visible: self.primary.clone_visibility() == ComputedVisibility::Visible,
            hides_contents: self.hides_contents(),
            block_level: matches!(
                display.outside(),
                DisplayOutside::Block | DisplayOutside::TableCaption
            ),
        }
    }

    pub(crate) fn property_value(&self, property: &str) -> Option<String> {
        if property.starts_with("--") {
            return serialize_computed_custom_property(&self.primary, property);
        }
        let property_id = PropertyId::parse_enabled_for_all_content(property).ok()?;
        serialize_raw_computed_property(&self.primary, property_id)
    }

    pub(crate) fn original_display(&self) -> String {
        style_traits::ToCss::to_css_string(&self.primary.get_box().original_display)
    }

    pub(crate) fn has_own_visibility_value(&self) -> bool {
        self.primary
            .flags
            .contains(ComputedValueFlags::HAS_OWN_VISIBILITY)
    }

    pub(crate) fn resolved_property_value(&self, property: &str) -> Option<String> {
        if property.starts_with("--") {
            return serialize_computed_custom_property(&self.primary, property);
        }
        let property_id = PropertyId::parse_enabled_for_all_content(property).ok()?;
        serialize_resolved_computed_property(&self.primary, property_id)
    }

    pub(crate) fn custom_property_names(&self) -> Vec<String> {
        computed_custom_property_names_for_style(&self.primary)
    }
}

fn content_visibility_applies(display: Display) -> bool {
    !display.is_none()
        && !display.is_contents()
        && !display.is_inline_flow()
        && display.outside() != DisplayOutside::TableCaption
        && !matches!(
            display.inside(),
            DisplayInside::Table
                | DisplayInside::TableRowGroup
                | DisplayInside::TableColumn
                | DisplayInside::TableColumnGroup
                | DisplayInside::TableHeaderGroup
                | DisplayInside::TableFooterGroup
                | DisplayInside::TableRow
        )
}

pub(super) fn retained_current_element_state(
    engine: &MoliStyleEngine,
    host: &DomHost,
    element: DomHandle,
) -> Option<StyloElementState> {
    let world = engine.owner_document_world(host, element)?;
    world.document_state.try_with_retained_style_system(|_| {
        engine.dom_adapter.with_bound_host(host, |adapter| {
            adapter.computed_element_state(host, element)
        })
    })?
}

pub(super) fn computed_style_property_value(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    property: &str,
    pseudo_element: Option<&str>,
    inputs: &FullStyleWorldSnapshot,
    document_context: StyleSourceDocumentContext<'_>,
    read_document: DomHandle,
    viewport: StyleViewport,
) -> Option<String> {
    let owner_document = owner_document_for_computed_style_read(host, handle)?;
    trace_computed_style_read(
        document_url,
        handle,
        owner_document,
        read_document,
        property,
        pseudo_element,
        document_context,
    );
    engine.drain_pending_style_invalidations_for_computed_style_read_with_document_context(
        host,
        owner_document,
        document_context,
    );
    let pseudo_element = match pseudo_element {
        Some(pseudo_element) => Some(stylo_pseudo_element_for_computed_style(pseudo_element)?),
        None => None,
    };
    let style_system_key = StyleWorldKey::new_for_observation(
        inputs,
        viewport,
        super::StyleTreeScopeVersions::current(host, Some(owner_document)),
    );
    if property.starts_with("--") {
        if let Some(ref pseudo_element) = pseudo_element
            && pseudo_element.is_lazy()
        {
            return with_lazily_resolved_pseudo_style(
                engine,
                host,
                document_url,
                handle,
                &style_system_key,
                inputs,
                pseudo_element,
                |style| serialize_computed_custom_property(style, property),
            );
        }
        return with_resolved_style(
            engine,
            host,
            document_url,
            handle,
            &style_system_key,
            inputs,
            pseudo_element.as_ref(),
            |style, _styles| serialize_computed_custom_property(style, property),
        );
    }
    let property_id = PropertyId::parse_enabled_for_all_content(property).ok()?;
    if let Some(ref pseudo_element) = pseudo_element
        && pseudo_element.is_lazy()
    {
        return with_lazily_resolved_pseudo_style(
            engine,
            host,
            document_url,
            handle,
            &style_system_key,
            inputs,
            pseudo_element,
            |style| serialize_resolved_computed_property(style, property_id.clone()),
        );
    }
    with_resolved_style(
        engine,
        host,
        document_url,
        handle,
        &style_system_key,
        inputs,
        pseudo_element.as_ref(),
        |style, _styles| serialize_resolved_computed_property(style, property_id),
    )
}

pub(super) fn computed_style_snapshot_after_style_update(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    inputs: &FullStyleWorldSnapshot,
    document_context: StyleSourceDocumentContext<'_>,
    read_document: DomHandle,
    viewport: StyleViewport,
) -> Option<StyloComputedStyleSnapshot> {
    let owner_document = owner_document_for_computed_style_read(host, handle)?;
    let style_system_key = StyleWorldKey::new_for_observation(
        inputs,
        viewport,
        super::StyleTreeScopeVersions::current(host, Some(owner_document)),
    );
    computed_style_snapshot_after_style_update_with_key(
        engine,
        host,
        document_url,
        handle,
        &style_system_key,
        inputs,
        document_context,
        read_document,
    )
}

pub(super) fn computed_style_snapshot_from_current_observation(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    environment: super::StyloStyleEnvironment,
    document_context: StyleSourceDocumentContext<'_>,
    read_document: DomHandle,
    viewport: StyleViewport,
    tree_scope_versions: super::StyleTreeScopeVersions,
) -> StyleObservationSnapshot {
    let Some(owner_document) = owner_document_for_computed_style_read(host, handle) else {
        return StyleObservationSnapshot::Current(None);
    };
    trace_computed_style_read(
        document_url,
        handle,
        owner_document,
        read_document,
        "<computed-style-observation>",
        None,
        document_context,
    );
    let quirks_mode = document_quirks_mode(host, owner_document);
    let world = engine.world_for_document(owner_document);
    if let Some(plan) = style_world_update_plan_for_observation(
        host,
        &world,
        viewport,
        environment,
        quirks_mode,
        tree_scope_versions,
    ) {
        return StyleObservationSnapshot::NeedsStyleWorldUpdate(plan);
    }
    StyleObservationSnapshot::Current(with_resolved_style_in_current_world(
        engine,
        host,
        document_url,
        handle,
        &world,
        quirks_mode,
        None,
        |_style, styles| styles.map(StyloComputedStyleSnapshot::from_element_styles),
    ))
}

pub(super) fn document_quirks_mode(host: &DomHost, document: DomHandle) -> QuirksMode {
    host.node(document)
        .and_then(Node::as_document)
        .map_or(QuirksMode::NoQuirks, |document| document.quirks_mode())
}

pub(super) fn style_world_update_plan_for_observation(
    host: &DomHost,
    world: &DocumentStyleWorld,
    viewport: StyleViewport,
    environment: super::StyloStyleEnvironment,
    quirks_mode: QuirksMode,
    tree_scope_versions: super::StyleTreeScopeVersions,
) -> Option<StyleWorldUpdatePlan> {
    (!world
        .document_state
        .retained_style_system_is_current_for_observation(
            viewport,
            environment,
            quirks_mode,
            tree_scope_versions,
        ))
    .then(|| {
        retained_style_world_update_plan(
            host,
            &world.document_state,
            world.document,
            quirks_mode,
            tree_scope_versions,
        )
    })
}

pub(super) fn computed_style_snapshot_after_world_update(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    update: &PreparedStyleWorldUpdate,
    document_context: StyleSourceDocumentContext<'_>,
    read_document: DomHandle,
) -> Option<StyloComputedStyleSnapshot> {
    let owner_document = owner_document_for_computed_style_read(host, handle)?;
    let trace = match update.update() {
        StyleWorldUpdate::Full(_) => "<computed-style-snapshot>",
        StyleWorldUpdate::Incremental(_) => "<incremental-computed-style-snapshot>",
    };
    trace_computed_style_read(
        document_url,
        handle,
        owner_document,
        read_document,
        trace,
        None,
        document_context,
    );
    let world = engine.world_for_document(owner_document);
    apply_style_world_update(engine, host, &world, update);
    with_resolved_style_in_current_world(
        engine,
        host,
        document_url,
        handle,
        &world,
        update.environment().quirks_mode,
        None,
        |_style, styles| styles.map(StyloComputedStyleSnapshot::from_element_styles),
    )
}

pub(super) fn apply_style_world_update(
    engine: &MoliStyleEngine,
    host: &DomHost,
    world: &DocumentStyleWorld,
    update: &PreparedStyleWorldUpdate,
) {
    match update.update() {
        StyleWorldUpdate::Full(inputs) => {
            let environment = update.environment();
            let style_system_key = StyleWorldKey::new_for_observation(
                inputs,
                environment.viewport,
                environment.tree_scope_versions,
            );
            ensure_retained_style_system_for_computed_read(
                engine,
                host,
                world,
                world.document,
                &style_system_key,
                inputs,
            );
        }
        StyleWorldUpdate::Incremental(incremental) => {
            ensure_retained_style_system_incrementally(
                host,
                &engine.dom_adapter,
                &world.document_state,
                world.document,
                engine.invalidation_cleanup_for_world(world),
                update.environment(),
                incremental,
            );
        }
    }
}

pub(super) fn computed_pseudo_style_snapshot_from_current_observation(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    pseudo_element: &str,
    document_context: StyleSourceDocumentContext<'_>,
    read_document: DomHandle,
) -> Option<StyloComputedStyleSnapshot> {
    let owner_document = owner_document_for_computed_style_read(host, handle)?;
    trace_computed_style_read(
        document_url,
        handle,
        owner_document,
        read_document,
        "<current-computed-pseudo-style-snapshot>",
        Some(pseudo_element),
        document_context,
    );
    let pseudo_element = stylo_pseudo_element_for_computed_style(pseudo_element)?;
    let world = engine.world_for_document(owner_document);
    let quirks_mode = host
        .node(owner_document)
        .and_then(Node::as_document)
        .map(|document| document.quirks_mode())
        .unwrap_or(QuirksMode::NoQuirks);
    if pseudo_element.is_lazy() {
        return with_lazily_resolved_pseudo_style_in_current_world(
            engine,
            host,
            document_url,
            handle,
            &world,
            quirks_mode,
            &pseudo_element,
            |style| Some(StyloComputedStyleSnapshot::from_primary(style.clone())),
        );
    }
    with_resolved_style_in_current_world(
        engine,
        host,
        document_url,
        handle,
        &world,
        quirks_mode,
        Some(&pseudo_element),
        |style, _styles| Some(StyloComputedStyleSnapshot::from_primary(style.clone())),
    )
}

fn computed_style_snapshot_after_style_update_with_key(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    style_system_key: &StyleWorldKey,
    inputs: &FullStyleWorldSnapshot,
    document_context: StyleSourceDocumentContext<'_>,
    read_document: DomHandle,
) -> Option<StyloComputedStyleSnapshot> {
    let owner_document = owner_document_for_computed_style_read(host, handle)?;
    trace_computed_style_read(
        document_url,
        handle,
        owner_document,
        read_document,
        "<computed-style-snapshot>",
        None,
        document_context,
    );
    with_resolved_style(
        engine,
        host,
        document_url,
        handle,
        style_system_key,
        inputs,
        None,
        |_style, styles| styles.map(StyloComputedStyleSnapshot::from_element_styles),
    )
}

fn computed_custom_property_names_for_style(style: &ComputedValues) -> Vec<String> {
    let custom_properties = style.custom_properties();
    let property_count = custom_properties.inherited.len() + custom_properties.non_inherited.len();
    let mut seen = HashSet::with_capacity(property_count);
    let mut names = Vec::with_capacity(property_count);
    for (name, _) in custom_properties
        .inherited
        .iter()
        .chain(custom_properties.non_inherited.iter())
    {
        // Stylo's maps are copy-on-write chains. Walk each chain once, but
        // publish a name only when its effective entry is live; an older
        // ancestor entry may be shadowed by a tombstone in the child map.
        if seen.insert(name)
            && custom_properties
                .inherited
                .get(name)
                .or_else(|| custom_properties.non_inherited.get(name))
                .is_some()
        {
            names.push(format!("--{name}"));
        }
    }
    names
}

fn with_resolved_style<R>(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    style_system_key: &StyleWorldKey,
    inputs: &FullStyleWorldSnapshot,
    pseudo_element: Option<&PseudoElement>,
    callback: impl FnOnce(&ServoArc<ComputedValues>, Option<&ElementStyles>) -> Option<R>,
) -> Option<R> {
    let owner_document = owner_document_for_computed_style_read(host, handle)?;
    let world = engine.world_for_document(owner_document);
    ensure_retained_style_system_for_computed_read(
        engine,
        host,
        &world,
        owner_document,
        style_system_key,
        inputs,
    );
    with_resolved_style_in_current_world(
        engine,
        host,
        document_url,
        handle,
        &world,
        inputs.quirks_mode,
        pseudo_element,
        callback,
    )
}

fn with_resolved_style_in_current_world<R>(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    world: &DocumentStyleWorld,
    quirks_mode: QuirksMode,
    pseudo_element: Option<&PseudoElement>,
    callback: impl FnOnce(&ServoArc<ComputedValues>, Option<&ElementStyles>) -> Option<R>,
) -> Option<R> {
    #[cfg(test)]
    let computed_cache_generation = world.document_state.computed_cache_generation();
    let path_generation = world.document_state.style_paths.generation();
    if host.owner_document_handle(handle) == Some(world.document)
        && let Some(styles) =
            engine
                .dom_adapter
                .clone_clean_element_styles(world.document, handle, path_generation)
    {
        if let Some(pseudo_element) = pseudo_element {
            if let Some(style) = styles.pseudos.get(pseudo_element) {
                return callback(style, None);
            }
            if let Some(style) = with_cached_pseudo_style(world, handle, pseudo_element, || None) {
                return callback(&style, None);
            }
        } else {
            #[cfg(test)]
            world
                .pseudo_style_cache
                .record_primary(computed_cache_generation, handle);
            return callback(styles.primary(), Some(&styles));
        }
    }

    with_style_resolution_pass_in_world(engine, host, document_url, world, quirks_mode, |pass| {
        let element = pass.prepare(handle)?;
        let canonical_styles = element.borrow_data()?.styles.clone();
        let style = if let Some(pseudo_element) = pseudo_element {
            canonical_styles
                .pseudos
                .get(pseudo_element)
                .cloned()
                .or_else(|| {
                    with_cached_pseudo_style(world, handle, pseudo_element, || {
                        pass.eager_pseudo_styles_for_read(element, pseudo_element)
                            .pseudos
                            .get(pseudo_element)
                            .cloned()
                    })
                })?
        } else {
            #[cfg(test)]
            world
                .pseudo_style_cache
                .record_primary(computed_cache_generation, handle);
            canonical_styles.primary().clone()
        };
        callback(&style, Some(&canonical_styles))
    })
}

fn child_restyle_scope(hint: RestyleHint) -> RestyleHint {
    let direct_children =
        RestyleHint::RECASCADE_SELF | RestyleHint::RECASCADE_SELF_IF_INHERIT_RESET_STYLE;
    // Other Stylo hints can reach descendants even when intervening
    // children retain the same computed values.
    if hint.difference(direct_children).is_empty() {
        hint
    } else {
        RestyleHint::restyle_subtree()
    }
}

fn record_screen_restyle<E: TElement>(
    world: &DocumentStyleWorld,
    element: E,
    previous: &ElementStyles,
    data: &ElementData,
    child_hint: RestyleHint,
) -> ServoRestyleDamage {
    let handle = DomHandle::new(element.as_node().debug_id());
    let old_primary = previous.get_primary();
    let new_primary = data.styles.get_primary();
    world
        .document_state
        .record_screen_style_damage(handle, old_primary.is_some(), || {
            if !element.as_node().is_in_document() {
                return Some(ServoRestyleDamage::empty());
            }
            if child_hint.will_recascade_subtree()
                || old_primary.zip(new_primary).is_some_and(|(old, new)| {
                    old.flags.maybe_inherited() != new.flags.maybe_inherited()
                })
            {
                return None;
            }
            let mut damage = data.damage | layout_damage_for_style_change(old_primary, new_primary);
            for (old, new) in previous
                .pseudos
                .as_array()
                .iter()
                .zip(data.styles.pseudos.as_array())
            {
                damage |= layout_damage_for_style_change(old.as_ref(), new.as_ref());
            }
            Some(damage)
        })
}

fn with_lazily_resolved_pseudo_style<R>(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    style_system_key: &StyleWorldKey,
    inputs: &FullStyleWorldSnapshot,
    pseudo_element: &PseudoElement,
    callback: impl FnOnce(&ServoArc<ComputedValues>) -> Option<R>,
) -> Option<R> {
    debug_assert!(pseudo_element.is_lazy());
    let owner_document = owner_document_for_computed_style_read(host, handle)?;
    let world = engine.world_for_document(owner_document);
    ensure_retained_style_system_for_computed_read(
        engine,
        host,
        &world,
        owner_document,
        style_system_key,
        inputs,
    );
    with_lazily_resolved_pseudo_style_in_current_world(
        engine,
        host,
        document_url,
        handle,
        &world,
        inputs.quirks_mode,
        pseudo_element,
        callback,
    )
}

fn with_lazily_resolved_pseudo_style_in_current_world<R>(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    world: &DocumentStyleWorld,
    quirks_mode: QuirksMode,
    pseudo_element: &PseudoElement,
    callback: impl FnOnce(&ServoArc<ComputedValues>) -> Option<R>,
) -> Option<R> {
    debug_assert!(pseudo_element.is_lazy());
    // Resolve/validate the primary path before consulting the pseudo sidecar.
    // A retained lazy invalidation root intentionally leaves old pseudo entries
    // in the cache until their owning element is demanded.
    let primary_style = with_resolved_style_in_current_world(
        engine,
        host,
        document_url,
        handle,
        world,
        quirks_mode,
        None,
        |style, _styles| Some(style.clone()),
    )?;

    let style = with_cached_pseudo_style(world, handle, pseudo_element, || {
        engine.dom_adapter.with_bound_host(host, |dom_adapter| {
            let element = dom_adapter.element(host, handle)?;
            with_resolution_context(world, dom_adapter, |mut context| {
                context.lazy_pseudo_style(element, pseudo_element, &primary_style)
            })
        })
    })?;
    callback(&style)
}

fn with_cached_pseudo_style(
    world: &DocumentStyleWorld,
    handle: DomHandle,
    pseudo: &PseudoElement,
    compute: impl FnOnce() -> Option<ServoArc<ComputedValues>>,
) -> Option<ServoArc<ComputedValues>> {
    let key = ComputedElementStyleCacheKey {
        computed_cache_generation: world.document_state.computed_cache_generation(),
        handle,
        pseudo_element: Some(pseudo.clone()),
    };
    if let Some(style) = world.pseudo_style_cache.get_pseudo(&key) {
        return Some(style);
    }
    let style = compute()?;
    world.pseudo_style_cache.insert_pseudo(key, style.clone());
    Some(style)
}

fn ensure_retained_style_system_for_computed_read(
    engine: &MoliStyleEngine,
    host: &DomHost,
    world: &DocumentStyleWorld,
    retained_document: DomHandle,
    key: &StyleWorldKey,
    inputs: &FullStyleWorldSnapshot,
) {
    ensure_retained_style_system(
        host,
        &engine.dom_adapter,
        &world.document_state,
        retained_document,
        engine.invalidation_cleanup_for_world(world),
        key,
        inputs,
        engine.author_styles_disabled(),
    );
}

fn owner_document_for_computed_style_read(host: &DomHost, handle: DomHandle) -> Option<DomHandle> {
    host.owner_document_handle(handle)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ComputedStyleReadTrace {
    pub(super) document_url: url::Url,
    pub(super) target: DomHandle,
    pub(super) owner_document: DomHandle,
    pub(super) read_document: DomHandle,
    pub(super) property: String,
    pub(super) pseudo_element: Option<String>,
    pub(super) document_context_documents: Vec<DomHandle>,
    pub(super) drain_documents: Vec<DomHandle>,
}

fn computed_style_read_trace(
    document_url: &url::Url,
    handle: DomHandle,
    owner_document: DomHandle,
    read_document: DomHandle,
    property: &str,
    pseudo_element: Option<&str>,
    document_context: StyleSourceDocumentContext<'_>,
) -> ComputedStyleReadTrace {
    ComputedStyleReadTrace {
        document_url: document_url.clone(),
        target: handle,
        owner_document,
        read_document,
        property: property.to_owned(),
        pseudo_element: pseudo_element.map(str::to_owned),
        document_context_documents: document_context.documents(),
        drain_documents: document_context.documents_with_owner(owner_document),
    }
}

fn trace_computed_style_read(
    document_url: &url::Url,
    handle: DomHandle,
    owner_document: DomHandle,
    read_document: DomHandle,
    property: &str,
    pseudo_element: Option<&str>,
    document_context: StyleSourceDocumentContext<'_>,
) {
    if !moli_trace::style_invalidation_trace_enabled() {
        return;
    }
    let trace = computed_style_read_trace(
        document_url,
        handle,
        owner_document,
        read_document,
        property,
        pseudo_element,
        document_context,
    );
    tracing::info!(
        document_url = %trace.document_url,
        target = ?trace.target,
        owner_document = ?trace.owner_document,
        read_document = ?trace.read_document,
        property = %trace.property,
        pseudo_element = ?trace.pseudo_element,
        document_context_documents = ?trace.document_context_documents,
        drain_documents = ?trace.drain_documents,
        "computed style read context"
    );
}

#[cfg(test)]
pub(super) fn computed_style_read_trace_for_test(
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    read_document: DomHandle,
    property: &str,
    pseudo_element: Option<&str>,
    document_context: StyleSourceDocumentContext<'_>,
) -> Option<ComputedStyleReadTrace> {
    Some(computed_style_read_trace(
        document_url,
        handle,
        owner_document_for_computed_style_read(host, handle)?,
        read_document,
        property,
        pseudo_element,
        document_context,
    ))
}

#[cfg(test)]
pub(super) fn ensure_retained_style_system_for_document_for_test(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document: DomHandle,
    key: StyleWorldKey,
    inputs: &FullStyleWorldSnapshot,
) {
    let world = engine.world_for_document(document);
    ensure_retained_style_system_for_computed_read(engine, host, &world, document, &key, inputs);
}

fn populate_inline_style_attribute_for_resolution(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    node: DomHandle,
    quirks_mode: QuirksMode,
) {
    let Some(world) = engine.owner_document_world(host, node) else {
        return;
    };
    let attribute = host.dom().find_attribute(node, "style");
    let cssom = attribute.and_then(moli_dom::native::Attribute::cssom_style);
    if !cssom.is_some_and(crate::css_style::CssomStyleAttribute::is_cssom_authored)
        && world.inline_style_metadata.csp_state(node)
            == super::InlineStyleCspState::BlockedAttribute
    {
        engine.dom_adapter.clear_inline_style_attribute(node);
        return;
    }
    let base_url = world
        .inline_style_metadata
        .base_url(node)
        .unwrap_or_else(|| style_resolution_base_url(host, document_url, node));
    if let Some(cssom) = cssom
        && cssom.resolution_text().is_none()
        && let Some((url_data, declarations)) =
            cssom.publish(world.document, &base_url, quirks_mode)
    {
        engine.dom_adapter.set_inline_style_attribute(
            node,
            InlineStyleSnapshot {
                document: world.document,
                source: None,
                url_data,
                quirks_mode,
                declarations,
            },
        );
        return;
    }
    let Some(source) = attribute.and_then(|attribute| attribute.shared_style_value()) else {
        engine.dom_adapter.clear_inline_style_attribute(node);
        return;
    };
    if engine.dom_adapter.inline_style_attribute_matches(
        world.document,
        node,
        &source,
        &base_url,
        quirks_mode,
    ) {
        return;
    }
    let url_data = UrlExtraData::from(base_url);
    let text = cssom
        .and_then(crate::css_style::CssomStyleAttribute::resolution_text)
        .unwrap_or_else(|| source.as_str());
    let declarations =
        parse_style_attribute(text, &url_data, None, quirks_mode, CssRuleType::Style);
    engine.dom_adapter.set_inline_style_attribute(
        node,
        InlineStyleSnapshot {
            document: world.document,
            source: Some(source),
            url_data,
            quirks_mode,
            declarations: ServoArc::new(engine.dom_adapter.shared_lock().wrap(declarations)),
        },
    );
}

fn style_resolution_base_url(
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
) -> url::Url {
    let document_handle = if host.node(handle).is_some_and(Node::is_document) {
        Some(handle)
    } else {
        host.node(handle).and_then(Node::owner_document)
    };
    document_handle
        .map(|document_handle| {
            if document_handle == host.document_handle() {
                host.document_base_url()
                    .unwrap_or_else(|| document_url.clone())
            } else {
                host.node(document_handle)
                    .and_then(Node::as_document)
                    .map(|document| document.base_url().clone())
                    .unwrap_or_else(|| document_url.clone())
            }
        })
        .unwrap_or_else(|| document_url.clone())
}

fn stylo_pseudo_element_for_computed_style(pseudo_element: &str) -> Option<PseudoElement> {
    match pseudo_element {
        "before" => Some(PseudoElement::Before),
        "after" => Some(PseudoElement::After),
        "backdrop" => Some(PseudoElement::Backdrop),
        "checkmark" => Some(PseudoElement::Checkmark),
        "first-letter" => Some(PseudoElement::FirstLetter),
        "selection" => Some(PseudoElement::Selection),
        "file-selector-button" => Some(PseudoElement::FileSelectorButton),
        "grammar-error" => Some(PseudoElement::GrammarError),
        "marker" => Some(PseudoElement::Marker),
        "picker(select)" => Some(PseudoElement::Picker),
        "picker-icon" => Some(PseudoElement::PickerIcon),
        "placeholder" => Some(PseudoElement::Placeholder),
        "spelling-error" => Some(PseudoElement::SpellingError),
        "view-transition" => Some(PseudoElement::ViewTransition),
        _ => {
            if let Some(name) = functional_pseudo_name(pseudo_element, "highlight") {
                return Some(PseudoElement::Highlight(AtomIdent::from(name)));
            }
            if let Some(name) = functional_pseudo_name(pseudo_element, "view-transition-group") {
                return Some(PseudoElement::ViewTransitionGroup(AtomIdent::from(name)));
            }
            if let Some(name) = functional_pseudo_name(pseudo_element, "view-transition-image-pair")
            {
                return Some(PseudoElement::ViewTransitionImagePair(AtomIdent::from(
                    name,
                )));
            }
            if let Some(name) = functional_pseudo_name(pseudo_element, "view-transition-old") {
                return Some(PseudoElement::ViewTransitionOld(AtomIdent::from(name)));
            }
            if let Some(name) = functional_pseudo_name(pseudo_element, "view-transition-new") {
                return Some(PseudoElement::ViewTransitionNew(AtomIdent::from(name)));
            }
            None
        }
    }
}

use moli_css_parse::functional_pseudo_name;

fn serialize_raw_computed_property(
    style: &ComputedValues,
    property_id: PropertyId,
) -> Option<String> {
    let mut serialized = String::new();
    style
        .computed_or_resolved_property_value(property_id, None, &mut serialized)
        .ok()?;
    Some(serialized)
}

/// Serializes an already sampled value with CSSOM's length/zoom normalization.
/// This does not initiate style observation or layout work.
pub(crate) fn serialize_resolved_computed_property(
    style: &ComputedValues,
    property_id: PropertyId,
) -> Option<String> {
    let mut context = resolved::Context {
        style,
        for_property: property_id.clone(),
        current_longhand: None,
    };
    let mut serialized = String::new();
    style
        .computed_or_resolved_property_value(property_id, Some(&mut context), &mut serialized)
        .ok()?;
    if serialized.is_empty()
        && let Ok(shorthand) = context.for_property.as_shorthand()
        && shorthand != ShorthandId::All
    {
        // Stylo's computed serializer handles some shorthands directly. The
        // declaration serializer can also compose the resolved longhand values
        // using the rules for the shorthand.
        let mut declarations = PropertyDeclarationBlock::new();
        for longhand in shorthand.longhands() {
            let declaration = style.computed_or_resolved_declaration(longhand, Some(&mut context));
            declarations.push(declaration, Importance::Normal);
        }
        declarations
            .shorthand_to_css(shorthand, &mut serialized)
            .ok()?;
    }
    Some(serialized)
}

fn serialize_computed_custom_property(style: &ComputedValues, property: &str) -> Option<String> {
    // `property_at(index)` is backed by an O(index) COW-chain walk. Parse the
    // custom name once and use the map's keyed lookup instead.
    let PropertyId::Custom(name) = PropertyId::parse_enabled_for_all_content(property).ok()? else {
        return None;
    };
    let custom_properties = style.custom_properties();
    custom_properties
        .inherited
        .get(&name)
        .or_else(|| custom_properties.non_inherited.get(&name))?;
    let property_id = PropertyId::Custom(name);
    let mut context = resolved::Context {
        style,
        for_property: property_id.clone(),
        current_longhand: None,
    };
    let mut serialized = String::new();
    style
        .computed_or_resolved_property_value(property_id, Some(&mut context), &mut serialized)
        .ok()?;
    if serialized.is_empty() {
        return Some(" ".to_owned());
    }
    Some(serialized)
}
