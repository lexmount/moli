use dom::ElementState as StyloElementState;
use style::{
    Atom,
    animation::DocumentAnimationSet,
    computed_value_flags::ComputedValueFlags,
    context::{
        QuirksMode, RegisteredSpeculativePainter, RegisteredSpeculativePainters,
        SharedStyleContext, StyleContext, StyleSystemOptions, ThreadLocalStyleContext,
    },
    data::ElementStyles,
    dom::TElement,
    properties::{
        ComputedValues, PropertyId,
        longhands::{
            text_wrap_mode::computed_value::T as StyloTextWrapMode,
            visibility::computed_value::T as ComputedVisibility,
            white_space_collapse::computed_value::T as StyloWhiteSpaceCollapse,
        },
        parse_style_attribute,
    },
    selector_parser::{PseudoElement, SnapshotMap},
    servo_arc::Arc as ServoArc,
    shared_lock::StylesheetGuards,
    stylesheets::{CssRuleType, UrlExtraData},
    stylist::RuleInclusion,
    thread_state::{self, ThreadState},
    traversal::resolve_style,
    traversal_flags::TraversalFlags,
    values::{
        AtomIdent, resolved,
        specified::{
            box_::{DisplayInside, DisplayOutside},
            text::TextTransformCase,
        },
    },
};

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Node},
};

use moli_selector::StyloElement;

use super::{
    MoliStyleEngine, StyleViewport, StyloComputedStyleInputs, StyloPreparedComputedStyleInputs,
    cache::ComputedElementStyleCacheKey,
    document_world::DocumentStyleWorld,
    source_lifecycle::StyleSourceDocumentContext,
    system::{StyleSystemCacheKey, ensure_retained_style_system},
};

#[derive(Clone)]
pub(crate) struct StyloComputedStyleSnapshot {
    primary: ServoArc<ComputedValues>,
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

    const fn trace_name(self) -> &'static str {
        match self {
            Self::Generic => "-servo-anonymous-box",
            Self::Table => "-servo-anonymous-table",
            Self::TableRow => "-servo-anonymous-table-row",
            Self::TableCell => "-servo-anonymous-table-cell",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ComputedRenderedStyleFacts {
    pub(crate) display: ComputedDisplayKind,
    pub(crate) content_visibility_applicable: bool,
    pub(crate) visibility_visible: bool,
    pub(crate) opacity_zero: bool,
    pub(crate) text_transform: ComputedTextTransformKind,
    pub(crate) white_space_collapse: ComputedWhiteSpaceCollapseKind,
    pub(crate) text_wrap_mode: ComputedTextWrapModeKind,
}

impl StyloComputedStyleSnapshot {
    pub(crate) fn computed_values(&self) -> ServoArc<ComputedValues> {
        self.primary.clone()
    }

    pub(crate) fn rendered_style_facts(&self) -> ComputedRenderedStyleFacts {
        let display = self.primary.clone_display();
        let content_visibility_applicable = !display.is_none()
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
            );
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
            content_visibility_applicable,
            visibility_visible: self.primary.clone_visibility() == ComputedVisibility::Visible,
            opacity_zero: self.primary.clone_opacity() == 0.0,
            text_transform,
            white_space_collapse,
            text_wrap_mode,
        }
    }

    pub(crate) fn property_value(&self, property: &str) -> Option<String> {
        if property.starts_with("--") {
            return serialize_computed_custom_property(&self.primary, property);
        }
        let property_id = PropertyId::parse_enabled_for_all_content(property).ok()?;
        serialize_raw_computed_property(&self.primary, property_id)
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
    inputs: &StyloComputedStyleInputs,
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
    let style_system_key = StyleSystemCacheKey::new(document_url, inputs, viewport);
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
                document_context,
                pseudo_element,
                |style| serialize_computed_custom_property(style, property),
            );
        }
        return with_resolved_styles(
            engine,
            host,
            document_url,
            handle,
            &style_system_key,
            inputs,
            document_context,
            pseudo_element.as_ref(),
            |styles| {
                let style = if let Some(ref pseudo_element) = pseudo_element {
                    styles.pseudos.get(pseudo_element)?
                } else {
                    styles.primary()
                };
                serialize_computed_custom_property(style, property)
            },
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
            document_context,
            pseudo_element,
            |style| serialize_resolved_computed_property(style, property_id.clone()),
        );
    }
    with_resolved_styles(
        engine,
        host,
        document_url,
        handle,
        &style_system_key,
        inputs,
        document_context,
        pseudo_element.as_ref(),
        |styles| {
            let style = if let Some(ref pseudo_element) = pseudo_element {
                styles.pseudos.get(pseudo_element)?
            } else {
                styles.primary()
            };
            serialize_resolved_computed_property(style, property_id)
        },
    )
}

pub(super) fn computed_style_snapshot_after_style_update(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    inputs: &StyloComputedStyleInputs,
    document_context: StyleSourceDocumentContext<'_>,
    read_document: DomHandle,
    viewport: StyleViewport,
) -> Option<StyloComputedStyleSnapshot> {
    let style_system_key = StyleSystemCacheKey::new(document_url, inputs, viewport);
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

pub(super) fn computed_style_snapshot_after_style_update_with_prepared_inputs(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    inputs: &StyloPreparedComputedStyleInputs,
    document_context: StyleSourceDocumentContext<'_>,
    read_document: DomHandle,
) -> Option<StyloComputedStyleSnapshot> {
    computed_style_snapshot_after_style_update_with_key(
        engine,
        host,
        document_url,
        handle,
        inputs.style_system_key(),
        inputs.inputs(),
        document_context,
        read_document,
    )
}

pub(super) fn computed_pseudo_style_snapshot_after_style_update_with_prepared_inputs(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    pseudo_element: &str,
    inputs: &StyloPreparedComputedStyleInputs,
    document_context: StyleSourceDocumentContext<'_>,
    read_document: DomHandle,
) -> Option<StyloComputedStyleSnapshot> {
    let owner_document = owner_document_for_computed_style_read(host, handle)?;
    trace_computed_style_read(
        document_url,
        handle,
        owner_document,
        read_document,
        "<computed-pseudo-style-snapshot>",
        Some(pseudo_element),
        document_context,
    );
    let pseudo_element = stylo_pseudo_element_for_computed_style(pseudo_element)?;
    if pseudo_element.is_lazy() {
        return with_lazily_resolved_pseudo_style(
            engine,
            host,
            document_url,
            handle,
            inputs.style_system_key(),
            inputs.inputs(),
            document_context,
            &pseudo_element,
            |style| {
                Some(StyloComputedStyleSnapshot {
                    primary: ServoArc::new(style.clone()),
                })
            },
        );
    }
    with_resolved_styles(
        engine,
        host,
        document_url,
        handle,
        inputs.style_system_key(),
        inputs.inputs(),
        document_context,
        Some(&pseudo_element),
        |styles| {
            Some(StyloComputedStyleSnapshot {
                primary: styles.pseudos.get(&pseudo_element)?.clone(),
            })
        },
    )
}

pub(super) fn computed_anonymous_style_snapshot_after_style_update_with_prepared_inputs(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    owner: DomHandle,
    parent_style: &ComputedValues,
    anonymous_kind: StyloAnonymousBoxKind,
    inputs: &StyloPreparedComputedStyleInputs,
    document_context: StyleSourceDocumentContext<'_>,
    read_document: DomHandle,
) -> Option<StyloComputedStyleSnapshot> {
    let owner_document = owner_document_for_computed_style_read(host, owner)?;
    trace_computed_style_read(
        document_url,
        owner,
        owner_document,
        read_document,
        "<computed-anonymous-style-snapshot>",
        Some(anonymous_kind.trace_name()),
        document_context,
    );
    let world = engine.world_for_document(owner_document);
    ensure_retained_style_system_for_computed_read(
        engine,
        host,
        &world,
        owner_document,
        inputs.style_system_key(),
        inputs.inputs(),
        document_context,
    );

    engine.dom_adapter.with_bound_host(host, |dom_adapter| {
        let shared_lock = dom_adapter.shared_lock().clone();
        let guard = shared_lock.read();
        let guards = StylesheetGuards::same(&guard);
        let pseudo = anonymous_kind.pseudo();
        let style = world.document_state.with_retained_style_system(|retained| {
            let _layout_thread_state = StyloLayoutThreadStateGuard::enter();
            retained
                .stylist
                .style_for_anonymous::<StyloElement<'_>>(&guards, &pseudo, parent_style)
        });
        Some(StyloComputedStyleSnapshot { primary: style })
    })
}

fn computed_style_snapshot_after_style_update_with_key(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    style_system_key: &StyleSystemCacheKey,
    inputs: &StyloComputedStyleInputs,
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
    with_resolved_styles(
        engine,
        host,
        document_url,
        handle,
        style_system_key,
        inputs,
        document_context,
        None,
        |styles| {
            Some(StyloComputedStyleSnapshot {
                primary: styles.primary().clone(),
            })
        },
    )
}

fn computed_custom_property_names_for_style(style: &ComputedValues) -> Vec<String> {
    let custom_properties = style.custom_properties();
    let mut names = Vec::new();
    let mut index = 0;
    while let Some((name, value)) = custom_properties.property_at(index) {
        if value.is_some() {
            names.push(format!("--{name}"));
        }
        index += 1;
    }
    names
}

fn with_resolved_styles<R>(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    style_system_key: &StyleSystemCacheKey,
    inputs: &StyloComputedStyleInputs,
    document_context: StyleSourceDocumentContext<'_>,
    pseudo_element: Option<&PseudoElement>,
    callback: impl FnOnce(&ElementStyles) -> Option<R>,
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
        document_context,
    );
    let computed_key = ComputedElementStyleCacheKey {
        computed_cache_generation: world.document_state.computed_cache_generation(),
        handle,
        pseudo_element: pseudo_element.cloned(),
    };
    if let Some(styles) = world.computed_style_cache.get(&computed_key) {
        return callback(&styles);
    }

    engine.dom_adapter.with_bound_host(host, |dom_adapter| {
        let element = dom_adapter.element(host, handle)?;

        dom_adapter.clear_element_data(handle);
        populate_inline_style_attributes_for_resolution(
            engine,
            host,
            document_url,
            handle,
            inputs.quirks_mode,
        );

        world
            .document_state
            .with_shadow_cascade_data(|shadow_cascade_data| {
                for (root, cascade_data) in shadow_cascade_data {
                    dom_adapter.set_shadow_cascade_data(*root, cascade_data.clone());
                }
            });

        let shared_lock = dom_adapter.shared_lock().clone();
        let guard = shared_lock.read();
        let guards = StylesheetGuards::same(&guard);

        let snapshot_map = SnapshotMap::new();
        let empty_painters = EmptyRegisteredSpeculativePainters;
        let mut retained_selector_caches = world.document_state.take_selector_caches();
        let styles = {
            world.document_state.with_retained_style_system(|retained| {
                let shared = SharedStyleContext {
                    stylist: &retained.stylist,
                    visited_styles_enabled: false,
                    options: StyleSystemOptions::default(),
                    guards,
                    current_time_for_animations: 0.0,
                    traversal_flags: TraversalFlags::empty(),
                    snapshot_map: &snapshot_map,
                    animations: DocumentAnimationSet::default(),
                    registered_speculative_painters: &empty_painters,
                };
                let _layout_thread_state = StyloLayoutThreadStateGuard::enter();
                let mut thread_local = ThreadLocalStyleContext::new();
                std::mem::swap(
                    &mut thread_local.selector_caches,
                    &mut retained_selector_caches,
                );
                let mut context = StyleContext {
                    shared: &shared,
                    thread_local: &mut thread_local,
                };
                materialize_ancestor_styles_for_resolution(&mut context, element);
                let styles = resolve_style(
                    &mut context,
                    element,
                    RuleInclusion::All,
                    pseudo_element,
                    None,
                );
                seed_root_font_relative_unit_state(&context, styles.primary());
                std::mem::swap(
                    &mut context.thread_local.selector_caches,
                    &mut retained_selector_caches,
                );
                styles
            })
        };
        world
            .document_state
            .replace_selector_caches(retained_selector_caches);

        world
            .computed_style_cache
            .insert(computed_key, styles.clone());
        callback(&styles)
    })
}

fn materialize_ancestor_styles_for_resolution<E>(context: &mut StyleContext<E>, element: E)
where
    E: TElement,
{
    let mut ancestors = Vec::new();
    let mut current = element.traversal_parent();
    while let Some(ancestor) = current {
        ancestors.push(ancestor);
        current = ancestor.traversal_parent();
    }

    for ancestor in ancestors.into_iter().rev() {
        let retained_primary = ancestor
            .borrow_data()
            .and_then(|data| data.styles.get_primary().cloned());
        if let Some(primary) = retained_primary {
            seed_root_font_relative_unit_state(context, &primary);
            continue;
        }
        let styles = resolve_style(context, ancestor, RuleInclusion::All, None, None);
        seed_root_font_relative_unit_state(context, styles.primary());
        unsafe {
            ancestor.ensure_data().styles = styles;
        }
    }
}

/// Keep Stylo's device-level root font-relative bases aligned with the root
/// style materialized by Moli's synchronous observation traversal.
///
/// A normal Stylo traversal performs this update from `finish_restyle`. Moli
/// resolves an exact ancestor chain with `resolve_style`, so it must publish
/// the same state before cascading descendants that use rem/rlh/root-metric
/// units. Retained root data must seed a freshly rebuilt device as well.
fn seed_root_font_relative_unit_state<E>(
    context: &StyleContext<E>,
    style: &ServoArc<ComputedValues>,
) where
    E: TElement,
{
    if !style
        .flags
        .contains(ComputedValueFlags::IS_ROOT_ELEMENT_STYLE)
    {
        return;
    }

    let device = context.shared.stylist.device();
    device.set_root_style(style);

    let font = style.get_font();
    let font_size = font.clone_font_size().computed_size();
    device.set_root_font_size(style.effective_zoom.unzoom(font_size.px()));

    let line_height = device.calc_line_height(font, style.writing_mode, None).0;
    device.set_root_line_height(style.effective_zoom.unzoom(line_height.px()));
}

fn with_lazily_resolved_pseudo_style<R>(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    style_system_key: &StyleSystemCacheKey,
    inputs: &StyloComputedStyleInputs,
    document_context: StyleSourceDocumentContext<'_>,
    pseudo_element: &PseudoElement,
    callback: impl FnOnce(&ComputedValues) -> Option<R>,
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
        document_context,
    );
    let computed_key = ComputedElementStyleCacheKey {
        computed_cache_generation: world.document_state.computed_cache_generation(),
        handle,
        pseudo_element: Some(pseudo_element.clone()),
    };
    if let Some(style) = world.computed_style_cache.get_lazy_pseudo(&computed_key) {
        return callback(&style);
    }

    let primary_style = with_resolved_styles(
        engine,
        host,
        document_url,
        handle,
        style_system_key,
        inputs,
        document_context,
        None,
        |styles| Some(styles.primary().clone()),
    )?;

    engine.dom_adapter.with_bound_host(host, |dom_adapter| {
        let element = dom_adapter.element(host, handle)?;

        world
            .document_state
            .with_shadow_cascade_data(|shadow_cascade_data| {
                for (root, cascade_data) in shadow_cascade_data {
                    dom_adapter.set_shadow_cascade_data(*root, cascade_data.clone());
                }
            });

        let shared_lock = dom_adapter.shared_lock().clone();
        let guard = shared_lock.read();
        let guards = StylesheetGuards::same(&guard);
        let style = world
            .document_state
            .with_retained_style_system(|retained| {
                let _layout_thread_state = StyloLayoutThreadStateGuard::enter();
                retained.stylist.lazily_compute_pseudo_element_style(
                    &guards,
                    element,
                    pseudo_element,
                    RuleInclusion::All,
                    &primary_style,
                    false,
                    None,
                )
            })?;
        world
            .computed_style_cache
            .insert_lazy_pseudo(computed_key, style.clone());
        callback(&style)
    })
}

fn ensure_retained_style_system_for_computed_read(
    engine: &MoliStyleEngine,
    host: &DomHost,
    world: &DocumentStyleWorld,
    retained_document: DomHandle,
    key: &StyleSystemCacheKey,
    inputs: &StyloComputedStyleInputs,
    document_context: StyleSourceDocumentContext<'_>,
) {
    let source_stores = world.borrow_source_stores();
    ensure_retained_style_system(
        host,
        &engine.dom_adapter,
        &world.document_state,
        &source_stores,
        document_context,
        retained_document,
        engine.cache_cleanup_for_world(world),
        key,
        inputs,
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
    key: StyleSystemCacheKey,
    inputs: &StyloComputedStyleInputs,
) {
    let world = engine.world_for_document(document);
    ensure_retained_style_system_for_computed_read(
        engine,
        host,
        &world,
        document,
        &key,
        inputs,
        StyleSourceDocumentContext::for_root_document(document),
    );
}

fn populate_inline_style_attributes_for_resolution(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    handle: DomHandle,
    quirks_mode: QuirksMode,
) {
    let shared_lock = engine.dom_adapter.shared_lock();
    let mut current = Some(handle);
    while let Some(node) = current {
        let world = engine.owner_document_world(host, node);
        let Some(world) = world else {
            current = inherited_style_parent(host, node);
            continue;
        };
        if world.inline_style_metadata.csp_state(node)
            == super::InlineStyleCspState::BlockedAttribute
        {
            engine.dom_adapter.clear_inline_style_attribute(node);
            current = inherited_style_parent(host, node);
            continue;
        }
        if let Some(style_attribute) = world
            .inline_style_metadata
            .resolution_text(node)
            .or_else(|| host.get_attribute(node, "style"))
        {
            let base_url = world
                .inline_style_metadata
                .base_url(node)
                .unwrap_or_else(|| style_resolution_base_url(host, document_url, node));
            let url_data = UrlExtraData::from(base_url);
            let declarations = parse_style_attribute(
                &style_attribute,
                &url_data,
                None,
                quirks_mode,
                CssRuleType::Style,
            );
            engine
                .dom_adapter
                .set_inline_style_attribute(node, ServoArc::new(shared_lock.wrap(declarations)));
        }
        current = inherited_style_parent(host, node);
    }
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

fn functional_pseudo_name<'a>(pseudo_element: &'a str, function_name: &str) -> Option<&'a str> {
    pseudo_element
        .strip_prefix(function_name)?
        .strip_prefix('(')?
        .strip_suffix(')')
        .filter(|name| !name.is_empty())
}

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

fn serialize_resolved_computed_property(
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
    Some(serialized)
}

fn serialize_computed_custom_property(style: &ComputedValues, property: &str) -> Option<String> {
    let custom_properties = style.custom_properties();
    let mut index = 0;
    while let Some((name, value)) = custom_properties.property_at(index) {
        if format!("--{name}") == property {
            value.as_ref()?;
            let property_id = PropertyId::Custom(name.clone());
            let mut context = resolved::Context {
                style,
                for_property: property_id.clone(),
                current_longhand: None,
            };
            let mut serialized = String::new();
            style
                .computed_or_resolved_property_value(
                    property_id,
                    Some(&mut context),
                    &mut serialized,
                )
                .ok()?;
            if serialized.is_empty() {
                return Some(" ".to_owned());
            }
            return Some(serialized);
        }
        index += 1;
    }
    None
}

fn inherited_style_parent(host: &DomHost, handle: DomHandle) -> Option<DomHandle> {
    if host.is_shadow_root(handle) {
        return host.shadow_root_host(handle);
    }
    let parent = host
        .node(handle)
        .and_then(crate::dom::native::Node::parent_node)?;
    if host.is_shadow_root(parent) {
        return host.shadow_root_host(parent);
    }
    Some(parent)
}

struct EmptyRegisteredSpeculativePainters;

impl RegisteredSpeculativePainters for EmptyRegisteredSpeculativePainters {
    fn get(&self, _name: &Atom) -> Option<&dyn RegisteredSpeculativePainter> {
        None
    }
}

struct StyloLayoutThreadStateGuard {
    entered: bool,
}

impl StyloLayoutThreadStateGuard {
    fn enter() -> Self {
        let entered = !thread_state::get().contains(ThreadState::LAYOUT);
        if entered {
            thread_state::enter(ThreadState::LAYOUT);
        }
        Self { entered }
    }
}

impl Drop for StyloLayoutThreadStateGuard {
    fn drop(&mut self) {
        if self.entered {
            thread_state::exit(ThreadState::LAYOUT);
        }
    }
}
