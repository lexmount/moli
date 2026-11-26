use dom::ElementState;
use indexmap::{IndexMap, IndexSet};
use moli_selector::{
    StyleMutationSnapshots, StyloDomHostBinding, StyloElement,
    attribute_affects_presentational_hints, html_auto_directionality_root,
};
use selectors::{Element, matching::ElementSelectorFlags};
use style::{
    LocalName,
    data::ElementData,
    dom::{TElement, TNode},
    invalidation::element::{
        invalidator::SiblingTraversalMap,
        relative_selector::{DomMutationOperation, RelativeSelectorInvalidator},
        state_and_attributes::invalidate_snapshot_dependencies,
    },
};

use crate::{
    document_runtime::DomHandle,
    dom::native::{DomHost, Node},
};

use super::{
    MoliStyleEngine, StyleMutationEffect, computed::with_resolution_context_and_snapshots,
    document_world::DocumentStyleWorld, mutation_effect::retain_live_structural_mutation_effects,
};

#[derive(Default)]
pub(super) struct PendingNativeInvalidation {
    pub(super) snapshots: StyleMutationSnapshots,
    original_parents: IndexSet<DomHandle>,
    structures: Vec<StyleMutationEffect>,
    connected_roots: IndexSet<DomHandle>,
    detached_roots: IndexSet<DomHandle>,
    deferred_child_list_parents: IndexSet<DomHandle>,
    #[cfg(test)]
    structural_effect_count: usize,
    subtrees: IndexSet<DomHandle>,
    states_without_snapshots: IndexMap<DomHandle, ElementState>,
}

impl PendingNativeInvalidation {
    pub(super) fn extend_snapshots(&mut self, host: &DomHost, snapshots: StyleMutationSnapshots) {
        self.original_parents.extend(
            snapshots
                .handles()
                .map(|handle| host.parent_node(handle).unwrap_or(handle)),
        );
        self.snapshots.merge(snapshots);
    }

    pub(super) fn record_mutations(
        &mut self,
        world: &DocumentStyleWorld,
        host: &DomHost,
        effects: &[StyleMutationEffect],
        defer_structural_cleanup: bool,
    ) {
        let mut snapshots = StyleMutationSnapshots::default();
        for effect in effects {
            if defer_structural_cleanup {
                self.detached_roots
                    .extend(effect.detached_roots().iter().copied());
                if let StyleMutationEffect::ChildList { parent, .. } = effect {
                    self.deferred_child_list_parents.insert(*parent);
                }
            }
            #[cfg(test)]
            if !matches!(
                effect,
                StyleMutationEffect::Attribute { .. } | StyleMutationEffect::CharacterData { .. }
            ) {
                self.structural_effect_count += 1;
            }
            match effect {
                StyleMutationEffect::Attribute {
                    element,
                    namespace,
                    name,
                    old_value,
                    new_value,
                } => {
                    if old_value == new_value {
                        continue;
                    }
                    let style_changes =
                        attribute_needs_subtree_restyle(host, *element, namespace.as_deref(), name);
                    if style_changes {
                        self.subtrees.insert(*element);
                    }
                    if namespace.is_none() {
                        // Reassigning a part name changes which outer rules
                        // enter the cascade, even without an attribute selector.
                        match name.as_str() {
                            "part" if host.containing_shadow_root(*element).is_some() => {
                                self.subtrees.insert(*element);
                            }
                            "exportparts" => {
                                self.subtrees.extend(host.shadow_root_handle(*element));
                            }
                            _ => {}
                        }
                    }
                    if style_changes
                        || matches!(name.as_str(), "class" | "id" | "href")
                        || host.has_shadow_roots()
                        || world
                            .document_state
                            .source_dirty_scope_snapshot()
                            .requires_retained_style_update()
                        || world
                            .document_state
                            .try_with_retained_style_system(|retained| {
                                let name = LocalName::from(name.as_str());
                                retained
                                    .stylist
                                    .iter_origins()
                                    .any(|(data, _)| data.might_have_attribute_dependency(&name))
                            })
                            != Some(false)
                    {
                        snapshots.record_attribute(
                            *element,
                            namespace.as_deref(),
                            name,
                            old_value.clone(),
                        );
                    }
                }
                StyleMutationEffect::ConnectedSubtrees { roots } => {
                    self.connected_roots.extend(roots.iter().copied());
                }
                StyleMutationEffect::DisconnectedSubtrees { .. } => {}
                effect => self.structures.push(effect.clone()),
            }
        }
        self.extend_snapshots(host, snapshots);
    }

    pub(super) fn record_state_without_snapshot(
        &mut self,
        host: &DomHost,
        element: DomHandle,
        state: ElementState,
    ) {
        self.subtrees
            .insert(host.parent_node(element).unwrap_or(element));
        self.states_without_snapshots
            .entry(element)
            .or_insert(ElementState::empty())
            .insert(state);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.snapshots.is_empty()
            && self.structures.is_empty()
            && self.connected_roots.is_empty()
            && self.detached_roots.is_empty()
            && self.subtrees.is_empty()
    }

    #[cfg(test)]
    pub(super) fn structural_effect_count_for_test(&self) -> usize {
        self.structural_effect_count
    }

    pub(super) fn drain(
        mut self,
        engine: &MoliStyleEngine,
        world: &DocumentStyleWorld,
        host: &DomHost,
    ) {
        if self.is_empty() {
            return;
        }

        // Retire removed roots before dropping history for consumed fragments.
        engine.invalidate_detached_style_subtrees(host, self.detached_roots);
        let cleanup = engine.invalidation_cleanup_for_world(world);
        cleanup.invalidate_subtrees(
            host,
            self.deferred_child_list_parents
                .into_iter()
                .filter_map(|parent| html_auto_directionality_root(host, parent)),
        );
        retain_live_structural_mutation_effects(host, &mut self.structures);

        let dom_adapter = &engine.dom_adapter;
        let mut subtrees = self.subtrees;
        let has_relative_selectors =
            |data: &style::stylist::CascadeData| data.relative_invalidation_map_attributes().used;
        let Some((relative_selectors, extra_snapshot_dependencies)) = world
            .document_state
            .try_with_retained_style_system(|retained| {
                let relative = retained
                    .stylist
                    .iter_origins()
                    .any(|(data, _)| has_relative_selectors(data))
                    || retained
                        .shadow_cascade_data
                        .iter()
                        .any(|(_, data)| has_relative_selectors(data));
                let snapshots = relative
                    || !retained.shadow_cascade_data.is_empty()
                    || retained
                        .stylist
                        .iter_origins()
                        .any(|(data, _)| data.has_nth_of_dependencies());
                (relative, snapshots)
            })
        else {
            subtrees.extend(self.original_parents);
            subtrees.extend(self.structures.iter().filter_map(mutation_root));
            cleanup.invalidate_subtrees(host, subtrees);
            return;
        };

        let replay_removal = self.snapshots.is_empty()
            && self
                .structures
                .iter()
                .filter(|effect| matches!(effect, StyleMutationEffect::ChildList { .. }))
                .take(2)
                .count()
                == 1;

        let splice_fallbacks = if relative_selectors {
            relative_splice_fallback_parents(host, &self.structures)
        } else {
            IndexSet::new()
        };

        let targets = dom_adapter.with_bound_host(host, |adapter| {
            adapter.with_mutation_snapshots(host, &self.snapshots, |map, handles| {
                with_resolution_context_and_snapshots(world, adapter, map, |mut resolution| {
                    resolution.with_stylo(|context| {
                        let stylist = context.shared.stylist;
                        for &handle in handles {
                            if host.owner_document_handle(handle) != Some(world.document) {
                                continue;
                            }
                            let Some(element) = adapter.element(host, handle) else {
                                continue;
                            };
                            if extra_snapshot_dependencies {
                                invalidate_snapshot_dependencies(stylist, map, element);
                            }
                            // An unstyled source can still affect a retained sibling.
                            let mut scratch = ElementData::default();
                            let mut data = element.mutate_data();
                            data.as_deref_mut()
                                .unwrap_or(&mut scratch)
                                .invalidate_style_if_needed(
                                    element,
                                    context.shared,
                                    Some(&context.thread_local.stack_limit_checker),
                                    &mut context.thread_local.selector_caches,
                                );
                        }

                        if relative_selectors {
                            for (handle, state) in self.states_without_snapshots {
                                if host.owner_document_handle(handle) == Some(world.document)
                                    && let Some(element) = adapter.element(host, handle)
                                {
                                    invalidate_relative_state(stylist, element, state);
                                }
                            }
                        }

                        for &parent in &splice_fallbacks {
                            subtrees.insert(parent);
                            invalidate_relative_boundaries(host, adapter, parent, &mut subtrees);
                        }
                        for mutation in &self.structures {
                            let replay_relative = relative_selectors
                                && !mutation_root(mutation)
                                    .is_some_and(|root| splice_fallbacks.contains(&root));
                            invalidate_structure(
                                host,
                                adapter,
                                replay_relative.then_some(stylist),
                                replay_removal,
                                mutation,
                                &mut subtrees,
                            );
                        }
                    });
                });
            })
        });
        cleanup.retain_native_invalidation_targets(host, targets);
        subtrees.extend(self.connected_roots);
        cleanup.invalidate_subtrees(host, subtrees);
    }
}

/// A series of splices can hide an original adjacent pair from the single-node
/// replay. Recheck the affected parent once when its history includes more
/// than one splice and at least one is a removal or an insertion in the middle.
fn relative_splice_fallback_parents(
    host: &DomHost,
    structures: &[StyleMutationEffect],
) -> IndexSet<DomHandle> {
    let mut parents = IndexMap::new();
    for effect in structures {
        if let StyleMutationEffect::ChildList {
            parent,
            added_nodes,
            removed_nodes,
            next_sibling,
            ..
        } = effect
            && added_nodes
                .iter()
                .chain(removed_nodes)
                .any(|&node| host.node(node).is_some_and(Node::is_element))
        {
            let append = next_sibling.is_none() && removed_nodes.is_empty();
            parents
                .entry(*parent)
                .and_modify(|(single, append_only)| {
                    *single = false;
                    *append_only &= append;
                })
                .or_insert((true, append));
        }
    }
    parents
        .into_iter()
        .filter_map(|(parent, (single, append_only))| (!single && !append_only).then_some(parent))
        .collect()
}

fn mutation_root(effect: &StyleMutationEffect) -> Option<DomHandle> {
    match effect {
        StyleMutationEffect::ChildList { parent, .. } => Some(*parent),
        StyleMutationEffect::SlotAssignment { slot, .. } => Some(*slot),
        _ => None,
    }
}

fn attribute_needs_subtree_restyle(
    host: &DomHost,
    element: DomHandle,
    namespace: Option<&str>,
    name: &str,
) -> bool {
    match (namespace, name) {
        // Language matching reads xml:lang and also accepts other namespaces.
        (_, "lang") | (None, "dir" | "style") => true,
        // Display size changes which option can act as a select's placeholder.
        (None, "size") => host.is_html_element_named(element, "select"),
        // Input type can change direction hints and submit controls' :default
        // state depends on type without using an attribute selector.
        (None, "type") => {
            host.is_html_element_named(element, "input")
                || host.is_html_element_named(element, "button")
        }
        _ => attribute_affects_presentational_hints(host, element, namespace, name),
    }
}

fn invalidate_structure(
    host: &DomHost,
    adapter: &StyloDomHostBinding<'_>,
    stylist: Option<&style::stylist::Stylist>,
    replay_removal: bool,
    mutation: &StyleMutationEffect,
    subtrees: &mut IndexSet<DomHandle>,
) {
    use ElementSelectorFlags as Flags;

    match mutation {
        StyleMutationEffect::ChildList {
            parent,
            added_nodes,
            removed_nodes,
            previous_sibling,
            next_sibling,
        } => {
            let flags = adapter.selector_flags(host, *parent);
            if flags.contains(Flags::HAS_EMPTY_SELECTOR) {
                subtrees.insert(*parent);
                subtrees.extend(following_sibling_subtrees(host, *parent));
                invalidate_relative_boundaries(host, adapter, *parent, subtrees);
            }
            if added_nodes
                .iter()
                .chain(removed_nodes)
                .any(|&node| host.node(node).is_some_and(Node::is_element))
            {
                invalidate_child_positions(
                    host,
                    *parent,
                    flags,
                    *previous_sibling,
                    *next_sibling,
                    subtrees,
                );
            }
            let Some(stylist) = stylist else {
                return;
            };
            if next_sibling.is_some()
                && added_nodes
                    .iter()
                    .filter(|&&node| host.node(node).is_some_and(Node::is_element))
                    .nth(1)
                    .is_some()
            {
                // A fragment can break the old adjacency across the entire
                // splice. No individual inserted node can replay that pair.
                subtrees.insert(*parent);
                invalidate_relative_boundaries(host, adapter, *parent, subtrees);
                return;
            }
            for &node in added_nodes {
                if host.parent_node(node) == Some(*parent)
                    && let Some(element) = adapter.element(host, node)
                {
                    let operation = if next_sibling.is_none() {
                        DomMutationOperation::Append
                    } else {
                        DomMutationOperation::Insert
                    };
                    invalidate_relative_splice(stylist, element, operation);
                }
            }

            if !removed_nodes.is_empty() {
                let previous = adjacent_element(host, *previous_sibling, |host, node| {
                    host.node(node).and_then(Node::prev_sibling)
                });
                let next = adjacent_element(host, *next_sibling, DomHost::next_sibling);
                // A single detached node still carries the old subtree and
                // the splice supplies its former DOM position.
                let replayed_removal = if replay_removal
                    && let [removed] = removed_nodes.as_slice()
                    && added_nodes.is_empty()
                    && previous.map_or_else(
                        || {
                            host.node(*parent)
                                .and_then(Node::first_child)
                                .and_then(|child| {
                                    adjacent_element(host, Some(child), DomHost::next_sibling)
                                })
                        },
                        |previous| {
                            adjacent_element(
                                host,
                                host.next_sibling(previous),
                                DomHost::next_sibling,
                            )
                        },
                    ) == next
                {
                    adapter
                        .with_removed_element_position(
                            host,
                            *removed,
                            *parent,
                            previous,
                            next,
                            |element| {
                                invalidate_relative_splice(
                                    stylist,
                                    element,
                                    DomMutationOperation::Remove,
                                );
                            },
                        )
                        .is_some()
                } else {
                    false
                };
                if !replayed_removal {
                    // A coalesced replacement can omit the old intermediate
                    // adjacency. Cover its affected siblings and
                    // any observed relative anchors outside the splice.
                    subtrees.insert(*parent);
                    invalidate_relative_boundaries(host, adapter, *parent, subtrees);
                }
            }
        }
        StyleMutationEffect::CharacterData { node } => {
            let Some(parent) = host.parent_node(*node) else {
                return;
            };
            if adapter
                .element(host, parent)
                .is_some_and(|element| element.has_selector_flags(Flags::HAS_EMPTY_SELECTOR))
            {
                subtrees.insert(parent);
                subtrees.extend(following_sibling_subtrees(host, parent));
                invalidate_relative_boundaries(host, adapter, parent, subtrees);
            }
        }
        StyleMutationEffect::SlotAssignment {
            slot,
            previous_assigned_nodes,
            assigned_nodes,
        } => {
            if let (Some(previous), Some(current)) = (previous_assigned_nodes, assigned_nodes) {
                if previous != current {
                    subtrees.extend(previous.iter().chain(current).copied());
                    subtrees.insert(*slot);
                }
            } else if let Some(host_element) = host
                .containing_shadow_root(*slot)
                .and_then(|root| host.shadow_root_host(root))
            {
                subtrees.insert(host_element);
            }
        }
        _ => {}
    }
}

fn invalidate_child_positions(
    host: &DomHost,
    parent: DomHandle,
    flags: ElementSelectorFlags,
    previous: Option<DomHandle>,
    next: Option<DomHandle>,
    subtrees: &mut IndexSet<DomHandle>,
) {
    use ElementSelectorFlags as Flags;
    let previous = adjacent_element(host, previous, |host, node| {
        host.node(node).and_then(Node::prev_sibling)
    });
    let next = adjacent_element(host, next, DomHost::next_sibling);
    if flags.intersects(Flags::HAS_SLOW_SELECTOR | Flags::MAY_HAVE_TREE_COUNTING_FUNCTION) {
        // nth-last-child and sibling-count() can reach earlier children.
        subtrees.insert(parent);
        subtrees.extend(previous);
        subtrees.extend(next);
    } else if flags.intersects(Flags::HAS_SLOW_SELECTOR_NTH | Flags::HAS_SLOW_SELECTOR_NTH_OF) {
        if let Some(next) = next {
            subtrees.insert(next);
            subtrees.extend(following_sibling_subtrees(host, next));
        }
    } else if flags.contains(Flags::HAS_SLOW_SELECTOR_LATER_SIBLINGS) {
        // Always dirty the immediate splice boundary. A sibling chain can
        // also affect later targets, so cover the rest lazily.
        if let Some(next) = next {
            subtrees.insert(next);
        }
        if next.is_some_and(|next| host.next_sibling(next).is_some()) {
            subtrees.insert(parent);
        }
    }

    if flags.contains(Flags::HAS_EDGE_CHILD_SELECTOR) {
        if previous.is_none() {
            subtrees.extend(next);
        }
        if next.is_none() {
            subtrees.extend(previous);
        }
    }
}

fn adjacent_element(
    host: &DomHost,
    start: Option<DomHandle>,
    step: impl Fn(&DomHost, DomHandle) -> Option<DomHandle>,
) -> Option<DomHandle> {
    std::iter::successors(start, |&handle| step(host, handle))
        .find(|&node| host.node(node).is_some_and(Node::is_element))
}

fn invalidate_relative_state(
    stylist: &style::stylist::Stylist,
    element: StyloElement<'_>,
    state: ElementState,
) {
    RelativeSelectorInvalidator {
        element,
        quirks_mode: stylist.quirks_mode(),
        snapshot_table: None,
        invalidated: |_, _| {},
        affected: None,
        sibling_traversal_map: SiblingTraversalMap::default(),
        _marker: std::marker::PhantomData,
    }
    .invalidate_relative_selectors_for_this(
        stylist,
        |element, host, data, quirks, collector| {
            // Aggregate states can change during a splice before the host can
            // observe their old bits. Consider every matching state dependency.
            data.relative_selector_invalidation_map()
                .state_affecting_selectors
                .lookup_with_additional(*element, quirks, None, &[], state, |dependency| {
                    if dependency.state.intersects(state) {
                        collector.add_dependency(&dependency.dep, *element, host);
                    }
                    true
                });
        },
    );
}

fn invalidate_relative_splice(
    stylist: &style::stylist::Stylist,
    element: StyloElement<'_>,
    operation: DomMutationOperation,
) {
    let inherited = inherited_search_direction(element);
    if inherited.is_empty() {
        return;
    }
    let previous = element.prev_sibling_element();
    let next = element.next_sibling_element();
    let invalidate = |element, subtree, operation, sibling_traversal_map| {
        RelativeSelectorInvalidator {
            element,
            quirks_mode: stylist.quirks_mode(),
            snapshot_table: None,
            invalidated: |_, _| {},
            affected: None,
            sibling_traversal_map,
            _marker: std::marker::PhantomData,
        }
        .invalidate_relative_selectors_for_dom_mutation(subtree, stylist, inherited, operation);
    };

    // The adjacent pair can gain or lose a match independently of the
    // inserted/removed element. Compare it with the splice node absent.
    if !matches!(operation, DomMutationOperation::Append)
        && let (Some(previous), Some(next)) = (previous, next)
    {
        invalidate(
            previous,
            false,
            DomMutationOperation::SideEffectPrevSibling,
            SiblingTraversalMap::new(previous, previous.prev_sibling_element(), Some(next)),
        );
        invalidate(
            next,
            false,
            DomMutationOperation::SideEffectNextSibling,
            SiblingTraversalMap::new(next, Some(previous), next.next_sibling_element()),
        );
    }
    invalidate(element, true, operation, SiblingTraversalMap::default());
}

fn inherited_search_direction(element: StyloElement<'_>) -> ElementSelectorFlags {
    use ElementSelectorFlags as Flags;
    let mut direction = Flags::empty();
    let mut parent = element.as_node().parent_element_or_host();
    while let Some(ancestor) = parent {
        if has_any_flags(
            ancestor,
            Flags::ANCHORS_RELATIVE_SELECTOR | Flags::ANCHORS_RELATIVE_SELECTOR_NON_SUBJECT,
        ) {
            direction.insert(Flags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR);
        }
        direction |= ancestor.relative_selector_search_direction();
        if !direction.is_empty() {
            break;
        }
        parent = ancestor.as_node().parent_element_or_host();
    }
    if let Some(previous) = element.prev_sibling_element() {
        if has_any_flags(
            previous,
            Flags::ANCHORS_RELATIVE_SELECTOR | Flags::ANCHORS_RELATIVE_SELECTOR_NON_SUBJECT,
        ) {
            direction.insert(Flags::RELATIVE_SELECTOR_SEARCH_DIRECTION_SIBLING);
        }
        direction |= previous.relative_selector_search_direction()
            & Flags::RELATIVE_SELECTOR_SEARCH_DIRECTION_SIBLING;
    }
    direction
}

fn invalidate_relative_boundaries(
    host: &DomHost,
    adapter: &StyloDomHostBinding<'_>,
    parent: DomHandle,
    subtrees: &mut IndexSet<DomHandle>,
) {
    use ElementSelectorFlags as Flags;

    let mut current = Some(parent);
    while let Some(handle) = current {
        if let Some(element) = adapter.element(host, handle) {
            let reaches_other_subjects =
                element.has_selector_flags(Flags::ANCHORS_RELATIVE_SELECTOR_NON_SUBJECT);
            if reaches_other_subjects
                || element.has_selector_flags(Flags::ANCHORS_RELATIVE_SELECTOR)
            {
                subtrees.insert(handle);
            }
            if reaches_other_subjects
                && host.parent_node(handle).is_some_and(|parent| {
                    adapter
                        .selector_flags(host, parent)
                        .contains(Flags::HAS_SLOW_SELECTOR_LATER_SIBLINGS)
                })
            {
                subtrees.extend(following_sibling_subtrees(host, handle));
            }
            if element
                .relative_selector_search_direction()
                .contains(Flags::RELATIVE_SELECTOR_SEARCH_DIRECTION_SIBLING)
                && let Some(parent) = host.parent_node(handle)
            {
                subtrees.insert(parent);
            }
        }
        current = host
            .parent_node(handle)
            .or_else(|| host.shadow_root_host(handle));
    }
}

fn following_sibling_subtrees(
    host: &DomHost,
    element: DomHandle,
) -> impl Iterator<Item = DomHandle> + '_ {
    std::iter::successors(host.next_sibling(element), |&node| host.next_sibling(node))
        .filter(|&node| host.node(node).is_some_and(Node::is_element))
}

pub(super) fn style_attribute_in_selector(stylist: &style::stylist::Stylist) -> bool {
    let style = LocalName::from("style");
    stylist
        .iter_origins()
        .any(|(data, _)| data.might_have_attribute_dependency(&style))
}

fn has_any_flags(element: StyloElement<'_>, flags: ElementSelectorFlags) -> bool {
    flags.iter().any(|flag| element.has_selector_flags(flag))
}
