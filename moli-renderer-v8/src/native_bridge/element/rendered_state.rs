use moli_layout::LayoutError;
use moli_webapi::element_visibility::parse_check_visibility_options;
use std::collections::HashSet;

use crate::{
    document_runtime::DomHandle,
    dom::native::{Element, Node},
    native_bridge::PageScriptHost,
    style_engine::{
        ComputedDisplayKind, ComputedTextTransformKind, ComputedTextWrapModeKind,
        ComputedWhiteSpaceCollapseKind,
    },
    webidl,
};

use super::{
    super::node::{
        node_is_element, node_runtime_and_handle_from_args_or_detached,
        node_runtime_and_handle_from_object_or_detached, require_element_method_receiver,
        throw_incompatible_getter_receiver, throw_incompatible_method_receiver,
    },
    styles::StyleObservation,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ElementBoxState {
    Box,
    DisplayContents,
    NoBox,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ElementRenderedStyle {
    pub(crate) display: ComputedDisplayKind,
    effective_zoom: f32,
    pub(crate) visibility_visible: bool,
    pub(crate) hides_contents: bool,
    pub(super) opacity_zero: bool,
    pub(super) text_transform: ComputedTextTransformKind,
    pub(crate) white_space_collapse: ComputedWhiteSpaceCollapseKind,
    pub(super) text_wrap_mode: ComputedTextWrapModeKind,
}

impl ElementRenderedStyle {
    pub(crate) fn read_in_scope(scope: &mut StyleObservation<'_>, handle: DomHandle) -> Self {
        let style = scope.read(handle);
        let effective_zoom = style
            .computed_values()
            .map_or(1.0, |values| values.effective_zoom.value());
        if let Some(facts) = style.rendered_style_facts() {
            return Self {
                display: facts.display,
                effective_zoom,
                visibility_visible: facts.visibility_visible,
                hides_contents: facts.hides_contents,
                opacity_zero: facts.opacity_zero,
                text_transform: facts.text_transform,
                white_space_collapse: facts.white_space_collapse,
                text_wrap_mode: facts.text_wrap_mode,
            };
        }
        let display_value = style.property("display");
        let display = computed_display_kind(&display_value);
        Self {
            display,
            effective_zoom,
            visibility_visible: style.property("visibility") == "visible",
            hides_contents: fallback_content_visibility_applicable(&display_value)
                && style.property("content-visibility") == "hidden",
            opacity_zero: computed_opacity_is_zero(&style.property("opacity")),
            text_transform: computed_text_transform_kind(&style.property("text-transform")),
            white_space_collapse: computed_white_space_collapse_kind(
                &style.property("white-space-collapse"),
            ),
            text_wrap_mode: computed_text_wrap_mode_kind(&style.property("text-wrap-mode")),
        }
    }
}

fn computed_display_kind(value: &str) -> ComputedDisplayKind {
    match value {
        "none" => ComputedDisplayKind::None,
        "contents" => ComputedDisplayKind::Contents,
        "inline" => ComputedDisplayKind::Inline,
        "inline-block" | "inline-flex" | "inline-grid" | "inline-flow-root" => {
            ComputedDisplayKind::InlineAtomic
        }
        "block" => ComputedDisplayKind::Block,
        "table" => ComputedDisplayKind::Table,
        "inline-table" => ComputedDisplayKind::InlineTable,
        "table-row-group" | "table-header-group" | "table-footer-group" | "table-column"
        | "table-column-group" => ComputedDisplayKind::TableInternal,
        "table-row" => ComputedDisplayKind::TableRow,
        "table-cell" => ComputedDisplayKind::TableCell,
        "list-item" => ComputedDisplayKind::ListItem,
        _ => ComputedDisplayKind::Other,
    }
}

fn fallback_content_visibility_applicable(display: &str) -> bool {
    !matches!(
        display,
        "none"
            | "contents"
            | "inline"
            | "table"
            | "inline-table"
            | "table-row-group"
            | "table-column"
            | "table-column-group"
            | "table-header-group"
            | "table-footer-group"
            | "table-row"
            | "table-caption"
    )
}

fn computed_text_transform_kind(value: &str) -> ComputedTextTransformKind {
    value
        .split_ascii_whitespace()
        .find_map(|value| match value.to_ascii_lowercase().as_str() {
            "none" => Some(ComputedTextTransformKind::None),
            "uppercase" => Some(ComputedTextTransformKind::Uppercase),
            "lowercase" => Some(ComputedTextTransformKind::Lowercase),
            "capitalize" => Some(ComputedTextTransformKind::Capitalize),
            _ => None,
        })
        .unwrap_or(ComputedTextTransformKind::Other)
}

fn computed_white_space_collapse_kind(value: &str) -> ComputedWhiteSpaceCollapseKind {
    match value {
        "collapse" => ComputedWhiteSpaceCollapseKind::Collapse,
        "preserve" => ComputedWhiteSpaceCollapseKind::Preserve,
        "preserve-breaks" => ComputedWhiteSpaceCollapseKind::PreserveBreaks,
        "break-spaces" => ComputedWhiteSpaceCollapseKind::BreakSpaces,
        _ => ComputedWhiteSpaceCollapseKind::Other,
    }
}

fn computed_text_wrap_mode_kind(value: &str) -> ComputedTextWrapModeKind {
    match value {
        "wrap" => ComputedTextWrapModeKind::Wrap,
        "nowrap" => ComputedTextWrapModeKind::NoWrap,
        _ => ComputedTextWrapModeKind::Other,
    }
}

struct RenderedPathEntry {
    style: Option<ElementRenderedStyle>,
}

/// A single computed-style snapshot of the target's inclusive flat-tree path.
///
/// Blink answers these questions from its retained layout tree. Moli has no
/// layout tree, so this is the lightweight owner for the same observable
/// boundary: flat-tree membership, an active render root, `display`, and
/// `content-visibility`.
pub(super) struct ElementRenderedState {
    path: Vec<RenderedPathEntry>,
    reaches_active_root: bool,
    hidden_by_closed_details: bool,
}

pub(crate) fn element_is_being_rendered(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    ElementRenderedState::read(runtime, handle).box_state() == ElementBoxState::Box
}

pub(crate) fn rendered_text_sources(
    scope: &mut StyleObservation<'_>,
    document: DomHandle,
    sources: &[DomHandle],
) -> Result<HashSet<DomHandle>, LayoutError> {
    let mut rendered = match super::geometry::observable_sources_with_fragments(
        scope.runtime(),
        document,
        sources,
    ) {
        Ok(rendered) => rendered,
        Err(LayoutError::NoLayoutSnapshot) => HashSet::new(),
        Err(error) => return Err(error),
    };
    // An on-demand layout snapshot can predate DOM edits in this script turn.
    // Resolve missing text sources against the current rendered tree.
    for &source in sources {
        if !rendered.contains(&source) && text_source_is_in_rendered_content(scope, source) {
            rendered.insert(source);
        }
    }
    Ok(rendered)
}

fn text_source_is_in_rendered_content(scope: &mut StyleObservation<'_>, handle: DomHandle) -> bool {
    let state = ElementRenderedState::read_in_scope(scope, handle);
    if state.box_state() == ElementBoxState::NoBox
        || !state
            .path
            .iter()
            .find_map(|entry| entry.style)
            .is_some_and(|style| style.visibility_visible)
    {
        return false;
    }
    rendered_dom_ancestors(scope.runtime(), handle)
        .all(|element| !rendered_element_ignores_dom_children(element) && !element.is_html_select())
}

pub(super) fn rendered_dom_ancestors(
    runtime: &PageScriptHost,
    handle: DomHandle,
) -> impl Iterator<Item = &Element> {
    let document = runtime.dom_host().owner_document_handle(handle);
    std::iter::successors(rendered_tree_parent(runtime, handle), move |parent| {
        rendered_tree_parent(runtime, *parent)
    })
    .take_while(move |parent| Some(*parent) != document)
    .filter_map(move |parent| runtime.dom_host().node(parent)?.as_element())
}

pub(crate) fn rendered_element_ignores_dom_children(element: &Element) -> bool {
    element.is_html_element("input")
        || element.is_html_element("textarea")
        || element.is_html_element("iframe")
        || element.is_html_element("audio")
        || element.is_html_element("video")
        || element.is_html_element("canvas")
        || element.is_html_element("object")
        || element.is_html_element("img")
        || element.is_html_element("embed")
        || (element.namespace() == "http://www.w3.org/2000/svg"
            && (element.local_name() == "stop"
                || svg_local_name_is_non_rendered_container(element.local_name())))
}

pub(crate) fn svg_local_name_is_non_rendered_container(local_name: &str) -> bool {
    matches!(
        local_name,
        "clipPath" | "defs" | "marker" | "mask" | "pattern" | "symbol"
    )
}

impl ElementRenderedState {
    pub(super) fn read(runtime: &PageScriptHost, handle: DomHandle) -> Self {
        let mut scope = StyleObservation::new(runtime);
        Self::read_in_scope(&mut scope, handle)
    }

    pub(super) fn read_in_scope(scope: &mut StyleObservation<'_>, handle: DomHandle) -> Self {
        let runtime = scope.runtime();
        if super::details_dialog::node_is_hidden_by_closed_details(runtime, handle) {
            return Self {
                path: Vec::new(),
                reaches_active_root: false,
                hidden_by_closed_details: true,
            };
        }
        let mut current = handle;
        let mut visited = HashSet::new();
        let mut path = Vec::new();

        loop {
            if !visited.insert(current) {
                return Self {
                    path,
                    reaches_active_root: false,
                    hidden_by_closed_details: false,
                };
            }

            let style = runtime
                .dom_host()
                .node(current)
                .filter(|node| node.is_element())
                .map(|_| ElementRenderedStyle::read_in_scope(scope, current));
            path.push(RenderedPathEntry { style });

            let Some(parent) = rendered_tree_parent(runtime, current) else {
                return Self {
                    path,
                    reaches_active_root: is_active_render_tree_root(runtime, current),
                    hidden_by_closed_details: false,
                };
            };
            current = parent;
        }
    }

    /// Returns the lightweight equivalent of whether the target has an
    /// associated CSS box.
    pub(super) fn box_state(&self) -> ElementBoxState {
        if self.hidden_by_closed_details || !self.reaches_active_root {
            return ElementBoxState::NoBox;
        }
        let target_display_contents = self
            .target_style()
            .is_some_and(|style| style.display == ComputedDisplayKind::Contents);
        for (index, entry) in self.path.iter().enumerate() {
            let Some(style) = entry.style.as_ref() else {
                continue;
            };
            if style.display == ComputedDisplayKind::None || (index != 0 && style.hides_contents) {
                return ElementBoxState::NoBox;
            }
        }
        if target_display_contents {
            ElementBoxState::DisplayContents
        } else {
            ElementBoxState::Box
        }
    }

    /// Models Blink's `LockedInclusiveAncestorPreventingPaint` rather than
    /// merely looking for a declaration. A detached or `display:none` subtree
    /// never creates a display lock, while an outer active lock still hides a
    /// `display:none` descendant.
    pub(super) fn has_content_visibility_lock(&self) -> bool {
        if self.hidden_by_closed_details {
            return true;
        }
        if !self.reaches_active_root {
            return false;
        }
        for entry in self.path.iter().rev() {
            let Some(style) = entry.style.as_ref() else {
                continue;
            };
            if style.display == ComputedDisplayKind::None {
                return false;
            }
            if style.hides_contents {
                return true;
            }
        }
        false
    }

    pub(super) fn target_style(&self) -> Option<&ElementRenderedStyle> {
        self.path.first().and_then(|entry| entry.style.as_ref())
    }

    /// Returns the cumulative CSS zoom of the target's generated box.
    ///
    /// CSSOM View exposes `1` when the element has no associated box. Reading
    /// the value from this style-observation path keeps it synchronous without
    /// coupling the API to a retained screenshot geometry snapshot.
    fn current_css_zoom(&self) -> f64 {
        if self.box_state() != ElementBoxState::Box {
            return 1.0;
        }
        self.target_style()
            .map_or(1.0, |style| f64::from(style.effective_zoom))
    }

    fn has_zero_opacity(&self) -> bool {
        self.path
            .iter()
            .any(|entry| entry.style.as_ref().is_some_and(|style| style.opacity_zero))
    }
}

pub(super) fn rendered_child_participates_in_flat_tree(
    scope: &mut StyleObservation<'_>,
    parent: DomHandle,
    child: DomHandle,
) -> bool {
    let runtime = scope.runtime();
    if !super::details_dialog::closed_details_child_participates(runtime, parent, child) {
        return false;
    }
    if runtime.dom_host().assigned_slot_for_node(child).is_some() {
        return ElementRenderedState::read_in_scope(scope, child).box_state()
            != ElementBoxState::NoBox;
    }
    !(runtime.dom_host().shadow_root_handle(parent).is_some()
        && rendered_tree_node_is_slotable(runtime, child))
}

fn rendered_tree_parent(runtime: &PageScriptHost, handle: DomHandle) -> Option<DomHandle> {
    if runtime
        .dom_host()
        .node(handle)
        .is_some_and(Node::is_document)
    {
        return runtime.child_browsing_context_host_for_document_handle(handle);
    }
    if let Some(slot) = runtime.dom_host().assigned_slot_for_node(handle) {
        return Some(slot);
    }
    let parent = runtime
        .dom_host()
        .node(handle)
        .and_then(Node::parent_node)?;
    if runtime.dom_host().is_shadow_root(parent) {
        return runtime.dom_host().shadow_root_host(parent);
    }
    if runtime.dom_host().shadow_root_handle(parent).is_some()
        && rendered_tree_node_is_slotable(runtime, handle)
    {
        return None;
    }
    Some(parent)
}

fn rendered_tree_node_is_slotable(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    runtime
        .dom_host()
        .node(handle)
        .is_some_and(|node| node.is_element() || node.is_text())
}

fn is_active_render_tree_root(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    if handle == runtime.document_handle()
        && runtime
            .dom_host()
            .node(handle)
            .is_some_and(Node::is_document)
    {
        return true;
    }
    runtime
        .lightweight_popup_id_for_document_handle(handle)
        .is_some_and(|popup_id| runtime.lightweight_popup_is_open(popup_id))
}

pub(super) fn node_check_visibility_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        throw_incompatible_method_receiver(scope, "Element", "checkVisibility");
        rv.set_bool(false);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !require_element_method_receiver(scope, runtime, handle, "checkVisibility") {
        rv.set_bool(false);
        return;
    }
    let options = match parse_check_visibility_options(scope, &args) {
        Ok(options) => options,
        Err(error) => {
            webidl::throw_error(scope, &error);
            rv.set_bool(false);
            return;
        }
    };

    let rendered_state = ElementRenderedState::read(runtime, handle);
    if rendered_state.box_state() != ElementBoxState::Box {
        rv.set_bool(false);
        return;
    }

    let Some(target_style) = rendered_state.target_style() else {
        rv.set_bool(false);
        return;
    };
    if (options.check_visibility_css || options.visibility_property)
        && !target_style.visibility_visible
    {
        rv.set_bool(false);
        return;
    }

    if (options.check_opacity || options.opacity_property) && rendered_state.has_zero_opacity() {
        rv.set_bool(false);
        return;
    }

    // Moli does not skip offscreen `content-visibility:auto` subtrees because
    // it intentionally has no viewport-driven layout locking. The option is
    // still parsed so its Web IDL behavior matches Chromium.
    let _ = options.content_visibility_auto;
    rv.set_bool(true);
}

pub(super) fn node_current_css_zoom_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, args.this())
    else {
        throw_incompatible_getter_receiver(scope, "Element", "currentCSSZoom");
        rv.set(v8::Number::new(scope, 1.0).into());
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if !node_is_element(runtime, handle) {
        throw_incompatible_getter_receiver(scope, "Element", "currentCSSZoom");
        rv.set(v8::Number::new(scope, 1.0).into());
        return;
    }

    let zoom = ElementRenderedState::read(runtime, handle).current_css_zoom();
    rv.set(v8::Number::new(scope, zoom).into());
}

fn computed_opacity_is_zero(value: &str) -> bool {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        return percent.trim().parse::<f64>().ok() == Some(0.0);
    }
    value.parse::<f64>().ok() == Some(0.0)
}
