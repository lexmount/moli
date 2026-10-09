use std::collections::HashMap;

use moli_selector::{StyloDomHostBinding, StyloElement};
use style::{
    Atom,
    animation::DocumentAnimationSet,
    context::{
        QuirksMode, RegisteredSpeculativePainter, RegisteredSpeculativePainters,
        SharedStyleContext, StyleContext, StyleSystemOptions, ThreadLocalStyleContext,
    },
    dom::{TElement, TNode},
    properties::ComputedValues,
    selector_parser::{PseudoElement, SnapshotMap},
    servo_arc::Arc as ServoArc,
    shared_lock::StylesheetGuards,
    style_resolver::{PseudoElementResolution, StyleResolverForElement},
    stylist::RuleInclusion,
    thread_state::{self, ThreadState},
    traversal_flags::TraversalFlags,
};

#[cfg(test)]
use super::STYLE_RESOLUTION_CONTEXTS;
use super::{
    DocumentStyleWorld, DomHandle, DomHost, MarkerStyleDecision, MoliStyleEngine,
    StyloAnonymousBoxKind, StyloComputedStyleSnapshot, clear_element_styles_for_resolution,
    document_quirks_mode, element_style_is_current, marker_style_decision,
    populate_inline_style_attribute_for_resolution, publish_resolved_element_styles,
    style_validation_path_needs_resolution, stylo_pseudo_element_for_computed_style,
    with_cached_lazy_pseudo_style,
};

/// One binding and stylesheet read scope for a stable document world. Create
/// Stylo's thread-local state only when an element actually needs resolving.
pub(super) struct ResolutionContext<'a, 'dom> {
    world: &'a DocumentStyleWorld,
    adapter: &'dom StyloDomHostBinding<'dom>,
    shared: &'a SharedStyleContext<'a>,
    local: Option<ThreadLocalStyleContext<StyloElement<'dom>>>,
    shadow_cascade_installed: bool,
}

impl<'dom> ResolutionContext<'_, 'dom> {
    fn install_shadow_cascade(&mut self) {
        if !self.shadow_cascade_installed {
            install_shadow_cascade_data_for_resolution(self.world, self.adapter);
            self.shadow_cascade_installed = true;
        }
    }

    pub(super) fn with_stylo<R>(
        &mut self,
        resolve: impl FnOnce(&mut StyleContext<'_, StyloElement<'dom>>) -> R,
    ) -> R {
        self.install_shadow_cascade();
        let local = self.local.get_or_insert_with(|| {
            #[cfg(test)]
            STYLE_RESOLUTION_CONTEXTS.with(|count| count.set(count.get() + 1));
            let mut local = ThreadLocalStyleContext::new();
            local.selector_caches = self.world.document_state.take_selector_caches();
            local
        });
        resolve(&mut StyleContext {
            shared: self.shared,
            thread_local: local,
        })
    }

    pub(super) fn lazy_pseudo_style(
        &mut self,
        element: StyloElement<'dom>,
        pseudo: &PseudoElement,
        primary: &ComputedValues,
    ) -> Option<ServoArc<ComputedValues>> {
        self.install_shadow_cascade();
        self.shared.stylist.lazily_compute_pseudo_element_style(
            &self.shared.guards,
            element,
            pseudo,
            RuleInclusion::All,
            primary,
            false,
            None,
        )
    }
}

impl Drop for ResolutionContext<'_, '_> {
    fn drop(&mut self) {
        if let Some(local) = self.local.as_mut() {
            self.world
                .document_state
                .replace_selector_caches(std::mem::take(&mut local.selector_caches));
        }
    }
}

pub(super) fn with_resolution_context<'dom, R>(
    world: &DocumentStyleWorld,
    adapter: &'dom StyloDomHostBinding<'dom>,
    resolve: impl FnOnce(ResolutionContext<'_, 'dom>) -> R,
) -> R {
    let guard = adapter.shared_lock().read();
    let snapshot_map = SnapshotMap::new();
    let empty_painters = EmptyRegisteredSpeculativePainters;
    let _layout_thread_state = StyloLayoutThreadStateGuard::enter();
    world.document_state.with_retained_style_system(|retained| {
        let shared = SharedStyleContext {
            stylist: &retained.stylist,
            visited_styles_enabled: false,
            options: StyleSystemOptions::default(),
            guards: StylesheetGuards::same(&guard),
            current_time_for_animations: 0.0,
            traversal_flags: TraversalFlags::empty(),
            snapshot_map: &snapshot_map,
            animations: DocumentAnimationSet::default(),
            registered_speculative_painters: &empty_painters,
        };
        resolve(ResolutionContext {
            world,
            adapter,
            shared: &shared,
            local: None,
            shadow_cascade_installed: false,
        })
    })
}

#[derive(Clone, Copy)]
struct PreparedElement {
    depth: usize,
    inherited_change: bool,
}

/// Demand-driven style resolution within one document's stable style world.
/// Element styles are published in the existing Stylo side table. The pass
/// holds only traversal metadata and the working state needed to resolve them.
pub(crate) struct StyleResolutionPass<'a, 'dom> {
    engine: &'a MoliStyleEngine,
    host: &'a DomHost,
    document_url: &'a url::Url,
    quirks_mode: QuirksMode,
    context: ResolutionContext<'a, 'dom>,
    prepared: HashMap<DomHandle, PreparedElement>,
    pending: Vec<StyloElement<'dom>>,
    resolutions: u64,
}

pub(in crate::style_engine) fn with_style_resolution_pass<R>(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    document: DomHandle,
    resolve: impl FnOnce(&mut StyleResolutionPass<'_, '_>) -> R,
) -> R {
    let world = engine.world_for_document(document);
    let quirks_mode = document_quirks_mode(host, document);
    engine.dom_adapter.with_bound_host(host, |adapter| {
        with_resolution_context(&world, adapter, |context| {
            let mut pass = StyleResolutionPass {
                engine,
                host,
                document_url,
                quirks_mode,
                context,
                prepared: HashMap::new(),
                pending: Vec::new(),
                resolutions: 0,
            };
            resolve(&mut pass)
        })
    })
}

impl<'dom> StyleResolutionPass<'_, 'dom> {
    pub(crate) const fn document(&self) -> DomHandle {
        self.context.world.document
    }

    pub(crate) fn element_styles(
        &mut self,
        handle: DomHandle,
    ) -> Option<StyloComputedStyleSnapshot> {
        let element = self.prepare(handle)?;
        let data = element.borrow_data()?;
        Some(StyloComputedStyleSnapshot::from_element_styles(
            &data.styles,
        ))
    }

    #[cfg(feature = "devtools")]
    pub(crate) fn primary_style(&mut self, handle: DomHandle) -> Option<ServoArc<ComputedValues>> {
        let element = self.prepare(handle)?;
        let data = element.borrow_data()?;
        Some(data.styles.primary().clone())
    }

    pub(crate) fn pseudo_style(
        &mut self,
        handle: DomHandle,
        pseudo: &str,
    ) -> Option<ServoArc<ComputedValues>> {
        let pseudo = stylo_pseudo_element_for_computed_style(pseudo)?;
        let element = self.prepare(handle)?;
        let primary = element.borrow_data()?.styles.primary().clone();
        self.lazy_pseudo_style(handle, element, &pseudo, &primary)
    }

    pub(crate) fn marker_for_generation(
        &mut self,
        handle: DomHandle,
    ) -> Option<ServoArc<ComputedValues>> {
        let element = self.prepare(handle)?;
        let primary = element.borrow_data()?.styles.primary().clone();
        if !primary.clone_display().is_list_item() {
            return None;
        }
        let marker = self.lazy_pseudo_style(handle, element, &PseudoElement::Marker, &primary)?;
        (marker_style_decision(&primary, &marker) != MarkerStyleDecision::Suppressed)
            .then_some(marker)
    }

    fn lazy_pseudo_style(
        &mut self,
        handle: DomHandle,
        element: StyloElement<'dom>,
        pseudo: &PseudoElement,
        primary: &ComputedValues,
    ) -> Option<ServoArc<ComputedValues>> {
        debug_assert!(pseudo.is_lazy());
        with_cached_lazy_pseudo_style(self.context.world, handle, pseudo, || {
            self.context.lazy_pseudo_style(element, pseudo, primary)
        })
    }

    pub(crate) fn anonymous_style(
        &self,
        owner: DomHandle,
        parent: &ComputedValues,
        kind: StyloAnonymousBoxKind,
    ) -> Option<ServoArc<ComputedValues>> {
        if self.host.owner_document_handle(owner) != Some(self.document()) {
            return None;
        }
        Some(
            self.context
                .shared
                .stylist
                .style_for_anonymous::<StyloElement<'_>>(
                    &self.context.shared.guards,
                    &kind.pseudo(),
                    parent,
                ),
        )
    }

    pub(crate) fn trace_profile(&self) {
        tracing::info!(
            target: "moli_cpu_profile",
            stage = "style_resolution_pass",
            document = self.document().index_u32(),
            adapter_bindings = 1,
            thread_local_contexts = u8::from(self.context.local.is_some()),
            node_wrappers = self.context.adapter.bound_node_count(),
            validated_elements = self.prepared.len(),
            resolved_elements = self.resolutions,
        );
    }

    #[cfg(test)]
    pub(crate) const fn resolutions_for_test(&self) -> u64 {
        self.resolutions
    }

    pub(crate) fn prepare_targets(&mut self, handles: &[DomHandle]) {
        for &handle in handles {
            self.prepare(handle);
        }
    }

    fn prepare(&mut self, handle: DomHandle) -> Option<StyloElement<'dom>> {
        if self.host.owner_document_handle(handle) != Some(self.document()) {
            return None;
        }
        let element = self.context.adapter.element(self.host, handle)?;

        let mut current = Some(element);
        let mut depth = 0;
        let mut inherited_change = false;
        while let Some(ancestor) = current {
            let handle = DomHandle::new(ancestor.as_node().debug_id());
            if let Some(prepared) = self.prepared.get(&handle) {
                if ancestor == element {
                    return Some(element);
                }
                depth = prepared.depth + 1;
                inherited_change = prepared.inherited_change;
                break;
            }
            self.pending.push(ancestor);
            current = ancestor.traversal_parent();
        }

        let world = self.context.world;
        let invalidation = &world.document_state.lazy_invalidation_roots;
        while let Some(element) = self.pending.pop() {
            let handle = DomHandle::new(element.as_node().debug_id());
            let (required_generation, needs_resolution) = if let Some(generation) =
                invalidation.required_generation_for_checked_element(handle)
            {
                (
                    generation,
                    !element_style_is_current(world, element, generation),
                )
            } else {
                let path = invalidation.validation_path(self.host, world.document, handle);
                (
                    invalidation
                        .required_generation_for_checked_element(handle)
                        .unwrap_or_default(),
                    style_validation_path_needs_resolution(
                        world,
                        self.context.adapter,
                        self.host,
                        &path,
                    ),
                )
            };
            if inherited_change || needs_resolution {
                populate_inline_style_attribute_for_resolution(
                    self.engine,
                    self.host,
                    self.document_url,
                    handle,
                    self.quirks_mode,
                );
                clear_element_styles_for_resolution(world, element);
                let styles = self.context.with_stylo(|context| {
                    context
                        .thread_local
                        .bloom_filter
                        .insert_parents_recovering(element, depth);
                    context.thread_local.current_dom_depth = depth;
                    let styles = StyleResolverForElement::new(
                        element,
                        context,
                        RuleInclusion::All,
                        PseudoElementResolution::IfApplicable,
                    )
                    .resolve_style_with_default_parents()
                    .into();
                    context.thread_local.bloom_filter.push(element);
                    styles
                });
                publish_resolved_element_styles(world, element, styles, required_generation);
                inherited_change = true;
                self.resolutions += 1;
            }
            self.prepared.insert(
                handle,
                PreparedElement {
                    depth,
                    inherited_change,
                },
            );
            depth += 1;
        }
        Some(element)
    }
}

impl Drop for StyleResolutionPass<'_, '_> {
    fn drop(&mut self) {
        self.context
            .world
            .document_state
            .note_ancestor_style_validation_visits(self.prepared.len() as u64);
        self.context
            .world
            .document_state
            .note_element_style_resolutions(self.resolutions);
    }
}

pub(super) fn install_shadow_cascade_data_for_resolution(
    world: &DocumentStyleWorld,
    dom_adapter: &moli_selector::StyloDomHostBinding<'_>,
) {
    world
        .document_state
        .with_shadow_cascade_data(|shadow_cascade_data| {
            for (root, cascade_data) in shadow_cascade_data {
                dom_adapter.set_shadow_cascade_data(*root, cascade_data.clone());
            }
        });
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
