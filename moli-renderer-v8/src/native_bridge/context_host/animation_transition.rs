use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::document_runtime::DomHandle;

/// Per-Document state for CSS animation and transition lifecycle events.
///
/// Moli resolves CSS animation values statically (keyframe midpoint) and has no
/// per-frame compositor timeline, so animation and transition events are
/// dispatched from coalesced rendering-update scans plus an `Instant`-based
/// clock advanced at each scan. State is keyed by the owning Document's element
/// handles; entries are dropped when an element disconnects so a later
/// re-insert restarts its animations.
#[derive(Default)]
pub(crate) struct AnimationTransitionState {
    /// Animation names already reported as started for an element. Keyed by
    /// element handle; a name is removed when the element disconnects or when
    /// the transition/animation finishes its last iteration.
    started_animations: HashMap<DomHandle, HashSet<String>>,
    /// In-flight CSS animations awaiting their completion event.
    running_animations: HashMap<DomHandle, Vec<RunningAnimation>>,
    /// Last observed computed value per transitioned property. A change between
    /// scans starts a transition.
    transition_values: HashMap<DomHandle, HashMap<String, String>>,
    /// In-flight transitions awaiting start/end/cancel.
    running_transitions: HashMap<DomHandle, Vec<RunningTransition>>,
}

impl AnimationTransitionState {
    pub(crate) fn has_lifecycle_state(&self, handle: DomHandle) -> bool {
        self.started_animations.contains_key(&handle)
            || self.running_animations.contains_key(&handle)
            || self.transition_values.contains_key(&handle)
            || self.running_transitions.contains_key(&handle)
    }

    pub(crate) fn started_animations_mut(&mut self, handle: DomHandle) -> &mut HashSet<String> {
        self.started_animations.entry(handle).or_default()
    }

    pub(crate) fn running_animations_mut(
        &mut self,
        handle: DomHandle,
    ) -> &mut Vec<RunningAnimation> {
        self.running_animations.entry(handle).or_default()
    }

    pub(crate) fn running_animations(&self, handle: DomHandle) -> &[RunningAnimation] {
        self.running_animations
            .get(&handle)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn transition_values(&self, handle: DomHandle) -> Option<&HashMap<String, String>> {
        self.transition_values.get(&handle)
    }

    pub(crate) fn transition_values_mut(
        &mut self,
        handle: DomHandle,
    ) -> &mut HashMap<String, String> {
        self.transition_values.entry(handle).or_default()
    }

    pub(crate) fn running_transitions_mut(
        &mut self,
        handle: DomHandle,
    ) -> &mut Vec<RunningTransition> {
        self.running_transitions.entry(handle).or_default()
    }

    pub(crate) fn running_transitions(&self, handle: DomHandle) -> &[RunningTransition] {
        self.running_transitions
            .get(&handle)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Clear animation lifecycle state without discarding transition baselines
    /// or in-flight transitions for the same element.
    pub(crate) fn forget_animations(&mut self, handle: DomHandle) {
        self.started_animations.remove(&handle);
        self.running_animations.remove(&handle);
    }

    /// Clear retained transition state when no supported property can be
    /// transitioned under the element's current computed style.
    pub(crate) fn forget_transitions(&mut self, handle: DomHandle) {
        self.transition_values.remove(&handle);
        self.running_transitions.remove(&handle);
    }

    /// Remove animation state for elements that are no longer connected in the
    /// Document. `connected` holds the handles still present in the tree.
    pub(crate) fn retain_connected(&mut self, connected: &HashSet<DomHandle>) {
        self.started_animations
            .retain(|handle, _| connected.contains(handle));
        self.running_animations
            .retain(|handle, _| connected.contains(handle));
        self.transition_values
            .retain(|handle, _| connected.contains(handle));
        self.running_transitions
            .retain(|handle, _| connected.contains(handle));
    }
}

/// One CSS animation currently spanning its declared active duration.
#[derive(Clone, Debug)]
pub(crate) struct RunningAnimation {
    pub(crate) name: String,
    pub(crate) started: Instant,
    pub(crate) delay: std::time::Duration,
    pub(crate) duration: std::time::Duration,
    pub(crate) iterations: f64,
}

/// One CSS transition currently spanning its declared active duration.
#[derive(Clone, Debug)]
pub(crate) struct RunningTransition {
    pub(crate) property: String,
    pub(crate) started: Instant,
    pub(crate) delay: std::time::Duration,
    pub(crate) duration: std::time::Duration,
    /// Whether `transitionstart` has already been dispatched. `transitionrun`
    /// fires at creation; `transitionstart` fires once the delay elapses.
    pub(crate) started_event_sent: bool,
}
