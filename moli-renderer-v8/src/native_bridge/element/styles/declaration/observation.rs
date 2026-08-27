use std::{cell::RefCell, rc::Rc};

use crate::{
    document_runtime::DomHandle,
    style_engine::{
        FullStyleWorldSnapshot, PreparedStyleWorldUpdate, StyleObservationSnapshot, StyleViewport,
        StyleWorldUpdatePlan, StyloComputedStyleSnapshot, StyloStyleEnvironment,
    },
};

use super::super::super::super::JsContextHost;
use super::super::style_viewport_for_document;
use super::style_world::{
    StyleObservationKey, prepare_style_world_update, stylesheet_query_fallback,
    stylesheet_source_document_for_handle, stylo_style_environment,
};

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::native_bridge::element::styles) struct StyleComputationContext {
    pub(in crate::native_bridge::element::styles) viewport: StyleViewport,
    pub(in crate::native_bridge::element::styles) read_document: Option<DomHandle>,
}

impl StyleComputationContext {
    pub(in crate::native_bridge::element::styles) const fn new(viewport: StyleViewport) -> Self {
        Self {
            viewport,
            read_document: None,
        }
    }

    pub(in crate::native_bridge::element::styles) const fn viewport_width(self) -> Option<f64> {
        self.viewport.width
    }

    pub(in crate::native_bridge::element::styles) const fn viewport(self) -> StyleViewport {
        self.viewport
    }

    pub(in crate::native_bridge::element::styles) const fn with_read_document(
        mut self,
        read_document: Option<DomHandle>,
    ) -> Self {
        self.read_document = read_document;
        self
    }

    pub(super) fn resolved_read_document(
        self,
        runtime: &JsContextHost,
        handle: DomHandle,
    ) -> DomHandle {
        runtime
            .dom_host()
            .owner_document_handle(handle)
            .or(self.read_document)
            .unwrap_or_else(|| runtime.document_handle())
    }
}

/// One synchronous observation of the persistent style worlds it touches.
///
/// The observation drains pending style work once per Document and reads
/// canonical element styles from the retained world. A complete TreeScope
/// input is materialized only if that world must be updated or a specialized
/// query needs it, then shared by every read in this observation.
pub(crate) struct StyleObservation<'a> {
    runtime: &'a JsContextHost,
    /// `None` resolves one stable context per touched Document. Layout and
    /// detached CSSOM snapshots use `Some` to pin an exact caller-owned
    /// viewport instead.
    fixed_context: Option<StyleComputationContext>,
    /// Explicit layout observations may intentionally use a caller-owned
    /// viewport that differs from the Window surface. They advance the style
    /// viewport input epoch, but do not become the baseline for ordinary DOM
    /// API observations.
    tracks_persistent_world: bool,
    drained_document: Option<DomHandle>,
    additional_drained_documents: Vec<DomHandle>,
    primary_document: Option<Rc<StyleObservationInputs<'a>>>,
    additional_documents: Vec<Rc<StyleObservationInputs<'a>>>,
}

pub(super) struct StyleObservationInputs<'a> {
    runtime: &'a JsContextHost,
    key: StyleObservationKey,
    context: StyleComputationContext,
    /// Page-level media inputs are stable for one synchronous observation.
    /// In particular, discovering `<meta name=color-scheme>` requires a
    /// document-order query and must not be repeated for every layout node.
    environment: StyloStyleEnvironment,
    #[cfg(debug_assertions)]
    tracks_persistent_world: bool,
    prepared_update: RefCell<Option<Rc<PreparedStyleWorldUpdate>>>,
    stylesheet_query_snapshot: RefCell<Option<Rc<FullStyleWorldSnapshot>>>,
}

impl<'a> StyleObservationInputs<'a> {
    fn new(
        runtime: &'a JsContextHost,
        source_document: Option<DomHandle>,
        context: StyleComputationContext,
        _tracks_persistent_world: bool,
    ) -> Self {
        let environment = stylo_style_environment(runtime, source_document);
        Self {
            runtime,
            key: StyleObservationKey::for_document(runtime, source_document),
            context,
            environment,
            #[cfg(debug_assertions)]
            tracks_persistent_world: _tracks_persistent_world,
            prepared_update: RefCell::new(None),
            stylesheet_query_snapshot: RefCell::new(None),
        }
    }

    pub(super) fn source_document(&self) -> Option<DomHandle> {
        self.key.source_document()
    }

    fn tree_scope_versions(&self) -> crate::style_engine::StyleTreeScopeVersions {
        self.key.tree_scope_versions()
    }

    fn context(&self) -> StyleComputationContext {
        self.context
    }

    fn environment(&self) -> StyloStyleEnvironment {
        self.environment
    }

    fn prepared_update(&self, plan: &StyleWorldUpdatePlan) -> Rc<PreparedStyleWorldUpdate> {
        if let Some(update) = self.prepared_update.borrow().as_ref() {
            return Rc::clone(update);
        }
        let update = prepare_style_world_update(
            self.runtime,
            &self.key,
            self.context,
            self.environment,
            plan,
        );
        *self.prepared_update.borrow_mut() = Some(Rc::clone(&update));
        update
    }

    pub(super) fn stylesheet_query_snapshot(&self) -> Rc<FullStyleWorldSnapshot> {
        if let Some(inputs) = self.stylesheet_query_snapshot.borrow().as_ref() {
            return Rc::clone(inputs);
        }
        let inputs = self
            .source_document()
            .and_then(|document| {
                self.runtime
                    .retained_stylesheet_query_snapshot_for_document(document)
            })
            .unwrap_or_else(|| {
                stylesheet_query_fallback(self.runtime, &self.key, self.context, self.environment)
            });
        *self.stylesheet_query_snapshot.borrow_mut() = Some(Rc::clone(&inputs));
        inputs
    }
}

pub(super) trait RetainedStyleObservation {
    fn style_snapshot(&self, handle: DomHandle) -> Option<StyloComputedStyleSnapshot>;
}

impl RetainedStyleObservation for StyleObservationInputs<'_> {
    fn style_snapshot(&self, handle: DomHandle) -> Option<StyloComputedStyleSnapshot> {
        #[cfg(debug_assertions)]
        let invariant_input = self
            .tracks_persistent_world
            .then_some(self.source_document())
            .flatten()
            .map(|document| {
                (
                    document,
                    self.runtime.computed_style_observation_input_epochs(
                        document,
                        self.tree_scope_versions(),
                    ),
                )
            });
        let read_document = self.context.resolved_read_document(self.runtime, handle);
        let style = match self
            .runtime
            .computed_style_snapshot_from_current_observation(
                handle,
                read_document,
                self.context.viewport,
                self.environment,
                self.tree_scope_versions(),
            ) {
            StyleObservationSnapshot::Current(style) => style,
            StyleObservationSnapshot::NeedsStyleWorldUpdate(plan) => {
                let update = self.prepared_update(&plan);
                self.runtime.computed_style_snapshot_with_world_update(
                    handle,
                    update.as_ref(),
                    read_document,
                )
            }
        };
        #[cfg(debug_assertions)]
        if let Some((document, input_epochs)) = invariant_input {
            self.runtime
                .complete_computed_style_observation(document, &input_epochs);
        }
        style
    }
}

impl<'a> StyleObservation<'a> {
    pub(crate) fn new(runtime: &'a JsContextHost) -> Self {
        Self::new_with_fixed_context(runtime, None, true)
    }

    /// Creates one synchronous style-observation scope for an exact document
    /// viewport selected by layout.
    ///
    /// Embedded documents cannot derive their used viewport from the iframe's
    /// authored width/height alone: parent layout may change it through
    /// box-sizing, padding, constraints, flex/grid sizing, or transforms. The
    /// scoped context keeps viewport units and media queries aligned with the
    /// numeric layout demand without mutating retained document state.
    pub(crate) fn new_for_document_viewport(
        runtime: &'a JsContextHost,
        document: DomHandle,
        viewport: StyleViewport,
    ) -> Self {
        runtime.note_layout_style_viewport(document, viewport);
        Self::new_with_fixed_context(
            runtime,
            Some(StyleComputationContext::new(viewport).with_read_document(Some(document))),
            false,
        )
    }

    pub(super) fn new_with_context(
        runtime: &'a JsContextHost,
        context: StyleComputationContext,
    ) -> Self {
        Self::new_with_fixed_context(runtime, Some(context), true)
    }

    fn new_with_fixed_context(
        runtime: &'a JsContextHost,
        fixed_context: Option<StyleComputationContext>,
        tracks_persistent_world: bool,
    ) -> Self {
        Self {
            runtime,
            fixed_context,
            tracks_persistent_world,
            drained_document: None,
            additional_drained_documents: Vec::new(),
            primary_document: None,
            additional_documents: Vec::new(),
        }
    }

    pub(crate) const fn runtime(&self) -> &'a JsContextHost {
        self.runtime
    }

    pub(crate) fn read(&mut self, handle: DomHandle) -> ComputedStyleRead<'a> {
        let read_document = self
            .fixed_context
            .unwrap_or_default()
            .resolved_read_document(self.runtime, handle);
        let source_document = stylesheet_source_document_for_handle(self.runtime, handle);
        if self.drained_document != Some(read_document)
            && !self.additional_drained_documents.contains(&read_document)
        {
            if self.drained_document.is_none() {
                self.drained_document = Some(read_document);
            } else {
                self.additional_drained_documents.push(read_document);
            }
            self.runtime
                .drain_pending_style_invalidations_for_computed_style_read_for_document(
                    read_document,
                );
        }

        let observation_inputs = self.observation_inputs(source_document, read_document);
        let context = observation_inputs.context();
        #[cfg(debug_assertions)]
        let invariant_input = self
            .tracks_persistent_world
            .then_some(source_document)
            .flatten()
            .map(|document| {
                (
                    document,
                    self.runtime.computed_style_observation_input_epochs(
                        document,
                        observation_inputs.tree_scope_versions(),
                    ),
                )
            });
        let stylo_style = match self
            .runtime
            .computed_style_snapshot_from_current_observation(
                handle,
                read_document,
                context.viewport,
                observation_inputs.environment(),
                observation_inputs.tree_scope_versions(),
            ) {
            StyleObservationSnapshot::Current(style) => style,
            StyleObservationSnapshot::NeedsStyleWorldUpdate(plan) => {
                let prepared_update = observation_inputs.prepared_update(&plan);
                self.runtime.computed_style_snapshot_with_world_update(
                    handle,
                    prepared_update.as_ref(),
                    read_document,
                )
            }
        };
        #[cfg(debug_assertions)]
        if let Some((document, input_epochs)) = invariant_input {
            self.runtime
                .complete_computed_style_observation(document, &input_epochs);
        }
        ComputedStyleRead {
            runtime: self.runtime,
            handle,
            context,
            observation_inputs,
            stylo_style,
        }
    }

    fn observation_inputs(
        &mut self,
        source_document: Option<DomHandle>,
        read_document: DomHandle,
    ) -> Rc<StyleObservationInputs<'a>> {
        if let Some(observation) = self.primary_document.as_ref()
            && observation.source_document() == source_document
        {
            return Rc::clone(observation);
        }
        if let Some(observation) = self
            .additional_documents
            .iter()
            .find(|observation| observation.source_document() == source_document)
        {
            return Rc::clone(observation);
        }

        let context = self
            .fixed_context
            .map(|context| {
                if context.read_document.is_some() && context.read_document != Some(read_document) {
                    StyleComputationContext::new(style_viewport_for_document(
                        self.runtime,
                        read_document,
                    ))
                    .with_read_document(Some(read_document))
                } else {
                    context
                }
            })
            .unwrap_or_else(|| {
                StyleComputationContext::new(style_viewport_for_document(
                    self.runtime,
                    read_document,
                ))
            });
        let observation = Rc::new(StyleObservationInputs::new(
            self.runtime,
            source_document,
            context,
            self.tracks_persistent_world,
        ));
        if self.primary_document.is_none() {
            self.primary_document = Some(Rc::clone(&observation));
        } else {
            self.additional_documents.push(Rc::clone(&observation));
        }
        observation
    }
}

/// One synchronous computed-style read after the pending style lifecycle has
/// been drained. Chromium updates style once and lets all properties in the
/// read observe the same retained ComputedStyle; Moli follows that model with
/// canonical Stylo `ElementData` and a lazy observation-local slow path.
pub(crate) struct ComputedStyleRead<'a> {
    pub(super) runtime: &'a JsContextHost,
    pub(super) handle: DomHandle,
    pub(super) context: StyleComputationContext,
    pub(super) observation_inputs: Rc<StyleObservationInputs<'a>>,
    pub(super) stylo_style: Option<StyloComputedStyleSnapshot>,
}
