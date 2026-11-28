use dom::ElementState as StyloElementState;
use indexmap::IndexSet;
use moli_selector::{
    StyleMutationSnapshots, StyloDomStyleAdapter, stylo_focus_change_handles,
    stylo_focus_state_matches_handle,
};

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::cause::PendingStyleInvalidationCause;

pub(super) fn snapshot_state_change(
    host: &DomHost,
    dom_adapter: &StyloDomStyleAdapter,
    document: DomHandle,
    snapshots: &mut StyleMutationSnapshots,
    cause: PendingStyleInvalidationCause,
) {
    match cause {
        PendingStyleInvalidationCause::StateChange {
            element, old_state, ..
        } => {
            if let Some(old_state) = old_state {
                snapshots.record_state(element, old_state);
            }
        }
        PendingStyleInvalidationCause::CustomStateChange {
            element,
            old_custom_states,
        } => {
            snapshots.record_custom_states(element, old_custom_states);
        }
        PendingStyleInvalidationCause::FocusChange {
            previous,
            next,
            previous_focus_within,
        } => {
            let handles = stylo_focus_change_handles(host, previous, next)
                .chain(previous_focus_within.iter().copied())
                .chain(host.focus_within_ancestors(host.focus_within_root()))
                .collect::<IndexSet<_>>();
            dom_adapter.with_bound_host(host, |adapter| {
                for handle in handles {
                    if host.owner_document_handle(handle) != Some(document) {
                        continue;
                    }
                    if let Some(current) = adapter.computed_element_state(host, handle) {
                        snapshots.record_state(
                            handle,
                            old_focus_snapshot_state(
                                host,
                                handle,
                                previous,
                                &previous_focus_within,
                                current,
                            ),
                        );
                    }
                }
            });
        }
        PendingStyleInvalidationCause::TargetChange { previous, next } => {
            dom_adapter.with_bound_host(host, |adapter| {
                for handle in [previous, next].into_iter().flatten() {
                    if let Some(current) = adapter.computed_element_state(host, handle) {
                        snapshots.record_state(
                            handle,
                            old_target_snapshot_state(handle, previous, current),
                        );
                    }
                }
            });
        }
    }
}

fn old_focus_snapshot_state(
    host: &DomHost,
    handle: DomHandle,
    previous: Option<DomHandle>,
    previous_focus_within: &[DomHandle],
    mut state: StyloElementState,
) -> StyloElementState {
    state.remove(StyloElementState::FOCUS | StyloElementState::FOCUSRING);
    state.remove(StyloElementState::FOCUS_WITHIN);
    if previous.is_some_and(|previous| stylo_focus_state_matches_handle(host, previous, handle)) {
        state.insert(StyloElementState::FOCUS | StyloElementState::FOCUSRING);
    }
    if previous_focus_within.contains(&handle) {
        state.insert(StyloElementState::FOCUS_WITHIN);
    }
    state
}

fn old_target_snapshot_state(
    handle: DomHandle,
    previous: Option<DomHandle>,
    mut state: StyloElementState,
) -> StyloElementState {
    state.remove(StyloElementState::URLTARGET);
    if previous == Some(handle) {
        state.insert(StyloElementState::URLTARGET);
    }
    state
}
