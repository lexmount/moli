use std::time::Instant;

use crate::{ClickAction, ScrollAction};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ActionBatchId(u64);

impl ActionBatchId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ActionSequence(u64);

impl ActionSequence {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }
}

/// Why a pending window was made ready for execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionBatchCause {
    Deadline,
    Barrier(ActionBarrier),
}

/// A read or synchronization operation that requires pending actions first.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionBarrier {
    Screenshot,
    Screencast,
    Explicit,
}

/// One retained input with its original admission metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduledAction<T> {
    sequence: ActionSequence,
    admitted_at: Instant,
    value: T,
}

impl<T> ScheduledAction<T> {
    pub(crate) const fn new(sequence: ActionSequence, admitted_at: Instant, value: T) -> Self {
        Self {
            sequence,
            admitted_at,
            value,
        }
    }

    #[must_use]
    pub const fn sequence(&self) -> ActionSequence {
        self.sequence
    }

    #[must_use]
    pub const fn admitted_at(&self) -> Instant {
        self.admitted_at
    }

    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    #[must_use]
    pub fn into_value(self) -> T {
        self.value
    }
}

/// Adjacent scroll steps for the same scope.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollRun {
    steps: Vec<ScheduledAction<ScrollAction>>,
}

impl ScrollRun {
    pub(crate) fn new(step: ScheduledAction<ScrollAction>) -> Self {
        Self { steps: vec![step] }
    }

    pub(crate) fn push(&mut self, step: ScheduledAction<ScrollAction>) {
        self.steps.push(step);
    }

    pub(crate) fn append(&mut self, other: &mut Self) {
        self.steps.append(&mut other.steps);
    }

    #[must_use]
    pub fn steps(&self) -> &[ScheduledAction<ScrollAction>] {
        &self.steps
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.steps.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// A compacted unit in execution order.
#[derive(Clone, Debug, PartialEq)]
pub enum PlannedAction<S, O = ()> {
    Scroll {
        scope: S,
        run: ScrollRun,
    },
    Click {
        scope: S,
        click: ScheduledAction<ClickAction>,
    },
    Ordered {
        scope: S,
        action: ScheduledAction<O>,
    },
}

impl<S, O> PlannedAction<S, O> {
    #[must_use]
    pub const fn scope(&self) -> &S {
        match self {
            Self::Scroll { scope, .. }
            | Self::Click { scope, .. }
            | Self::Ordered { scope, .. } => scope,
        }
    }

    pub(crate) fn retained_action_count(&self) -> usize {
        match self {
            Self::Scroll { run, .. } => run.len(),
            Self::Click { .. } | Self::Ordered { .. } => 1,
        }
    }
}

/// A closed window ready for ordered execution followed by one derived-work
/// commit.
#[derive(Clone, Debug, PartialEq)]
pub struct ActionBatch<S, O = ()> {
    id: ActionBatchId,
    opened_at: Instant,
    deadline: Instant,
    released_at: Instant,
    cause: ActionBatchCause,
    admitted_action_count: usize,
    retained_action_count: usize,
    actions: Vec<PlannedAction<S, O>>,
}

impl<S, O> ActionBatch<S, O> {
    pub(crate) const fn new(
        id: ActionBatchId,
        opened_at: Instant,
        deadline: Instant,
        released_at: Instant,
        cause: ActionBatchCause,
        admitted_action_count: usize,
        retained_action_count: usize,
        actions: Vec<PlannedAction<S, O>>,
    ) -> Self {
        Self {
            id,
            opened_at,
            deadline,
            released_at,
            cause,
            admitted_action_count,
            retained_action_count,
            actions,
        }
    }

    #[must_use]
    pub const fn id(&self) -> ActionBatchId {
        self.id
    }

    #[must_use]
    pub const fn opened_at(&self) -> Instant {
        self.opened_at
    }

    #[must_use]
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    /// The time at which the caller observed the batch as ready.
    #[must_use]
    pub const fn released_at(&self) -> Instant {
        self.released_at
    }

    #[must_use]
    pub const fn cause(&self) -> ActionBatchCause {
        self.cause
    }

    /// Number of actions admitted, including clicks later superseded.
    #[must_use]
    pub const fn admitted_action_count(&self) -> usize {
        self.admitted_action_count
    }

    /// Number of retained logical inputs. Every scroll step counts once.
    #[must_use]
    pub const fn retained_action_count(&self) -> usize {
        self.retained_action_count
    }

    /// Number of compacted execution units. A scroll run counts once.
    #[must_use]
    pub const fn planned_action_count(&self) -> usize {
        self.actions.len()
    }

    #[must_use]
    pub fn actions(&self) -> &[PlannedAction<S, O>] {
        &self.actions
    }

    #[must_use]
    pub fn into_actions(self) -> Vec<PlannedAction<S, O>> {
        self.actions
    }
}
