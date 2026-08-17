use std::time::{Duration, Instant};

use crate::{
    ActionBarrier, ActionBatch, ActionBatchCause, ActionBatchId, ActionSequence, PlannedAction,
    ScheduledAction, ScrollRun, WindowAction,
};

const ACTION_WINDOW_DURATION: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionCompaction {
    Added,
    AppendedToScrollRun,
    ReplacedClick,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionState {
    Opened,
    Joined,
    Rotated,
}

/// Result of admitting an action.
///
/// `ready_batch` is present when the previous window reached its deadline. The
/// caller must execute it before the newly admitted action.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionAdmission<S, O = ()> {
    state: AdmissionState,
    batch_id: ActionBatchId,
    sequence: ActionSequence,
    deadline: Instant,
    compaction: ActionCompaction,
    ready_batch: Option<ActionBatch<S, O>>,
}

impl<S, O> ActionAdmission<S, O> {
    #[must_use]
    pub const fn state(&self) -> AdmissionState {
        self.state
    }

    #[must_use]
    pub const fn batch_id(&self) -> ActionBatchId {
        self.batch_id
    }

    /// The stable sequence assigned to the newly admitted action.
    ///
    /// Hosts can use this to associate renderer-specific payloads with the
    /// semantic action retained by the window.
    #[must_use]
    pub const fn sequence(&self) -> ActionSequence {
        self.sequence
    }

    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    #[must_use]
    pub const fn compaction(&self) -> ActionCompaction {
        self.compaction
    }

    #[must_use]
    pub const fn ready_batch(&self) -> Option<&ActionBatch<S, O>> {
        self.ready_batch.as_ref()
    }

    #[must_use]
    pub fn into_ready_batch(self) -> Option<ActionBatch<S, O>> {
        self.ready_batch
    }
}

struct PendingWindow<S, O> {
    id: ActionBatchId,
    opened_at: Instant,
    deadline: Instant,
    admitted_action_count: usize,
    retained_action_count: usize,
    actions: Vec<PlannedAction<S, O>>,
}

impl<S: PartialEq, O> PendingWindow<S, O> {
    fn admit(
        &mut self,
        scope: S,
        action: WindowAction<O>,
        sequence: ActionSequence,
        admitted_at: Instant,
    ) -> ActionCompaction {
        self.admitted_action_count += 1;

        match action {
            WindowAction::Scroll(scroll) => {
                self.retained_action_count += 1;
                let scheduled = ScheduledAction::new(sequence, admitted_at, scroll);
                if let Some(PlannedAction::Scroll {
                    scope: previous_scope,
                    run,
                }) = self.actions.last_mut()
                    && previous_scope == &scope
                {
                    run.push(scheduled);
                    ActionCompaction::AppendedToScrollRun
                } else {
                    self.actions.push(PlannedAction::Scroll {
                        scope,
                        run: ScrollRun::new(scheduled),
                    });
                    ActionCompaction::Added
                }
            }
            WindowAction::Click(click) => {
                let previous = self.actions.iter().position(|action| {
                    matches!(action, PlannedAction::Click { scope: click_scope, .. } if click_scope == &scope)
                });
                if let Some(index) = previous {
                    self.actions.remove(index);
                    self.normalize_scroll_runs();
                } else {
                    self.retained_action_count += 1;
                }
                self.actions.push(PlannedAction::Click {
                    scope,
                    click: ScheduledAction::new(sequence, admitted_at, click),
                });
                if previous.is_some() {
                    ActionCompaction::ReplacedClick
                } else {
                    ActionCompaction::Added
                }
            }
            WindowAction::Ordered(action) => {
                self.retained_action_count += 1;
                self.actions.push(PlannedAction::Ordered {
                    scope,
                    action: ScheduledAction::new(sequence, admitted_at, action),
                });
                ActionCompaction::Added
            }
        }
    }

    fn normalize_scroll_runs(&mut self) {
        let mut normalized = Vec::with_capacity(self.actions.len());
        for action in self.actions.drain(..) {
            match action {
                PlannedAction::Scroll { scope, mut run } => {
                    if let Some(PlannedAction::Scroll {
                        scope: previous_scope,
                        run: previous_run,
                    }) = normalized.last_mut()
                        && previous_scope == &scope
                    {
                        previous_run.append(&mut run);
                    } else {
                        normalized.push(PlannedAction::Scroll { scope, run });
                    }
                }
                other => normalized.push(other),
            }
        }
        self.actions = normalized;
    }

    fn remove_scope(&mut self, scope: &S) -> Vec<ActionSequence> {
        let mut removed_sequences = Vec::new();
        self.actions.retain(|action| {
            if action.scope() != scope {
                return true;
            }
            match action {
                PlannedAction::Scroll { run, .. } => {
                    removed_sequences.extend(run.steps().iter().map(|step| step.sequence()))
                }
                PlannedAction::Click { click, .. } => {
                    removed_sequences.push(click.sequence());
                }
                PlannedAction::Ordered { action, .. } => {
                    removed_sequences.push(action.sequence());
                }
            }
            false
        });
        self.retained_action_count = self
            .actions
            .iter()
            .map(PlannedAction::retained_action_count)
            .sum();
        self.normalize_scroll_runs();
        removed_sequences
    }

    fn into_batch(self, released_at: Instant, cause: ActionBatchCause) -> ActionBatch<S, O> {
        ActionBatch::new(
            self.id,
            self.opened_at,
            self.deadline,
            released_at,
            cause,
            self.admitted_action_count,
            self.retained_action_count,
            self.actions,
        )
    }
}

/// A timer-independent, fixed one-second, one-shot action window.
pub struct ActionWindow<S, O = ()> {
    next_batch_id: u64,
    next_sequence: u64,
    open: Option<PendingWindow<S, O>>,
}

impl<S, O> Default for ActionWindow<S, O> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S, O> ActionWindow<S, O> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next_batch_id: 1,
            next_sequence: 1,
            open: None,
        }
    }

    #[must_use]
    pub const fn is_idle(&self) -> bool {
        self.open.is_none()
    }

    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        self.open.as_ref().map(|window| window.deadline)
    }

    #[must_use]
    pub fn pending_admitted_action_count(&self) -> usize {
        self.open
            .as_ref()
            .map_or(0, |window| window.admitted_action_count)
    }

    #[must_use]
    pub fn pending_retained_action_count(&self) -> usize {
        self.open
            .as_ref()
            .map_or(0, |window| window.retained_action_count)
    }

    #[must_use]
    pub fn pending_planned_action_count(&self) -> usize {
        self.open.as_ref().map_or(0, |window| window.actions.len())
    }

    /// Drops the pending window and returns its retained action count.
    pub fn clear(&mut self) -> usize {
        self.open
            .take()
            .map_or(0, |window| window.retained_action_count)
    }
}

impl<S: PartialEq, O> ActionWindow<S, O> {
    /// Admits an action, possibly returning an older batch that must execute
    /// first.
    pub fn push(
        &mut self,
        scope: S,
        action: WindowAction<O>,
        admitted_at: Instant,
    ) -> ActionAdmission<S, O> {
        let should_rotate = self
            .open
            .as_ref()
            .is_some_and(|window| admitted_at >= window.deadline);

        let ready_batch = should_rotate.then(|| {
            self.open
                .take()
                .expect("rotation requires an open action window")
                .into_batch(admitted_at, ActionBatchCause::Deadline)
        });

        let state = if ready_batch.is_some() {
            AdmissionState::Rotated
        } else if self.open.is_some() {
            AdmissionState::Joined
        } else {
            AdmissionState::Opened
        };

        if self.open.is_none() {
            self.open_window(admitted_at);
        }

        let sequence = self.allocate_sequence();
        let window = self.open.as_mut().expect("action window was just opened");
        let compaction = window.admit(scope, action, sequence, admitted_at);

        ActionAdmission {
            state,
            batch_id: window.id,
            sequence,
            deadline: window.deadline,
            compaction,
            ready_batch,
        }
    }

    /// Returns a deadline batch when `now` is at or past the fixed deadline.
    pub fn take_due(&mut self, now: Instant) -> Option<ActionBatch<S, O>> {
        if self
            .open
            .as_ref()
            .is_some_and(|window| now >= window.deadline)
        {
            self.open
                .take()
                .map(|window| window.into_batch(now, ActionBatchCause::Deadline))
        } else {
            None
        }
    }

    /// Immediately releases pending actions before a read/synchronization
    /// barrier and resets the window to idle.
    pub fn flush(&mut self, barrier: ActionBarrier, now: Instant) -> Option<ActionBatch<S, O>> {
        self.open
            .take()
            .map(|window| window.into_batch(now, ActionBatchCause::Barrier(barrier)))
    }

    /// Cancels all pending actions for a scope and returns their retained
    /// sequences in execution order.
    ///
    /// If no actions remain, the old deadline is canceled and the queue
    /// returns to idle. Hosts can use the returned identities to retire
    /// matching sidecar payloads at the same boundary.
    pub fn cancel_scope(&mut self, scope: &S) -> Vec<ActionSequence> {
        let Some(window) = self.open.as_mut() else {
            return Vec::new();
        };
        let removed = window.remove_scope(scope);
        if window.actions.is_empty() {
            self.open = None;
        }
        removed
    }

    fn open_window(&mut self, opened_at: Instant) {
        let deadline = opened_at
            .checked_add(ACTION_WINDOW_DURATION)
            .expect("action window deadline exceeds Instant's range");
        let id = self.allocate_batch_id();
        self.open = Some(PendingWindow {
            id,
            opened_at,
            deadline,
            admitted_action_count: 0,
            retained_action_count: 0,
            actions: Vec::new(),
        });
    }

    fn allocate_batch_id(&mut self) -> ActionBatchId {
        let id = ActionBatchId::new(self.next_batch_id);
        self.next_batch_id = self
            .next_batch_id
            .checked_add(1)
            .expect("action batch id exhausted");
        id
    }

    fn allocate_sequence(&mut self) -> ActionSequence {
        let sequence = ActionSequence::new(self.next_sequence);
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("action sequence exhausted");
        sequence
    }
}
