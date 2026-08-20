use std::collections::HashMap;

use moli_layout::{
    LayoutDisplay, LayoutError, LayoutLastRememberedSizePolicy, LayoutPseudo, LayoutStyleResolver,
    LayoutViewport, ResolvedLayoutStyle,
};

use crate::{
    document_runtime::DomHandle,
    native_bridge::{JsContextHost, element::ComputedStyleReadScope},
    style_engine::{StyleViewport, StyloAnonymousBoxKind},
};

use super::{
    display_lock::{AutoDisplayLockState, DisplayLockStyleRequests},
    remembered_size::IntrinsicSizeObserverState,
};

pub(super) struct NativeLayoutStyleResolver<'a> {
    runtime: &'a JsContextHost,
    reads: ComputedStyleReadScope<'a>,
    scripting_enabled: bool,
    display_locks: &'a AutoDisplayLockState,
    remembered_sizes: IntrinsicSizeObserverState,
    remembered_size_policies: HashMap<DomHandle, LayoutLastRememberedSizePolicy>,
    display_lock_requests: DisplayLockStyleRequests,
}

pub(super) struct NativeLayoutPassPolicies {
    pub(super) remembered_sizes: HashMap<DomHandle, LayoutLastRememberedSizePolicy>,
    pub(super) display_locks: DisplayLockStyleRequests,
}

impl<'a> NativeLayoutStyleResolver<'a> {
    pub(super) fn new(
        runtime: &'a JsContextHost,
        document: DomHandle,
        viewport: LayoutViewport,
        display_locks: &'a AutoDisplayLockState,
        remembered_sizes: &IntrinsicSizeObserverState,
    ) -> Self {
        Self {
            runtime,
            reads: layout_style_read_scope(runtime, document, viewport),
            scripting_enabled: runtime.document_scripting_enabled(document),
            display_locks,
            remembered_sizes: remembered_sizes.clone(),
            remembered_size_policies: HashMap::new(),
            display_lock_requests: DisplayLockStyleRequests::default(),
        }
    }

    pub(super) fn into_pass_policies(self) -> NativeLayoutPassPolicies {
        NativeLayoutPassPolicies {
            remembered_sizes: self.remembered_size_policies,
            display_locks: self.display_lock_requests,
        }
    }
}

/// Completes pending style work before the synchronous, non-reentrant layout
/// pass starts. The pass reads one NativeDom/Stylo view and retains every
/// computed allocation it projects until its pass-local world is dropped.
pub(super) fn prepare_layout_style_inputs(
    runtime: &JsContextHost,
    root: DomHandle,
    document: DomHandle,
    viewport: LayoutViewport,
) {
    let mut reads = layout_style_read_scope(runtime, document, viewport);
    let _ = reads.read(root).computed_values();
}

/// Applies computed-style changes to state that can outlive a principal
/// layout box. This separate walk is required for remembered elements below a
/// `display:none` ancestor: box construction will not ask for their style, but
/// losing an `auto` component still clears that logical axis.
pub(super) fn reconcile_remembered_size_policies(
    runtime: &JsContextHost,
    document: DomHandle,
    viewport: LayoutViewport,
    remembered_sizes: &mut IntrinsicSizeObserverState,
) {
    let handles = remembered_sizes
        .handles()
        .filter(|element| runtime.dom_host().owner_document_handle(*element) == Some(document))
        .collect::<Vec<_>>();
    if handles.is_empty() {
        return;
    }

    let mut reads = layout_style_read_scope(runtime, document, viewport);
    for element in handles {
        let read = reads.read(element);
        let Some(computed) = read.computed_values() else {
            // Blink deliberately preserves remembered values when there is no
            // computed style. The element is unobserved until style returns.
            continue;
        };
        let resolved = ResolvedLayoutStyle::from_stylo(computed);
        remembered_sizes.reconcile_policy(element, resolved.last_remembered_size_policy());
    }
}

fn layout_style_read_scope<'a>(
    runtime: &'a JsContextHost,
    document: DomHandle,
    viewport: LayoutViewport,
) -> ComputedStyleReadScope<'a> {
    let viewport = layout_style_viewport(runtime, viewport);
    if document == runtime.document_handle() && viewport == runtime.style_viewport() {
        // Keep the normal main-document observation key so layout and
        // rendered-text collection can reuse the same prepared Stylo input.
        // Only an override viewport or an embedded Document needs the explicit
        // read-document context below.
        ComputedStyleReadScope::new(runtime)
    } else {
        ComputedStyleReadScope::new_for_document_viewport(runtime, document, viewport)
    }
}

fn layout_style_viewport(runtime: &JsContextHost, viewport: LayoutViewport) -> StyleViewport {
    let screen = runtime.style_viewport();
    StyleViewport::new(
        Some(f64::from(viewport.css_width)),
        Some(f64::from(viewport.css_height)),
    )
    .with_screen_size(screen.screen_width, screen.screen_height)
}

impl LayoutStyleResolver<DomHandle> for NativeLayoutStyleResolver<'_> {
    fn primary_style(
        &mut self,
        node: DomHandle,
    ) -> Result<Option<ResolvedLayoutStyle>, LayoutError> {
        let read = self.reads.read(node);
        let Some(computed) = read.computed_values() else {
            return Ok(None);
        };
        let mut resolved = ResolvedLayoutStyle::from_stylo(computed);
        let requests_auto = resolved.content_visibility_is_auto();
        if requests_auto || self.display_locks.contains(node) {
            self.display_lock_requests.record(node, requests_auto);
        }
        resolved.resolve_content_visibility_state(
            requests_auto && self.display_locks.is_locked_for_layout(node),
            self.remembered_sizes.get(node),
        );
        self.remembered_size_policies
            .insert(node, resolved.last_remembered_size_policy());
        if self
            .runtime
            .dom_host()
            .get_attribute(node, "hidden")
            .is_some()
        {
            resolved.force_display_none();
        }
        if self.scripting_enabled
            && self
                .runtime
                .dom_host()
                .is_html_element_named(node, "noscript")
        {
            // Match Blink's HTMLNoScriptElement::LayoutObjectIsNeeded(): the
            // computed `display` remains observable, including author
            // overrides, but no layout object is generated while this exact
            // Document can execute scripts.
            resolved.force_display_none();
        }
        Ok(Some(resolved))
    }

    fn pseudo_style(
        &mut self,
        node: DomHandle,
        pseudo: LayoutPseudo,
    ) -> Result<Option<ResolvedLayoutStyle>, LayoutError> {
        let pseudo_name = pseudo_style_name(pseudo);
        let read = self.reads.read(node);
        let computed = read.pseudo_computed_values(pseudo_name);
        Ok(computed.map(ResolvedLayoutStyle::from_stylo))
    }

    fn anonymous_style(
        &mut self,
        owner: DomHandle,
        parent: &ResolvedLayoutStyle,
        display: LayoutDisplay,
    ) -> Result<ResolvedLayoutStyle, LayoutError> {
        let parent_computed = parent.stylo_computed_values().ok_or_else(|| {
            LayoutError::style_resolution(
                format!("{owner:?}"),
                "native anonymous box parent has no retained Stylo ComputedValues",
            )
        })?;
        let anonymous_kind = anonymous_box_kind(display);
        let read = self.reads.read(owner);
        let computed = read
            .anonymous_computed_values(parent_computed.as_ref(), anonymous_kind)
            .ok_or_else(|| {
                LayoutError::style_resolution(
                    format!("{owner:?}"),
                    format!("Stylo could not resolve {anonymous_kind:?} anonymous style"),
                )
            })?;
        let mut resolved = ResolvedLayoutStyle::from_stylo(computed);
        resolved.force_layout_display(display);
        Ok(resolved)
    }
}

const fn anonymous_box_kind(display: LayoutDisplay) -> StyloAnonymousBoxKind {
    match display {
        LayoutDisplay::Table | LayoutDisplay::InlineTable => StyloAnonymousBoxKind::Table,
        LayoutDisplay::TableRow => StyloAnonymousBoxKind::TableRow,
        LayoutDisplay::TableCell => StyloAnonymousBoxKind::TableCell,
        LayoutDisplay::None
        | LayoutDisplay::Contents
        | LayoutDisplay::Block
        | LayoutDisplay::FlowRoot
        | LayoutDisplay::Inline
        | LayoutDisplay::InlineBlock
        | LayoutDisplay::Flex
        | LayoutDisplay::InlineFlex
        | LayoutDisplay::Grid
        | LayoutDisplay::InlineGrid
        | LayoutDisplay::BlockListItem
        | LayoutDisplay::InlineListItem
        | LayoutDisplay::TableCaption
        | LayoutDisplay::TableRowGroup
        | LayoutDisplay::TableHeaderGroup
        | LayoutDisplay::TableFooterGroup
        | LayoutDisplay::TableColumnGroup
        | LayoutDisplay::TableColumn => StyloAnonymousBoxKind::Generic,
    }
}

const fn pseudo_style_name(pseudo: LayoutPseudo) -> &'static str {
    match pseudo {
        LayoutPseudo::Marker => "marker",
        LayoutPseudo::Before => "before",
        LayoutPseudo::After => "after",
    }
}

#[cfg(test)]
mod tests {
    use super::{anonymous_box_kind, pseudo_style_name};
    use crate::style_engine::StyloAnonymousBoxKind;
    use moli_layout::{LayoutDisplay, LayoutPseudo};

    #[test]
    fn all_layout_pseudos_map_to_stylo_names() {
        assert_eq!(pseudo_style_name(LayoutPseudo::Marker), "marker");
        assert_eq!(pseudo_style_name(LayoutPseudo::Before), "before");
        assert_eq!(pseudo_style_name(LayoutPseudo::After), "after");
    }

    #[test]
    fn anonymous_table_roles_use_the_matching_servo_precomputed_pseudo() {
        assert_eq!(
            anonymous_box_kind(LayoutDisplay::Table),
            StyloAnonymousBoxKind::Table
        );
        assert_eq!(
            anonymous_box_kind(LayoutDisplay::TableRow),
            StyloAnonymousBoxKind::TableRow
        );
        assert_eq!(
            anonymous_box_kind(LayoutDisplay::TableCell),
            StyloAnonymousBoxKind::TableCell
        );
        assert_eq!(
            anonymous_box_kind(LayoutDisplay::TableRowGroup),
            StyloAnonymousBoxKind::Generic
        );
        assert_eq!(
            anonymous_box_kind(LayoutDisplay::Block),
            StyloAnonymousBoxKind::Generic
        );
    }
}
