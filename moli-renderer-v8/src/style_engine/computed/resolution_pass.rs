use moli_selector::{StyloDomHostBinding, StyloElement};
use style::{
    Atom,
    animation::AnimationSetKey,
    computed_value_flags::ComputedValueFlags,
    context::{
        QuirksMode, RegisteredSpeculativePainter, RegisteredSpeculativePainters,
        SharedStyleContext, StyleContext, StyleSystemOptions, ThreadLocalStyleContext,
    },
    data::{ElementDataFlags, ElementStyles},
    dom::{TElement, TNode},
    invalidation::element::restyle_hints::RestyleHint,
    properties::ComputedValues,
    selector_parser::{PseudoElement, SnapshotMap},
    servo::restyle_damage::ServoRestyleDamage,
    servo_arc::Arc as ServoArc,
    shared_lock::StylesheetGuards,
    stylist::RuleInclusion,
    thread_state::{self, ThreadState},
    traversal::{resolve_style, restyle_element},
    traversal_flags::TraversalFlags,
};

#[cfg(test)]
use super::STYLE_RESOLUTION_CONTEXTS;
use super::{
    DocumentStyleWorld, DomHandle, DomHost, MarkerStyleDecision, MoliStyleEngine,
    StyloAnonymousBoxKind, StyloComputedStyleSnapshot, child_restyle_scope, document_quirks_mode,
    marker_style_decision, populate_inline_style_attribute_for_resolution, record_screen_restyle,
    stylo_pseudo_element_for_computed_style, with_cached_pseudo_style,
};

/// One binding and stylesheet read scope for a stable document world. Create
/// Stylo's thread-local state only when an element actually needs resolving.
pub(in crate::style_engine) struct ResolutionContext<'a, 'dom> {
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

    pub(in crate::style_engine) fn with_stylo<R>(
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
    with_resolution_context_and_snapshots(world, adapter, &SnapshotMap::new(), resolve)
}

pub(in crate::style_engine) fn with_resolution_context_and_snapshots<'dom, R>(
    world: &DocumentStyleWorld,
    adapter: &'dom StyloDomHostBinding<'dom>,
    snapshot_map: &SnapshotMap,
    resolve: impl FnOnce(ResolutionContext<'_, 'dom>) -> R,
) -> R {
    let guard = adapter.shared_lock().read();
    let empty_painters = EmptyRegisteredSpeculativePainters;
    let _layout_thread_state = StyloLayoutThreadStateGuard::enter();
    world.document_state.with_retained_style_system(|retained| {
        let animation_time = world.animation_time.get();
        let shared = SharedStyleContext {
            stylist: &retained.stylist,
            visited_styles_enabled: false,
            // Demand-driven passes can skip ancestors that a standard
            // traversal would have marked ineligible for cousin sharing.
            options: StyleSystemOptions {
                disable_style_sharing_cache: true,
                ..StyleSystemOptions::default()
            },
            guards: StylesheetGuards::same(&guard),
            current_time_for_animations: animation_time.unwrap_or_default(),
            traversal_flags: if animation_time.is_some() {
                TraversalFlags::empty()
            } else {
                TraversalFlags::SkipAnimationUpdates
            },
            snapshot_map,
            animations: world.animations.clone(),
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

/// Demand-driven style resolution within one document's stable style world.
/// Element styles are published in the existing Stylo side table. The pass
/// holds only traversal metadata and the working state needed to resolve them.
pub(crate) struct StyleResolutionPass<'a, 'dom> {
    engine: &'a MoliStyleEngine,
    host: &'a DomHost,
    document_url: &'a url::Url,
    quirks_mode: QuirksMode,
    context: ResolutionContext<'a, 'dom>,
    pending: Vec<StyloElement<'dom>>,
    validations: u64,
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
    with_style_resolution_pass_in_world(engine, host, document_url, &world, quirks_mode, resolve)
}

pub(super) fn with_style_resolution_pass_in_world<R>(
    engine: &MoliStyleEngine,
    host: &DomHost,
    document_url: &url::Url,
    world: &DocumentStyleWorld,
    quirks_mode: QuirksMode,
    resolve: impl FnOnce(&mut StyleResolutionPass<'_, '_>) -> R,
) -> R {
    engine.dom_adapter.with_bound_host(host, |adapter| {
        with_resolution_context(world, adapter, |context| {
            let mut pass = StyleResolutionPass {
                engine,
                host,
                document_url,
                quirks_mode,
                context,
                pending: Vec::new(),
                validations: 0,
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

    pub(crate) fn screen_style_coverage(&self) -> crate::style_engine::ScreenStyleCoverage {
        self.context.world.document_state.screen_style_coverage()
    }

    pub(crate) fn resolve_screen_target_for_reuse(
        &mut self,
        handle: DomHandle,
        after: u64,
        supported_damage: ServoRestyleDamage,
    ) -> Option<RestyleHint> {
        let state = &self.context.world.document_state;
        if !state.screen_style_dependencies_are_current() || state.screen_uses_lazy_pseudo(handle) {
            return None;
        }
        let element = self.prepare(handle)?;
        if !supported_damage.is_empty()
            && element
                .borrow_data()
                .is_some_and(|data| !supported_damage.contains(data.damage))
        {
            return None;
        }
        let invalidation = &state.style_paths;
        // Resolving the target can also restyle ancestors or retire old branch
        // observations. Either event can require a new screen layout.
        if state.screen_restyle_is_full()
            || (supported_damage.is_empty() && state.screen_needs_layout())
            || !invalidation.can_compare_after(after)
        {
            return None;
        }
        Some(
            invalidation
                .branch_requirements(handle)
                .child_hint_after(after),
        )
    }

    pub(crate) fn may_inherit_reset_style(&self, handle: DomHandle) -> bool {
        self.context
            .adapter
            .element(self.host, handle)
            .and_then(|element| {
                element.borrow_data()?.styles.get_primary().map(|style| {
                    style
                        .flags
                        .contains(ComputedValueFlags::INHERITS_RESET_STYLE)
                })
            })
            .unwrap_or(true)
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
        with_cached_pseudo_style(self.context.world, handle, pseudo, || {
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
            validated_elements = self.validations,
            resolved_elements = self.resolutions,
            cached_branch_requirements = self
                .context
                .world
                .document_state
                .style_paths
                .branch_requirement_count(),
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

    pub(super) fn prepare(&mut self, handle: DomHandle) -> Option<StyloElement<'dom>> {
        let document = self.document();
        if self.host.owner_document_handle(handle) != Some(document) {
            return None;
        }
        let element = self.context.adapter.element(self.host, handle)?;

        let world = self.context.world;
        let invalidation = &world.document_state.style_paths;
        let dom_adapter = &self.engine.dom_adapter;

        let mut current = Some(element);
        let mut parent = None;
        while let Some(ancestor) = current {
            let handle = DomHandle::new(ancestor.as_node().debug_id());
            invalidation.note_path_node_visit();
            if let Some(path) = dom_adapter.style_path(document, handle)
                && path.observed_at == invalidation.generation()
                && ancestor
                    .borrow_data()
                    .is_some_and(|data| data.has_styles() && data.hint.is_empty())
            {
                parent = Some((handle, path));
                break;
            }
            self.pending.push(ancestor);
            current = ancestor.traversal_parent();
        }

        while let Some(element) = self.pending.pop() {
            let handle = DomHandle::new(element.as_node().debug_id());
            let (depth, inherited_subtree, child_restyles) =
                parent.map_or((0, 0, Default::default()), |(parent, path)| {
                    (
                        path.depth() + 1,
                        path.subtree_generation,
                        invalidation.branch_requirements(parent),
                    )
                });
            let mut subtree_generation =
                inherited_subtree.max(invalidation.subtree_generation(handle));
            // The style-parent chain skips the Document and ShadowRoot.
            // Their subtree invalidations still apply at that boundary.
            if let Some(dom_parent) = self.host.parent_node(handle)
                && parent.is_none_or(|(style_parent, _)| style_parent != dom_parent)
            {
                subtree_generation =
                    subtree_generation.max(invalidation.subtree_generation(dom_parent));
            }
            let data = element.borrow_data();
            let inherits_reset = data
                .as_ref()
                .and_then(|data| data.styles.get_primary())
                .is_some_and(|style| {
                    style
                        .flags
                        .contains(ComputedValueFlags::INHERITS_RESET_STYLE)
                });
            let last_observation = dom_adapter
                .style_path(document, handle)
                .map_or(0, |path| path.observed_at);
            let mut hint = data.as_ref().map_or(RestyleHint::empty(), |data| data.hint);
            if subtree_generation > last_observation {
                hint.insert(RestyleHint::RESTYLE_SELF);
            }
            if child_restyles.child_requirement(inherits_reset) > last_observation {
                hint.insert(RestyleHint::RECASCADE_SELF);
            }
            let has_styles = data.is_some_and(|data| data.has_styles());
            let needs_resolution = !hint.is_empty() || !has_styles;

            if needs_resolution {
                populate_inline_style_attribute_for_resolution(
                    self.engine,
                    self.host,
                    self.document_url,
                    handle,
                    self.quirks_mode,
                );
                let mut data = unsafe { element.ensure_data() };
                data.hint = hint;
                let previous = data.styles.clone();
                let pending_damage = std::mem::take(&mut data.damage);
                if world.animation_time.get().is_some()
                    && let Some(animations) =
                        world
                            .animations
                            .sets
                            .write()
                            .get_mut(&AnimationSetKey::new_for_non_pseudo(
                                element.as_node().opaque(),
                            ))
                {
                    // Reinstall the animation cascade after restyles that can
                    // replace the base rule, including canceling an animation.
                    animations.dirty = true;
                }
                let child_hint = self.context.with_stylo(|context| {
                    context.thread_local.current_dom_depth = depth;
                    let mut child_hint =
                        data.restyle_kind(context.shared)
                            .map_or(RestyleHint::empty(), |kind| {
                                world.pseudo_style_cache.invalidate_handles([handle]);
                                self.resolutions += 1;
                                restyle_element(context, element, &mut data, kind)
                            });
                    child_hint |= data.hint.propagate(&context.shared.traversal_flags);
                    child_hint
                });
                let child_hint = if previous.get_primary().is_some() {
                    child_restyle_scope(child_hint)
                } else {
                    RestyleHint::recascade_subtree()
                };
                let damage = record_screen_restyle(world, element, &previous, &data, child_hint);
                data.flags.remove(ElementDataFlags::WAS_RESTYLED);
                data.damage = pending_damage | damage;
                if previous.get_primary().is_some() {
                    invalidation.record_child_requirements(handle, child_hint, subtree_generation);
                }
            }
            self.validations += 1;
            subtree_generation = subtree_generation.max(invalidation.subtree_generation(handle));
            let path = moli_selector::StyloStylePath::new(
                invalidation.generation(),
                subtree_generation,
                depth,
            );
            dom_adapter.set_style_path(document, handle, path);
            parent = Some((handle, path));
        }
        Some(element)
    }

    pub(super) fn eager_pseudo_styles_for_read(
        &mut self,
        element: StyloElement<'dom>,
        pseudo: &PseudoElement,
    ) -> ElementStyles {
        // CSSOM can query a pseudo that didn't generate a box. Keep its
        // forced resolution separate from the canonical element styles.
        self.context.with_stylo(|context| {
            resolve_style(context, element, RuleInclusion::All, Some(pseudo), None)
        })
    }
}

impl Drop for StyleResolutionPass<'_, '_> {
    fn drop(&mut self) {
        self.context
            .world
            .document_state
            .note_ancestor_style_validation_visits(self.validations);
        self.context
            .world
            .document_state
            .note_element_style_resolutions(self.resolutions);
    }
}

fn install_shadow_cascade_data_for_resolution(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::native::NativeDom;
    use crate::style_engine::{
        FullStyleWorldSnapshot, StyleViewport, StyleWorldKey, StyloStylesheetSource,
    };
    use style::context::ElementCascadeInputs;
    use style::invalidation::element::restyle_hints::RestyleHint;
    use style::matching::MatchMethods;
    use style::style_resolver::{PseudoElementResolution, StyleResolverForElement};
    use style_traits::ToCss;

    #[test]
    fn stylo_incremental_restyle_keeps_wide_inheritance_demand_driven() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url.clone()));
        let document = host.document_handle();
        let html = host.create_element("html");
        let parent = host.create_element("section");
        assert!(host.append_child(document, html));
        assert!(host.append_child(html, parent));
        let children: Vec<_> = (0..10_000)
            .map(|_| {
                let child = host.create_element("span");
                assert!(host.append_child(parent, child));
                child
            })
            .collect();

        let mut engine = MoliStyleEngine::new();
        let inputs = FullStyleWorldSnapshot::default();
        let key = StyleWorldKey::new(&inputs, StyleViewport::default());
        engine.ensure_retained_style_system_for_document(&host, document, key, &inputs);
        with_style_resolution_pass(&engine, &host, &url, document, |pass| {
            pass.prepare_targets(&children);
        });

        assert!(host.set_attribute(parent, "style", "color: rgb(1, 2, 3)"));
        engine.invalidate_inline_style_subtree(&host, parent);
        with_style_resolution_pass(&engine, &host, &url, document, |pass| {
            let child = pass.element_styles(children[0]).unwrap();
            assert_eq!(
                child
                    .primary
                    .get_inherited_text()
                    .clone_color()
                    .to_css_string(),
                "rgb(1, 2, 3)"
            );
            let stats = &pass.context.local.as_ref().unwrap().statistics;
            assert_eq!(stats.elements_styled, 2);
            assert_eq!(stats.elements_matched, 0);
            assert!(pass.context.adapter.bound_node_count() < 16);
        });

        let leaf = children[1];
        engine.invalidate_style_subtree(&host, leaf);
        with_style_resolution_pass(&engine, &host, &url, document, |pass| {
            assert!(pass.prepare(leaf).is_some());
            assert_eq!(
                pass.context
                    .local
                    .as_ref()
                    .unwrap()
                    .statistics
                    .elements_matched,
                1
            );
        });
    }

    #[test]
    fn stylo_root_pseudos_use_current_font_bases_on_match_and_recascade() {
        let url = url::Url::parse("https://example.test/").unwrap();
        let mut host = DomHost::from_dom(NativeDom::new_html(url.clone()));
        let document = host.document_handle();
        let html = host.create_element("html");
        assert!(host.append_child(document, html));

        let engine = MoliStyleEngine::new();
        let source = StyloStylesheetSource::new(
            r#"html { font: 20px/30px monospace; }
               html::before { content: ""; font: 7px/9px monospace; width: 2rch; height: 2rlh; }"#
                .into(),
            url.clone(),
        );
        let mut inputs = FullStyleWorldSnapshot::default();
        inputs.document_stylesheet_sources.push(source);
        let key = StyleWorldKey::new(&inputs, StyleViewport::default());
        engine.ensure_retained_style_system_for_document(&host, document, key, &inputs);
        let world = engine.world_for_document(document);

        for (replacement, width) in [(None, "20px"), (Some("font-family: Ahem"), "40px")] {
            if let Some(replacement) = replacement {
                assert!(host.set_attribute(html, "style", replacement));
                populate_inline_style_attribute_for_resolution(
                    &engine,
                    &host,
                    &url,
                    html,
                    document_quirks_mode(&host, document),
                );
            }

            engine.dom_adapter.with_bound_host(&host, |adapter| {
                let root = adapter.element(&host, html).unwrap();
                with_resolution_context(&world, adapter, |mut scope| {
                    scope.with_stylo(|context| {
                        let mut data = unsafe { root.ensure_data() };
                        let mut important_rules_changed = true;
                        let cascade_inputs = replacement.map(|_| {
                            let mut inputs = ElementCascadeInputs::new_from_element_data(&data);
                            important_rules_changed = root.replace_rules(
                                RestyleHint::RESTYLE_STYLE_ATTRIBUTE,
                                context,
                                &mut inputs,
                            );
                            inputs
                        });
                        let mut resolver = StyleResolverForElement::new(
                            root,
                            context,
                            RuleInclusion::All,
                            PseudoElementResolution::IfApplicable,
                        );
                        let styles = match cascade_inputs {
                            Some(inputs) => resolver.cascade_styles_with_default_parents(inputs),
                            None => resolver.resolve_style_with_default_parents(),
                        };
                        let before = styles.pseudos.get(&PseudoElement::Before).unwrap();
                        assert_eq!(before.get_position().width.to_css_string(), width);
                        assert_eq!(before.get_position().height.to_css_string(), "60px");

                        let child_hint = root.finish_restyle(
                            context,
                            &mut data,
                            styles,
                            important_rules_changed,
                        );
                        if replacement.is_some() {
                            assert!(
                                child_hint.contains(
                                    RestyleHint::RESTYLE_IF_AFFECTED_BY_WM_OR_ANCESTOR_FONT
                                )
                            );
                        }

                        let shared = context.shared;
                        let device = shared.stylist.device();
                        let published_font_size = device.root_font_size();
                        let published_ch = device.root_font_metrics_ch();
                        StyleResolverForElement::new(
                            root,
                            context,
                            RuleInclusion::DefaultOnly,
                            PseudoElementResolution::IfApplicable,
                        )
                        .resolve_style_with_default_parents();
                        assert_eq!(device.root_font_size(), published_font_size);
                        assert_eq!(device.root_font_metrics_ch(), published_ch);
                    });
                });
            });
        }
    }
}
