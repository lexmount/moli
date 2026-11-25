use dom::ElementState;
use indexmap::IndexSet;
use moli_selector::{MoliStyleMutationSnapshot, StyloDomStyleAdapter};
use style::{
    LocalName, data::ElementData, dom::TElement, invalidation::element::restyle_hints::RestyleHint,
};

use crate::{
    document_runtime::DomHandle, dom::native::DomHost, protocol_types::EmulatedMediaOverrides,
};

use super::{
    StyleMutationEffect, StyleTreeScopeVersions, StyleViewport, StyloStyleEnvironment,
    cause::PendingStyleInvalidationCause,
    computed::{document_quirks_mode, with_resolution_context_and_snapshots},
    document_world::DocumentStyleWorld,
    snapshot::moli_style_mutation_snapshot_for_pending_cause,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct NativeSnapshotBatch {
    pub(super) snapshots: MoliStyleMutationSnapshot,
    former_parents: IndexSet<DomHandle>,
}

impl NativeSnapshotBatch {
    pub(super) fn merge_from(&mut self, incoming: Self) {
        self.snapshots.merge_from(incoming.snapshots);
        self.former_parents.extend(incoming.former_parents);
    }

    pub(super) fn fallback_roots(
        &self,
        world: &DocumentStyleWorld,
        host: &DomHost,
    ) -> Vec<DomHandle> {
        if !can_invalidate_native_snapshots_in_current_world(world, host)
            || self.former_parents.iter().any(|&parent| {
                parent != world.document
                    && host.owner_document_handle(parent) != Some(world.document)
            })
        {
            return vec![world.document];
        }
        // The supported dependencies can affect the source, its descendants,
        // and later siblings (including their descendants). Keep the former
        // parent so removing or reparenting the source doesn't miss those
        // siblings. Structural invalidation handles moved target subtrees.
        self.former_parents
            .iter()
            .copied()
            .chain(
                self.snapshots
                    .element_handles()
                    .filter(|&handle| host.owner_document_handle(handle) == Some(world.document))
                    .map(|handle| host.parent_node(handle).unwrap_or(handle)),
            )
            .collect()
    }
}

pub(super) fn snapshots_for_native_invalidation(
    dom_adapter: &StyloDomStyleAdapter,
    world: &DocumentStyleWorld,
    host: &DomHost,
    cause: &PendingStyleInvalidationCause,
    emulated_media: &EmulatedMediaOverrides,
    viewport: StyleViewport,
) -> Option<NativeSnapshotBatch> {
    let snapshot_cause = match cause {
        PendingStyleInvalidationCause::Mutation(effects) => {
            !effects.is_empty()
                && effects.iter().all(|effect| {
                    matches!(effect, StyleMutationEffect::Attribute { name, namespace: None, .. }
                    if name == "id" || name == "class"
                        || name.starts_with("aria-") || name.starts_with("data-"))
                })
        }
        // Selection and checkedness can also change an owning control or
        // container's validity. These setters currently snapshot only the
        // elements whose selectedness or checkedness changed.
        PendingStyleInvalidationCause::StateChange { state, .. } => {
            !(state.contains(ElementState::CHECKED)
                && state.intersects(ElementState::VALIDITY_STATES))
        }
        PendingStyleInvalidationCause::CustomStateChange { .. }
        | PendingStyleInvalidationCause::FocusChange { .. }
        | PendingStyleInvalidationCause::TargetChange { .. } => true,
    };
    if !snapshot_cause
        || !supports_native_snapshots(world, host)
        || !world
            .document_state
            .retained_style_system_is_current_for_observation(
                viewport,
                StyloStyleEnvironment::from_emulated_media(emulated_media),
                document_quirks_mode(host, world.document),
                StyleTreeScopeVersions::current(host, Some(world.document)),
            )
    {
        return None;
    }
    if let PendingStyleInvalidationCause::Mutation(effects) = cause
        && world
            .document_state
            .try_with_retained_style_system(|retained| {
                effects.iter().all(|effect| {
                    let StyleMutationEffect::Attribute { name, .. } = effect else {
                        return false;
                    };
                    if matches!(name.as_str(), "id" | "class") {
                        return false;
                    }
                    let name = LocalName::from(name.as_str());
                    !retained
                        .stylist
                        .iter_origins()
                        .any(|(cascade, _)| cascade.might_have_attribute_dependency(&name))
                })
            })
            == Some(true)
    {
        // Non-selector attr() dependencies were already handled by the host.
        return Some(NativeSnapshotBatch::default());
    }
    let snapshots = moli_style_mutation_snapshot_for_pending_cause(host, dom_adapter, cause)?;
    if !snapshot_roots_are_current(world, host, &snapshots) {
        return None;
    }
    let former_parents = snapshots
        .element_handles()
        .map(|handle| host.parent_node(handle).unwrap_or(handle))
        .collect();
    Some(NativeSnapshotBatch {
        snapshots,
        former_parents,
    })
}

fn snapshot_roots_are_current(
    world: &DocumentStyleWorld,
    host: &DomHost,
    snapshots: &MoliStyleMutationSnapshot,
) -> bool {
    snapshots.element_handles().all(|handle| {
        host.owner_document_handle(handle) == Some(world.document) && host.is_connected(handle)
    })
}

fn supports_native_snapshots(world: &DocumentStyleWorld, host: &DomHost) -> bool {
    // Document rules can reach shadow content through ::part even when there
    // are no author stylesheets attached to a shadow root.
    !host.has_shadow_roots()
        && world
            .document_state
            .try_with_retained_style_system(|retained| {
                // Relative selectors, nth-child(of S), and shadow boundaries have
                // extra host invalidation steps beyond StateAndAttr.
                retained.stylist.iter_origins().all(|(cascade, _)| {
                    !cascade.relative_invalidation_map_attributes().used
                        && !cascade.has_nth_of_dependencies()
                })
            })
            .unwrap_or(false)
}

fn can_invalidate_native_snapshots_in_current_world(
    world: &DocumentStyleWorld,
    host: &DomHost,
) -> bool {
    !world
        .document_state
        .source_dirty_scope_snapshot()
        .requires_retained_style_update()
        && supports_native_snapshots(world, host)
}

pub(super) fn native_snapshot_invalidation_targets(
    dom_adapter: &StyloDomStyleAdapter,
    world: &DocumentStyleWorld,
    host: &DomHost,
    snapshots: &MoliStyleMutationSnapshot,
) -> Option<Vec<(DomHandle, RestyleHint)>> {
    if !can_invalidate_native_snapshots_in_current_world(world, host)
        || !snapshot_roots_are_current(world, host, snapshots)
    {
        return None;
    }

    dom_adapter.with_bound_host(host, |adapter| {
        adapter.collect_snapshot_invalidations(host, snapshots, |map, handles| {
            with_resolution_context_and_snapshots(world, adapter, map, |mut resolution| {
                resolution.with_stylo(|context| {
                    for &handle in handles {
                        let element = adapter.element(host, handle).expect("snapshot element");
                        // An unstyled source can affect styled siblings. The
                        // native processor only needs local scratch data here.
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
                });
            });
        })
    })
}
