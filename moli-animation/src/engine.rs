use crate::{
    AdvanceMode, AnimationCommand, AnimationEvent, AnimationId, AnimationSample, AnimationState,
    CommandOutcome, InvalidPlaybackState,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
    time::Instant,
};

/// Hosts can retain their associations alongside the single playback state.
/// The engine never inspects host data and does not allocate a second instance.
pub trait PlaybackState {
    fn playback(&self) -> &AnimationState;
    fn playback_mut(&mut self) -> &mut AnimationState;
}
impl PlaybackState for AnimationState {
    fn playback(&self) -> &AnimationState {
        self
    }
    fn playback_mut(&mut self) -> &mut AnimationState {
        self
    }
}

/// A timeline's identity registry and active instances. Dormant entries are weak:
/// registering an animation must not keep its host, DOM target or wrapper alive.
/// Advancement visits only active instances, in admission order, and invokes no callbacks.
pub struct AnimationEngine<T: PlaybackState = AnimationState> {
    instances: HashMap<AnimationId, Entry<T>>,
    active: Vec<Rc<RefCell<T>>>,
    prune_at: usize,
}
struct Entry<T> {
    instance: Weak<RefCell<T>>,
    queued: bool,
}
impl<T: PlaybackState> Default for AnimationEngine<T> {
    fn default() -> Self {
        Self {
            instances: HashMap::new(),
            active: Vec::new(),
            prune_at: 64,
        }
    }
}
/// All states have been committed before this batch is returned to the host.
pub struct Advance<T: PlaybackState> {
    pub updates: Vec<AnimationUpdate<T>>,
    pub next_wake: Option<Instant>,
}
pub struct AnimationUpdate<T: PlaybackState> {
    pub animation: Rc<RefCell<T>>,
    pub events: Vec<AnimationEvent>,
}
impl<T: PlaybackState> AnimationEngine<T> {
    pub fn create(&mut self, state: T) -> Rc<RefCell<T>> {
        let instance = Rc::new(RefCell::new(state));
        self.activate(&instance);
        instance
    }
    /// Re-admission after a host-side effect/configuration change is idempotent.
    pub fn activate(&mut self, animation: &Rc<RefCell<T>>) {
        if self.instances.len() >= self.prune_at {
            self.prune();
        }
        let id = animation.borrow().playback().id();
        let entry = self.instances.entry(id).or_insert_with(|| Entry {
            instance: Rc::downgrade(animation),
            queued: false,
        });
        if !entry.queued {
            entry.queued = true;
            self.active.push(animation.clone());
        }
    }
    pub fn get(&self, id: AnimationId) -> Option<Rc<RefCell<T>>> {
        self.instances.get(&id)?.instance.upgrade()
    }
    pub fn command(
        &mut self,
        id: AnimationId,
        command: AnimationCommand,
        now: Instant,
    ) -> Result<CommandOutcome, InvalidPlaybackState> {
        let animation = self.get(id).ok_or(InvalidPlaybackState)?;
        let outcome = animation
            .borrow_mut()
            .playback_mut()
            .command(command, now)?;
        self.activate(&animation);
        Ok(outcome)
    }
    pub fn sample(&self, id: AnimationId, now: Instant) -> Option<AnimationSample> {
        Some(self.get(id)?.borrow().playback().sample(now))
    }
    pub fn active_instances(&self) -> impl Iterator<Item = &Rc<RefCell<T>>> {
        self.active.iter()
    }
    pub fn is_active(&self) -> bool {
        !self.active.is_empty()
    }
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.active
            .iter()
            .filter_map(|a| a.borrow().playback().next_deadline(now))
            .min()
    }
    pub fn advance(&mut self, now: Instant) -> Advance<T> {
        let mut updates = Vec::with_capacity(self.active.len());
        let active = std::mem::take(&mut self.active);
        for animation in active {
            let (events, keep) = {
                let mut instance = animation.borrow_mut();
                let state = instance.playback_mut();
                let events = state.advance(now, AdvanceMode::Tick);
                (events, state.needs_timeline_service(now))
            };
            if keep {
                self.active.push(animation.clone());
            } else if let Some(entry) = self.instances.get_mut(&animation.borrow().playback().id())
            {
                entry.queued = false;
            }
            updates.push(AnimationUpdate { animation, events });
        }
        Advance {
            updates,
            next_wake: self.next_deadline(now),
        }
    }
    /// Admissions also prune, so script-only animations do not depend on CSS
    /// reconciliation. Grow the threshold with live entries to amortize scans.
    pub fn prune(&mut self) {
        self.instances
            .retain(|_, entry| entry.instance.strong_count() > 0);
        self.prune_at = self.instances.len().saturating_mul(2).max(64);
    }
    pub fn into_instances(self) -> impl Iterator<Item = Rc<RefCell<T>>> {
        self.instances
            .into_values()
            .filter(|entry| !entry.queued)
            .filter_map(|entry| entry.instance.upgrade())
            .chain(self.active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn script_only_admissions_bound_dead_registry_entries() {
        let now = Instant::now();
        let mut engine = AnimationEngine::default();
        for _ in 0..1000 {
            let mut state = AnimationState::css(now);
            state.duration = Duration::from_millis(1);
            let instance = engine.create(state);
            drop(engine.advance(now + Duration::from_secs(1)));
            drop(instance);
            assert!(!engine.is_active());
            assert!(engine.instances.len() <= 64);
        }
    }

    #[test]
    fn pruning_preserves_live_dormant_ids_and_scales_with_live_entries() {
        let now = Instant::now();
        let mut engine = AnimationEngine::default();
        let instances = (0..1024)
            .map(|_| engine.create(AnimationState::new(now)))
            .collect::<Vec<_>>();
        engine.advance(now);
        assert!(!engine.is_active());
        let limit = engine.prune_at;
        for instance in &instances {
            let id = instance.borrow().id();
            assert!(Rc::ptr_eq(&engine.get(id).unwrap(), instance));
            engine.activate(instance);
        }
        // The first readmission can grow the threshold; subsequent ones do
        // not each scan all live entries.
        assert!(engine.prune_at >= limit);
        assert!(engine.prune_at >= 2 * instances.len());
        assert_eq!(engine.instances.len(), instances.len());
    }
}
