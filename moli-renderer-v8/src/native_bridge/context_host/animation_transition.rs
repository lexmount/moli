use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use super::WindowDocumentOwner;
use crate::animation_model::{Animation, CssLifecycleEvent};
use crate::document_runtime::DomHandle;
#[cfg(test)]
use moli_animation::AnimationClock;

/// CSS lifecycle state is isolated by exact Document owner, including iframe
/// replacement and document.open(), rather than by the current scan's tree.
#[derive(Default)]
pub(crate) struct AnimationTransitionState {
    script_time: Rc<Cell<Option<Instant>>>,
    // Ordinary style observations only borrow the Host immutably. They still
    // establish an element's first before-change style, before any lifecycle task.
    documents: RefCell<HashMap<WindowDocumentOwner, DocumentAnimationTransitionState>>,
    // Effect targets can detach or move between Documents independently of the
    // timeline owner. This index never keeps an effect alive by itself.
    effects: RefCell<
        HashMap<DomHandle, Vec<std::rc::Weak<RefCell<crate::animation_model::AnimationInstance>>>>,
    >,
}

impl AnimationTransitionState {
    pub(crate) fn sample_time(&self) -> Instant {
        self.script_time.get().unwrap_or_else(Instant::now)
    }
    pub(crate) fn enter_script_time(&self, now: Instant) -> AnimationScriptTimeGuard {
        let previous = self.script_time.get();
        self.script_time.set(Some(previous.unwrap_or(now)));
        AnimationScriptTimeGuard {
            time: self.script_time.clone(),
            previous,
        }
    }
    pub(crate) fn document_mut(
        &self,
        owner: WindowDocumentOwner,
    ) -> RefMut<'_, DocumentAnimationTransitionState> {
        RefMut::map(self.documents.borrow_mut(), |documents| {
            documents.entry(owner).or_default()
        })
    }

    pub(crate) fn document(
        &self,
        owner: WindowDocumentOwner,
    ) -> Option<Ref<'_, DocumentAnimationTransitionState>> {
        Ref::filter_map(self.documents.borrow(), |documents| documents.get(&owner)).ok()
    }

    pub(super) fn take_scheduled_scan(
        &self,
        owner: WindowDocumentOwner,
    ) -> Option<ScheduledAnimationScan> {
        self.documents
            .borrow_mut()
            .get_mut(&owner)?
            .scheduled_scan
            .take()
    }

    pub(crate) fn remove_document(&mut self, owner: WindowDocumentOwner) {
        let retired = self.documents.get_mut().remove(&owner);
        let indexed = self
            .effects
            .get_mut()
            .values()
            .flat_map(|effects| effects.iter().filter_map(std::rc::Weak::upgrade))
            .collect::<Vec<_>>();
        let document_effects = retired.into_iter().flat_map(|d| {
            d.engine
                .into_instances()
                .chain(d.elements.into_values().flat_map(|s| {
                    s.animations
                        .into_iter()
                        .chain(s.running_transitions.into_iter().map(|t| t.animation))
                }))
        });
        for animation in indexed.into_iter().chain(document_effects) {
            let mut state = animation.borrow_mut();
            if state
                .execution_owner
                .is_some_and(|target| target.owner() == owner)
            {
                state.playback.retire(Instant::now());
                state.retired = true;
            }
        }
        self.effects.get_mut().retain(|_, effects| {
            effects.retain(|a| a.upgrade().is_some_and(|a| !a.borrow().retired));
            !effects.is_empty()
        });
    }

    pub(crate) fn index_effect(&self, target: DomHandle, animation: &Animation) {
        let mut effects = self.effects.borrow_mut();
        let entries = effects.entry(target).or_default();
        entries.retain(|a| a.strong_count() > 0);
        if !entries
            .iter()
            .any(|a| a.ptr_eq(&std::rc::Rc::downgrade(animation)))
        {
            entries.push(std::rc::Rc::downgrade(animation));
        }
    }
    pub(crate) fn effects_for_target(&self, target: DomHandle) -> Vec<Animation> {
        self.effects
            .borrow()
            .get(&target)
            .into_iter()
            .flatten()
            .filter_map(std::rc::Weak::upgrade)
            .collect()
    }
    pub(crate) fn remove_script_reference(
        &self,
        target: DomHandle,
        id: moli_animation::AnimationId,
    ) {
        let mut effects = self.effects.borrow_mut();
        if let Some(entries) = effects.get_mut(&target) {
            entries.retain(|a| a.upgrade().is_some_and(|a| a.borrow().playback.id() != id));
            if entries.is_empty() {
                effects.remove(&target);
            }
        }
    }
    /// Empty pages should not resolve frame ownership or effect targets for
    /// each ordinary CSSOM/layout read. False positives are harmless; no
    /// duplicated counter has to be synchronized with instance admission.
    pub(crate) fn has_effects(&self) -> bool {
        !self.effects.borrow().is_empty()
            || self
                .documents
                .borrow()
                .values()
                .any(|document| !document.elements.is_empty())
    }

    pub(crate) fn observe_initial_style(
        &self,
        owner: WindowDocumentOwner,
        handle: DomHandle,
        style: &crate::style_engine::StyloComputedStyleSnapshot,
    ) {
        self.document_mut(owner)
            .before_styles
            .entry(handle)
            .or_insert_with(|| style.clone());
    }

    pub(crate) fn finish_style_sample(&self, owner: WindowDocumentOwner) {
        if let Some(document) = self.documents.borrow_mut().get_mut(&owner) {
            document.finish_style_sample();
        }
    }

    pub(crate) fn retain_style_baselines(
        &self,
        owner: WindowDocumentOwner,
        mut keep: impl FnMut(DomHandle) -> bool,
    ) {
        if let Some(document) = self.documents.borrow_mut().get_mut(&owner) {
            document.before_styles.retain(|handle, _| keep(*handle));
        }
    }
}

/// Own only the sampling cell across JS, never a Host/Document RefCell borrow.
/// Nested realm entry and native callbacks inherit the enclosing sample.
pub(crate) struct AnimationScriptTimeGuard {
    time: Rc<Cell<Option<Instant>>>,
    previous: Option<Instant>,
}
impl Drop for AnimationScriptTimeGuard {
    fn drop(&mut self) {
        self.time.set(self.previous);
    }
}

/// CSS declaration associations and unanimated baselines belong to a Document;
/// its timeline services only registered active instances at time boundaries.
#[derive(Default)]
pub(crate) struct DocumentAnimationTransitionState {
    pub(super) scheduled_scan: Option<ScheduledAnimationScan>,
    style_change_time: StyleChangeTime,
    pub(crate) reconciling_style: bool,
    pub(crate) pending_events: Vec<(DomHandle, CssLifecycleEvent)>,
    pub(crate) elements: HashMap<DomHandle, ElementAnimationTransitionState>,
    pub(crate) engine: moli_animation::AnimationEngine<crate::animation_model::AnimationInstance>,
    // Baselines also cover idle elements. Keep them separate so those elements
    // do not pay for empty animation/transition vectors.
    pub(crate) before_styles: HashMap<DomHandle, crate::style_engine::StyloComputedStyleSnapshot>,
}

/// Mutations in a lifecycle callback use that batch's sampled timeline time.
/// Other mutations capture their admission time, so a delayed scan does not
/// count its queueing delay as playback before applying a pause.
#[derive(Clone, Copy, Default)]
enum StyleChangeTime {
    #[default]
    Idle,
    Dispatching(Instant),
    Pending(Instant),
}

#[derive(Default)]
pub(crate) struct ElementAnimationTransitionState {
    /// Keep completed effects too: timing edits update the same animation,
    /// and an unrelated style scan must not admit it again.
    pub(crate) animations: Vec<Animation>,
    pub(crate) running_transitions: Vec<RunningTransition>,
}

pub(super) struct ScheduledAnimationScan {
    pub(super) task_id: crate::page_task_queue::RendererPageRenderingUpdateTaskId,
    pub(super) task: tokio::task::JoinHandle<()>,
}

impl Drop for ScheduledAnimationScan {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl DocumentAnimationTransitionState {
    pub(crate) fn activate(&mut self, animation: &Animation) {
        self.engine.activate(animation);
    }
    pub(crate) fn note_style_change(&mut self, now: Instant) {
        self.style_change_time = StyleChangeTime::Pending(match self.style_change_time {
            StyleChangeTime::Idle => now,
            StyleChangeTime::Dispatching(at) | StyleChangeTime::Pending(at) => at,
        });
    }

    pub(crate) fn begin_style_sample(&mut self, now: Instant) -> Instant {
        let changed_at = match self.style_change_time {
            StyleChangeTime::Pending(at) => at,
            _ => now,
        };
        self.style_change_time = StyleChangeTime::Dispatching(now);
        changed_at
    }

    pub(crate) fn finish_observed_style_sample(&mut self) {
        self.finish_style_sample();
    }

    fn finish_style_sample(&mut self) {
        if matches!(self.style_change_time, StyleChangeTime::Dispatching(_)) {
            self.style_change_time = StyleChangeTime::Idle;
        }
    }

    pub(crate) fn element_mut(
        &mut self,
        handle: DomHandle,
    ) -> &mut ElementAnimationTransitionState {
        self.elements.entry(handle).or_default()
    }

    pub(crate) fn take_disconnected(
        &mut self,
        connected: &HashSet<DomHandle>,
    ) -> Vec<(DomHandle, ElementAnimationTransitionState)> {
        let removed = self
            .elements
            .extract_if(|handle, _| !connected.contains(handle))
            .collect();
        self.elements.retain(|_, state| {
            !state.animations.is_empty() || !state.running_transitions.is_empty()
        });
        self.before_styles
            .retain(|handle, _| connected.contains(handle));
        removed
    }
}

/// A transition effect retains its value interval and reversal history; timing
/// and script control live in the same instance used by CSS animations.
#[derive(Debug)]
pub(crate) struct RunningTransition {
    pub(crate) property: String,
    pub(crate) animation: Animation,
    pub(crate) to: String,
    pub(crate) reversing_adjusted_start: String,
    pub(crate) shortening_factor: f64,
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_pause_uses_its_batch_time_even_when_the_next_scan_is_late() {
        let now = Instant::now();
        let mut document = DocumentAnimationTransitionState::default();
        let mut clock = AnimationClock::new(now, false);
        let sampled = now + Duration::from_millis(20);
        document.begin_style_sample(sampled);
        document.note_style_change(now + Duration::from_millis(50));
        document.note_style_change(now + Duration::from_secs(1));
        document.finish_style_sample();

        let changed_at = document.begin_style_sample(now + Duration::from_secs(10));
        clock.update_play_state(changed_at, true);
        assert_eq!(changed_at, sampled);
        assert_eq!(
            moli_animation::duration(clock.seconds_at(now + Duration::from_secs(10))),
            Duration::from_millis(20)
        );
        document.finish_style_sample();

        let resumed = now + Duration::from_secs(11);
        document.note_style_change(resumed);
        let changed_at = document.begin_style_sample(resumed + Duration::from_millis(10));
        clock.update_play_state(changed_at, false);
        assert_eq!(
            moli_animation::duration(clock.seconds_at(resumed + Duration::from_millis(10))),
            Duration::from_millis(30)
        );
    }
}
