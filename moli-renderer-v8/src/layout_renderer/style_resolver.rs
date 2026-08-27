use moli_layout::{
    LayoutDisplay, LayoutError, LayoutStyleResolver, LayoutViewport, ResolvedLayoutElementStyles,
    ResolvedLayoutPseudoStyle, ResolvedLayoutStyle,
};
use std::time::Instant;

use crate::{
    document_runtime::DomHandle,
    native_bridge::{JsContextHost, element::StyleObservation},
    style_engine::{StyleViewport, StyloAnonymousBoxKind},
};

pub(super) struct NativeLayoutStyleResolver<'a> {
    runtime: &'a JsContextHost,
    reads: StyleObservation<'a>,
    scripting_enabled: bool,
    profile: Option<NativeLayoutStyleResolverProfile>,
}

#[derive(Default)]
struct NativeLayoutStyleResolverProfile {
    primary_count: u64,
    primary_read_ns: u128,
    primary_projection_ns: u128,
    primary_policy_ns: u128,
    eager_pseudo_count: u64,
    eager_pseudo_projection_ns: u128,
    marker_count: u64,
    marker_read_ns: u128,
    marker_projection_ns: u128,
    anonymous_count: u64,
    anonymous_read_ns: u128,
    anonymous_projection_ns: u128,
}

impl<'a> NativeLayoutStyleResolver<'a> {
    pub(super) fn new(
        runtime: &'a JsContextHost,
        root: DomHandle,
        document: DomHandle,
        viewport: LayoutViewport,
    ) -> Self {
        let mut reads = layout_style_read_scope(runtime, document, viewport);
        let _ = reads.read(root).computed_values();
        Self {
            runtime,
            reads,
            scripting_enabled: runtime.document_scripting_enabled(document),
            profile: moli_trace::cpu_profile_enabled()
                .then(NativeLayoutStyleResolverProfile::default),
        }
    }

    pub(super) fn trace_profile(&self, document: crate::document_runtime::DomHandle) {
        let Some(profile) = self.profile.as_ref() else {
            return;
        };
        tracing::info!(
            target: "moli_cpu_profile",
            stage = "layout_style_resolver",
            document = document.index_u32(),
            primary_count = profile.primary_count,
            primary_read_us = profile.primary_read_ns / 1_000,
            primary_projection_us = profile.primary_projection_ns / 1_000,
            primary_policy_us = profile.primary_policy_ns / 1_000,
            eager_pseudo_count = profile.eager_pseudo_count,
            eager_pseudo_projection_us = profile.eager_pseudo_projection_ns / 1_000,
            marker_count = profile.marker_count,
            marker_read_us = profile.marker_read_ns / 1_000,
            marker_projection_us = profile.marker_projection_ns / 1_000,
            anonymous_count = profile.anonymous_count,
            anonymous_read_us = profile.anonymous_read_ns / 1_000,
            anonymous_projection_us = profile.anonymous_projection_ns / 1_000,
        );
    }
}

fn layout_style_read_scope<'a>(
    runtime: &'a JsContextHost,
    document: DomHandle,
    viewport: LayoutViewport,
) -> StyleObservation<'a> {
    let viewport = layout_style_viewport(runtime, viewport);
    if document == runtime.document_handle() && viewport == runtime.style_viewport() {
        // Keep the normal main-document observation environment so layout and
        // rendered-text collection read the same persistent style world. Only
        // an override viewport or embedded Document needs an explicit context.
        StyleObservation::new(runtime)
    } else {
        StyleObservation::new_for_document_viewport(runtime, document, viewport)
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
    fn element_styles(
        &mut self,
        node: DomHandle,
    ) -> Result<Option<ResolvedLayoutElementStyles>, LayoutError> {
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        let read = self.reads.read(node);
        let Some((computed, before, after)) = read.into_element_computed_values() else {
            if let Some(profile) = self.profile.as_mut() {
                profile.primary_count = profile.primary_count.saturating_add(1);
                profile.primary_read_ns = profile.primary_read_ns.saturating_add(
                    phase_started
                        .map(|started| started.elapsed().as_nanos())
                        .unwrap_or_default(),
                );
            }
            return Ok(None);
        };
        let eager_pseudo_count = u64::from(before.is_some()) + u64::from(after.is_some());
        if let Some(profile) = self.profile.as_mut() {
            profile.primary_count = profile.primary_count.saturating_add(1);
            profile.primary_read_ns = profile.primary_read_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        let mut resolved = ResolvedLayoutStyle::from_stylo(computed);
        if let Some(profile) = self.profile.as_mut() {
            profile.primary_projection_ns = profile.primary_projection_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        let before = before.map(ResolvedLayoutStyle::from_stylo);
        let after = after.map(ResolvedLayoutStyle::from_stylo);
        if let Some(profile) = self.profile.as_mut() {
            profile.eager_pseudo_count = profile
                .eager_pseudo_count
                .saturating_add(eager_pseudo_count);
            profile.eager_pseudo_projection_ns = profile.eager_pseudo_projection_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
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
        if let Some(profile) = self.profile.as_mut() {
            profile.primary_policy_ns = profile.primary_policy_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        Ok(Some(ResolvedLayoutElementStyles::new(
            resolved, before, after,
        )))
    }

    fn marker_style(
        &mut self,
        node: DomHandle,
    ) -> Result<Option<ResolvedLayoutPseudoStyle>, LayoutError> {
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        let read = self.reads.read(node);
        let computed = read.pseudo_computed_values("marker");
        if let Some(profile) = self.profile.as_mut() {
            profile.marker_count = profile.marker_count.saturating_add(1);
            profile.marker_read_ns = profile.marker_read_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        let resolved = computed.map(|computed| {
            ResolvedLayoutPseudoStyle::new(ResolvedLayoutStyle::from_stylo(computed))
        });
        if let Some(profile) = self.profile.as_mut() {
            profile.marker_projection_ns = profile.marker_projection_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        Ok(resolved)
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
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        let read = self.reads.read(owner);
        let computed = read
            .anonymous_computed_values(parent_computed.as_ref(), anonymous_kind)
            .ok_or_else(|| {
                LayoutError::style_resolution(
                    format!("{owner:?}"),
                    format!("Stylo could not resolve {anonymous_kind:?} anonymous style"),
                )
            })?;
        if let Some(profile) = self.profile.as_mut() {
            profile.anonymous_count = profile.anonymous_count.saturating_add(1);
            profile.anonymous_read_ns = profile.anonymous_read_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
        let phase_started = self.profile.as_ref().map(|_| Instant::now());
        let mut resolved = ResolvedLayoutStyle::from_stylo(computed);
        resolved.force_layout_display(display);
        if let Some(profile) = self.profile.as_mut() {
            profile.anonymous_projection_ns = profile.anonymous_projection_ns.saturating_add(
                phase_started
                    .map(|started| started.elapsed().as_nanos())
                    .unwrap_or_default(),
            );
        }
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

#[cfg(test)]
mod tests {
    use super::anonymous_box_kind;
    use crate::style_engine::StyloAnonymousBoxKind;
    use moli_layout::LayoutDisplay;

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
