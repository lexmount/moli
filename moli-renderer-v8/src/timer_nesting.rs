//! Timer chains belong to their target ExecutionContext, even when the
//! callback function comes from another Realm.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Duration,
};

use crate::native_bridge::WindowExecutionContextOwner;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TimerNestingOwner {
    Window(WindowExecutionContextOwner),
    Worker,
}

/// Only the distinction between levels 0 through 6 and higher levels affects
/// timer delays. Saturate once the 4ms minimum applies.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TimerNestingLevel(u8);

impl TimerNestingLevel {
    pub(crate) fn for_new_timer(isolate: &v8::Isolate, owner: TimerNestingOwner) -> Self {
        let parent = isolate
            .get_slot::<Rc<TimerNestingState>>()
            .and_then(|state| state.levels.borrow().get(&owner).copied())
            .unwrap_or_default();
        parent.next()
    }

    pub(crate) fn next(self) -> Self {
        Self((self.0 + 1).min(7))
    }

    pub(crate) fn clamp_delay_ms(self, delay_ms: u64) -> u64 {
        if self.0 > 6 {
            delay_ms.max(4)
        } else {
            delay_ms
        }
    }

    pub(crate) fn clamp_duration(self, delay: Duration) -> Duration {
        if self.0 > 6 {
            delay.max(Duration::from_millis(4))
        } else {
            delay
        }
    }

    pub(crate) fn activate(self, isolate: &v8::Isolate, owner: TimerNestingOwner) {
        let state = isolate
            .get_slot::<Rc<TimerNestingState>>()
            .expect("a running timer has a task nesting scope");
        state.levels.borrow_mut().insert(owner, self);
    }
}

type TimerLevels = HashMap<TimerNestingOwner, TimerNestingLevel>;

#[derive(Default)]
struct TimerNestingState {
    levels: RefCell<TimerLevels>,
    scope_depth: Cell<usize>,
}

/// Keep an ExecutionContext's timer level through its callback checkpoint.
/// Consume this guard after the checkpoint, before asynchronous runtime
/// follow-up. Drop also restores it when a callback or checkpoint fails.
pub(crate) struct TimerNestingScope {
    state: Rc<TimerNestingState>,
    previous: TimerLevels,
    previous_depth: usize,
}

impl TimerNestingScope {
    pub(crate) fn enter(isolate: &mut v8::Isolate) -> Self {
        let state = match isolate.get_slot::<Rc<TimerNestingState>>() {
            Some(state) => state.clone(),
            None => {
                let state = Rc::new(TimerNestingState::default());
                isolate.set_slot(state.clone());
                state
            }
        };
        Self::enter_with_state(state)
    }

    fn enter_with_state(state: Rc<TimerNestingState>) -> Self {
        let previous = state.levels.borrow().clone();
        let previous_depth = state.scope_depth.get();
        state.scope_depth.set(previous_depth + 1);
        Self {
            state,
            previous,
            previous_depth,
        }
    }

    /// Body-only execution has no checkpoint. Complete-task entrypoints keep
    /// an outer scope through the checkpoint as well.
    pub(crate) fn enter_if_absent(isolate: &mut v8::Isolate) -> Option<Self> {
        if isolate
            .get_slot::<Rc<TimerNestingState>>()
            .is_some_and(|state| state.scope_depth.get() != 0)
        {
            None
        } else {
            Some(Self::enter(isolate))
        }
    }
}

impl Drop for TimerNestingScope {
    fn drop(&mut self) {
        debug_assert_eq!(self.state.scope_depth.get(), self.previous_depth + 1);
        let mut levels = self.state.levels.borrow_mut();
        levels.clear();
        levels.extend(std::mem::take(&mut self.previous));
        self.state.scope_depth.set(self.previous_depth);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_nesting_preserves_six_unclamped_levels_then_saturates() {
        let mut level = TimerNestingLevel::default();
        for expected in [0, 0, 0, 0, 0, 0, 4, 4, 4] {
            level = level.next();
            assert_eq!(level.clamp_delay_ms(0), expected);
            assert_eq!(
                level.clamp_duration(Duration::ZERO).as_millis(),
                u128::from(expected)
            );
            assert_eq!(level.clamp_delay_ms(u64::MAX), u64::MAX);
        }
    }

    #[test]
    fn timer_nesting_scopes_restore_reentry_and_finish_before_followup() {
        let state = Rc::new(TimerNestingState::default());
        let outer = TimerNestingScope::enter_with_state(state.clone());
        state
            .levels
            .borrow_mut()
            .insert(TimerNestingOwner::Worker, TimerNestingLevel(6));
        let inner = TimerNestingScope::enter_with_state(state.clone());
        state
            .levels
            .borrow_mut()
            .insert(TimerNestingOwner::Worker, TimerNestingLevel(7));
        drop(inner);
        assert_eq!(
            state.levels.borrow()[&TimerNestingOwner::Worker],
            TimerNestingLevel(6)
        );
        let followup = TimerNestingScope::enter_with_state(state.clone());
        state
            .levels
            .borrow_mut()
            .insert(TimerNestingOwner::Worker, TimerNestingLevel(2));
        assert_eq!(
            state.levels.borrow()[&TimerNestingOwner::Worker],
            TimerNestingLevel(2)
        );
        drop(followup);
        assert_eq!(
            state.levels.borrow()[&TimerNestingOwner::Worker],
            TimerNestingLevel(6)
        );
        drop(outer);
        assert!(state.levels.borrow().is_empty());
        assert_eq!(state.scope_depth.get(), 0);
    }
}
