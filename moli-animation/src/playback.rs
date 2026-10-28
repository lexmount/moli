use crate::{AnimationClock, CssTime, FillMode, active_duration, duration};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct AnimationId(pub u64);

static NEXT_ANIMATION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnimationPhase {
    Before,
    Active,
    After,
}

// CSS event history is distinct from the effect's three sampling phases.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CssPhase {
    Idle,
    Pending,
    Before,
    Active,
    After,
}

/// Play and pause tasks share a ready promise, but commit different clock
/// changes. Switching between them preserves the pending promise's identity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PendingOperation {
    #[default]
    None,
    Play,
    Pause,
}

/// Stable playback and effect state, independent of DOM, JS wrappers and timers.
#[derive(Debug)]
pub struct AnimationState {
    id: AnimationId,
    phase: AnimationPhase,
    css_phase: CssPhase,
    phase_elapsed: f64,
    clock: AnimationClock,
    // A finished hold can be outside the effect's range after an explicit seek.
    // Keep it separately from pausing and the advancing timeline mapping.
    finished_hold: Option<f64>,
    pub delay: CssTime,
    pub duration: Duration,
    pub iterations: f64,
    pub fill: FillMode,
    pub direction: String,
    pub frames: Vec<PropertyKeyframes>,
    idle: bool,
    pending_cancel: Option<ReadyPromiseUpdate>,
    pending: PendingOperation,
    timeline_attached: bool,
    pub has_effect: bool,
    finish_notified: bool,
    synchronous_finish: bool,
    cycle: u64,
    ready_generation: u64,
    finished_generation: u64,
    pub duration_infinite: bool,
    pub end_delay: CssTime,
    pub iteration_start: f64,
}

#[derive(Clone, Debug)]
pub struct PropertyKeyframes {
    pub property: String,
    pub values: Vec<KeyframeValue>,
}

impl PropertyKeyframes {
    /// Complete a sorted property track without resolving authored CSS values.
    /// Neutral endpoints depend on the underlying value at sampling time.
    pub fn ensure_neutral_endpoints(&mut self) {
        if self.values.is_empty() {
            return;
        }
        if self.values.first().is_none_or(|v| v.offset != 0.0) {
            self.values.insert(0, KeyframeValue::underlying(0.0));
        }
        if self.values.last().is_none_or(|v| v.offset != 1.0) {
            self.values.push(KeyframeValue::underlying(1.0));
        }
    }
}

/// Per-property keyframe information survives interval selection and sorting.
#[derive(Clone, Debug)]
pub struct KeyframeValue {
    pub offset: f64,
    /// None is a neutral endpoint, resolved against the current underlying
    /// value during effect-stack composition, never against creation-time style.
    pub value: Option<String>,
    pub easing: Option<String>,
}
impl KeyframeValue {
    pub fn new(offset: f64, value: String) -> Self {
        Self {
            offset,
            value: Some(value),
            easing: None,
        }
    }
    pub fn underlying(offset: f64) -> Self {
        Self {
            offset,
            value: None,
            easing: None,
        }
    }
}

/// Endpoints and progress for typed value interpolation by the caller.
pub struct PropertySample {
    pub property: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub progress: f64,
    /// The host applies this parsed timing function to interval-local progress.
    pub easing: Option<String>,
}

/// The sampling caller explicitly selects endpoint semantics. This never
/// changes playback, pending tasks, or committed event history.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SamplingMode {
    Presentation,
    CommitStyles,
}

/// Playback progress before applying the host's parsed timing function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectProgress {
    pub fraction: f64,
    pub before: bool,
}

impl AnimationState {
    /// Create an idle animation. The host attaches its effect and starts playback.
    pub fn new(now: Instant) -> Self {
        let id = NEXT_ANIMATION_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("animation identity exhausted");
        Self {
            id: AnimationId(id),
            phase: AnimationPhase::Before,
            css_phase: CssPhase::Idle,
            phase_elapsed: 0.0,
            clock: AnimationClock::new(now, false),
            finished_hold: None,
            delay: CssTime::default(),
            duration: Duration::ZERO,
            iterations: 1.0,
            fill: FillMode::None,
            direction: "normal".to_owned(),
            frames: Vec::new(),
            idle: true,
            pending_cancel: None,
            pending: PendingOperation::None,
            timeline_attached: true,
            has_effect: false,
            finish_notified: false,
            synchronous_finish: false,
            cycle: 0,
            ready_generation: 0,
            finished_generation: 0,
            duration_infinite: false,
            end_delay: CssTime::default(),
            iteration_start: 0.0,
        }
    }
    pub fn notification(&self, kind: PlaybackNotificationKind) -> PlaybackNotification {
        PlaybackNotification {
            kind,
            cycle: self.cycle,
            ready: self.ready_generation,
            finished: self.finished_generation,
            synchronous_finish: false,
            ready_update: if kind == PlaybackNotificationKind::Ready {
                ReadyPromiseUpdate::Resolve
            } else {
                ReadyPromiseUpdate::Unchanged
            },
        }
    }
    pub fn accepts(&self, notification: PlaybackNotification) -> bool {
        self.cycle == notification.cycle
            && (notification.kind != PlaybackNotificationKind::Ready
                || self.ready_generation == notification.ready)
            && (!matches!(
                notification.kind,
                PlaybackNotificationKind::Finish | PlaybackNotificationKind::Reset
            ) || self.finished_generation == notification.finished)
    }
    pub fn finished_generation(&self) -> u64 {
        self.finished_generation
    }
    pub fn id(&self) -> AnimationId {
        self.id
    }
    pub fn clock(&self) -> &AnimationClock {
        &self.clock
    }
    pub fn is_idle(&self) -> bool {
        self.idle
    }
    pub fn is_pending(&self) -> bool {
        self.pending != PendingOperation::None
    }
    pub fn timeline_attached(&self) -> bool {
        self.timeline_attached
    }
    pub fn committed_phase(&self) -> AnimationPhase {
        self.phase
    }
    /// CSS admission starts at the timeline origin without a pending play task.
    pub fn css(now: Instant) -> Self {
        let mut state = Self::new(now);
        state.idle = false;
        state.has_effect = true;
        state
    }
    pub fn sample(&self, now: Instant) -> AnimationSample {
        AnimationSample {
            id: self.id,
            phase: self.current_phase(now),
            play_state: self.play_state(now),
            local_time: self.current_seconds(now),
            progress: self.progress(now),
            pending: self.is_pending(),
        }
    }
    /// Controls playback without consuming any lifecycle notifications.
    pub fn command(
        &mut self,
        command: AnimationCommand,
        now: Instant,
    ) -> Result<CommandOutcome, InvalidPlaybackState> {
        let was_pending = self.is_pending();
        let was_idle = self.idle;
        let generation = self.ready_generation;
        if !self.apply_command(command, now) {
            return Err(InvalidPlaybackState);
        }
        Ok(CommandOutcome {
            ready: if self.is_pending() && self.ready_generation != generation {
                ReadyPromiseUpdate::Pending
            } else if was_pending && !self.is_pending() {
                if command == AnimationCommand::Cancel {
                    ReadyPromiseUpdate::Reject
                } else {
                    ReadyPromiseUpdate::Resolve
                }
            } else {
                ReadyPromiseUpdate::Unchanged
            },
            finished: if command == AnimationCommand::Cancel && !was_idle {
                FinishedPromiseUpdate::RejectAndRenew
            } else if command == AnimationCommand::Finish
                && !self.is_pending()
                && self.play_state(now) == "finished"
            {
                FinishedPromiseUpdate::Resolve
            } else {
                FinishedPromiseUpdate::Unchanged
            },
        })
    }
    fn apply_command(&mut self, command: AnimationCommand, now: Instant) -> bool {
        match command {
            AnimationCommand::Play => return self.play(now),
            AnimationCommand::Pause => return self.pause(now),
            AnimationCommand::Cancel => self.cancel(now),
            AnimationCommand::Finish => return self.finish(now),
            AnimationCommand::Seek(local) => {
                self.finished_hold = None;
                self.clock.seek(now, local);
                if self.idle || !self.timeline_attached || self.pending == PendingOperation::Pause {
                    self.clock.update_play_state(now, true);
                    self.idle = false;
                }
                if self.pending == PendingOperation::Pause {
                    self.pending = PendingOperation::None;
                }
                if self.is_finished_at(local) {
                    self.finished_hold = Some(local);
                }
            }
            AnimationCommand::SetRate(rate) => {
                self.set_rate(now, rate);
                // The playbackRate setter sets the current time again; like
                // seek, that synchronously completes a pending pause.
                if self.pending == PendingOperation::Pause
                    && self.timeline_attached
                    && let Some(local) = self.current_seconds(now)
                {
                    self.clock.hold(now, Some(local));
                    self.finished_hold = None;
                    self.pending = PendingOperation::None;
                }
            }
            AnimationCommand::Reverse => {
                let rate = self.clock.rate;
                self.set_rate(now, -rate);
                if !self.timeline_attached || !self.play(now) {
                    self.set_rate(now, rate);
                    return false;
                }
                // Reverse is a pending play operation even when playback was
                // already running. Preserve an existing pending ready promise.
                self.queue_pending(PendingOperation::Play);
            }
            AnimationCommand::AttachTimeline(attached) => {
                // Timeline availability does not change the start/hold mapping
                // or the author's paused state. A running mapping is merely
                // unresolved while its time source is unavailable.
                if self.timeline_attached != attached && self.clock.has_start_time() {
                    self.finished_hold = None;
                }
                if self.timeline_attached && !attached {
                    self.phase_elapsed = self.active_elapsed(now).as_secs_f64();
                }
                self.timeline_attached = attached;
            }
            AnimationCommand::SetStartTime { origin, seconds } => {
                let previous = if seconds.is_some() && !self.timeline_attached {
                    None
                } else {
                    self.current_seconds(now)
                };
                self.pending = PendingOperation::None;
                self.finished_hold = None;
                self.clock.set_start_time(now, origin, seconds, previous);
                self.idle = seconds.is_none() && previous.is_none();
            }
        }
        true
    }
    /// Changing a timeline origin preserves its numeric start time. Paused
    /// holds and pending ready operations are independent of that mapping.
    pub fn rebase_timeline(&mut self, previous: Instant, next: Instant, now: Instant) {
        if self.idle || previous == next || !self.clock.has_start_time() {
            return;
        }
        let shift = if previous >= next {
            previous.duration_since(next).as_secs_f64()
        } else {
            -next.duration_since(previous).as_secs_f64()
        };
        let local = self.clock.seconds_at(now) + shift * self.clock.rate;
        self.clock.seek(now, local);
        self.finished_hold = None;
    }

    // The animation's bounded current time is authoritative, including its
    // completed hold. A rate change must not reintroduce time spent at the end.
    fn set_rate(&mut self, now: Instant, rate: f64) {
        let local = self.current_seconds(now);
        self.clock.set_rate(now, rate);
        if let Some(local) = local {
            self.clock.seek(now, local);
            self.finished_hold = self.is_finished_at(local).then_some(local);
        }
    }
    fn is_finished_at(&self, local: f64) -> bool {
        (self.clock.rate > 0.0 && compare_time(local, self.end_time()).is_ge())
            || (self.clock.rate < 0.0 && compare_time(local, 0.0).is_le())
    }
    pub fn preserve_current_time_on_timing_change(&mut self, now: Instant) {
        self.finished_hold = self.current_seconds(now);
    }
    fn update_finished_hold(&mut self) {
        if self
            .finished_hold
            .is_some_and(|hold| !self.is_finished_at(hold))
        {
            // Timing edits reevaluate the existing start-time mapping. Do not
            // restart a naturally completed animation at its old endpoint.
            self.finished_hold = None;
        }
    }
    fn phase_boundaries(&self) -> (f64, f64) {
        let end = self.end_time();
        let before = self.delay.seconds().min(end).max(0.0);
        let after = (self.delay.seconds()
            + self
                .active_duration()
                .map_or(f64::INFINITY, |v| v.as_secs_f64()))
        .min(end)
        .max(0.0);
        (before, after)
    }
    /// CSS play-state updates preserve the existing CSS timeline mapping.
    pub fn set_css_paused(&mut self, paused: bool, now: Instant) {
        // A pending WAAPI operation already owns a ready promise. CSS may
        // replace its requested operation, but must not complete it early.
        if self.is_pending() {
            if paused {
                self.pause(now);
            } else {
                self.play(now);
            }
            return;
        }
        let local = self.current_seconds(now).unwrap_or(0.0);
        self.clock.seek(now, local);
        self.finished_hold = None;
        self.clock.update_play_state(now, paused);
    }
    /// Commit playback notifications; only a Document tick commits CSS phase history.
    pub fn advance(&mut self, now: Instant, mode: AdvanceMode) -> Vec<AnimationEvent> {
        let mut events = Vec::new();
        let synchronous_finish = std::mem::take(&mut self.synchronous_finish);
        if mode == AdvanceMode::Tick && self.commit_pending(now) {
            events.push(AnimationEvent::Playback(
                self.notification(PlaybackNotificationKind::Ready),
            ));
        }
        self.update_finished_hold();
        if let Some(ready) = self.pending_cancel.take() {
            let mut notification = self.notification(PlaybackNotificationKind::Cancel);
            notification.ready_update = ready;
            events.push(AnimationEvent::Playback(notification));
        }
        let next = if self.idle {
            CssPhase::Idle
        } else if self.is_pending() && matches!(self.css_phase, CssPhase::Idle | CssPhase::Pending)
        {
            CssPhase::Pending
        } else if self.current_seconds(now).is_none() {
            CssPhase::Idle
        } else {
            match self.current_phase(now) {
                AnimationPhase::Before => CssPhase::Before,
                AnimationPhase::Active => CssPhase::Active,
                AnimationPhase::After => CssPhase::After,
            }
        };
        let (before, after) = self.phase_boundaries();
        let interval_elapsed = |boundary: f64| {
            duration(boundary - self.delay.seconds())
                .min(self.active_duration().unwrap_or(Duration::MAX))
                .as_secs_f64()
        };
        let start = interval_elapsed(before);
        let end = interval_elapsed(after);
        use CssPhase::{Active, After, Before, Idle, Pending};
        // CSS phases include unresolved and pending playback. Ordinary reads
        // never commit this history; commands in one task coalesce at a tick.
        if mode == AdvanceMode::Tick || self.idle {
            if self.css_phase == Idle && next != Idle {
                events.push(AnimationEvent::Run { elapsed: start });
            }
            let mut emit = |kind, elapsed| events.push(AnimationEvent::Phase { kind, elapsed });
            match (self.css_phase, next) {
                (Idle | Pending | Before, Active) => emit(PhaseEvent::Start, start),
                (Idle | Pending | Before, After) => {
                    emit(PhaseEvent::Start, start);
                    emit(PhaseEvent::End, end);
                }
                (Active, Before) => emit(PhaseEvent::End, start),
                (Active, After) => emit(PhaseEvent::End, end),
                (After, Active) => emit(PhaseEvent::Start, end),
                (After, Before) => {
                    emit(PhaseEvent::Start, end);
                    emit(PhaseEvent::End, start);
                }
                (Pending | Before | Active, Idle) => emit(
                    PhaseEvent::Cancel,
                    if self.idle {
                        self.active_elapsed(now).as_secs_f64()
                    } else {
                        self.phase_elapsed
                    },
                ),
                _ => {}
            }
            self.css_phase = next;
            if next != Idle && next != Pending {
                self.phase = self.current_phase(now);
                self.phase_elapsed = self.active_elapsed(now).as_secs_f64();
            }
        }
        if self.idle {
            return events;
        }
        let finished = !self.is_pending() && self.play_state(now) == "finished";
        if finished != self.finish_notified {
            if !finished {
                self.finished_generation = self.finished_generation.wrapping_add(1);
            }
            let mut notification = self.notification(if finished {
                PlaybackNotificationKind::Finish
            } else {
                PlaybackNotificationKind::Reset
            });
            notification.synchronous_finish = finished && synchronous_finish;
            events.push(AnimationEvent::Playback(notification));
        }
        self.finish_notified = finished;
        events
    }
    /// Document retirement discards pending work without dispatching into a dead owner.
    pub fn retire(&mut self, now: Instant) {
        self.cancel(now);
        self.timeline_attached = false;
        self.pending_cancel = None;
        self.css_phase = CssPhase::Idle;
    }
    /// Copy effect configuration without copying identity, playback or host ownership.
    pub fn copy_effect_from(&mut self, effect: &Self) {
        self.delay = effect.delay;
        self.duration = effect.duration;
        self.duration_infinite = effect.duration_infinite;
        self.iterations = effect.iterations;
        self.fill = effect.fill;
        self.direction = effect.direction.clone();
        self.frames = effect.frames.clone();
        self.end_delay = effect.end_delay;
        self.iteration_start = effect.iteration_start;
        self.has_effect = true;
    }
    pub fn active_duration(&self) -> Option<Duration> {
        if !self.has_effect {
            return Some(Duration::ZERO);
        }
        if self.duration_infinite && self.iterations != 0.0 {
            None
        } else {
            active_duration(self.duration, self.iterations)
        }
    }
    pub fn end_time(&self) -> f64 {
        if !self.has_effect {
            return 0.0;
        }
        (self.end_delay.seconds()
            + self.delay.seconds()
            + self
                .active_duration()
                .map_or(f64::INFINITY, |v| v.as_secs_f64()))
        .max(0.0)
    }
    pub fn current_seconds(&self, now: Instant) -> Option<f64> {
        if self.idle
            || (!self.timeline_attached && !self.clock.is_paused() && self.finished_hold.is_none())
        {
            return None;
        }
        Some({
            if let Some(hold) = self.finished_hold.filter(|hold| self.is_finished_at(*hold)) {
                return Some(hold);
            }
            let local = self.clock.resolved_seconds_at(now)?;
            if self.clock.is_paused() || self.clock.rate == 0.0 {
                local
            } else if self.clock.rate < 0.0 {
                local.max(0.0)
            } else {
                local.min(self.end_time())
            }
        })
    }
    pub fn active_elapsed(&self, now: Instant) -> Duration {
        duration(
            self.current_seconds(now)
                .unwrap_or_else(|| self.clock.seconds_at(now))
                - self.delay.seconds(),
        )
        .min(self.active_duration().unwrap_or(Duration::MAX))
    }
    pub fn current_phase(&self, now: Instant) -> AnimationPhase {
        let local = self.current_seconds(now).unwrap_or(0.0);
        let (before, after) = self.phase_boundaries();
        let before_order = compare_time(local, before);
        let after_order = compare_time(local, after);
        if before_order.is_lt() || (before_order.is_eq() && self.clock.rate < 0.0) {
            AnimationPhase::Before
        } else if after_order.is_gt() || (after_order.is_eq() && self.clock.rate >= 0.0) {
            AnimationPhase::After
        } else {
            AnimationPhase::Active
        }
    }

    pub fn play_state(&self, now: Instant) -> &'static str {
        if self.idle {
            "idle"
        } else if self.pending == PendingOperation::Pause
            || (self.pending != PendingOperation::Play && self.clock.is_paused())
        {
            "paused"
        } else if self
            .current_seconds(now)
            .is_some_and(|local| self.is_finished_at(local))
        {
            "finished"
        } else {
            "running"
        }
    }
    pub fn pause(&mut self, now: Instant) -> bool {
        if self.pending == PendingOperation::Pause || self.play_state(now) == "paused" {
            return true;
        }
        let current = self.current_seconds(now);
        let local = current.unwrap_or_else(|| {
            if self.clock.rate < 0.0 {
                self.end_time()
            } else {
                0.0
            }
        });
        if !local.is_finite() {
            return false;
        }
        // A running clock is frozen when the pause task commits, not when
        // pause() is called. Idle animations need an initial hold immediately.
        if current.is_none() {
            self.clock.hold(now, Some(local));
        }
        self.idle = false;
        self.queue_pending(PendingOperation::Pause);
        true
    }
    pub fn play(&mut self, now: Instant) -> bool {
        let local = self.current_seconds(now);
        let rewind = local.is_none()
            || local.is_some_and(|time| {
                if self.clock.rate < 0.0 {
                    time <= 0.0 || time > self.end_time()
                } else {
                    time < 0.0 || time >= self.end_time()
                }
            });
        if rewind {
            let local = if self.clock.rate < 0.0 {
                self.end_time()
            } else {
                0.0
            };
            if !local.is_finite() {
                return false;
            }
            self.cycle = self.cycle.wrapping_add(1);
            self.clock.seek(now, local);
            self.finished_hold = None;
        }
        if self.pending == PendingOperation::Play || (!rewind && self.play_state(now) == "running")
        {
            return true;
        }
        self.idle = false;
        if rewind || !self.clock.has_start_time() {
            self.clock.update_play_state(now, true);
        }
        self.queue_pending(PendingOperation::Play);
        true
    }
    fn queue_pending(&mut self, operation: PendingOperation) {
        if !self.is_pending() {
            self.ready_generation = self.ready_generation.wrapping_add(1);
        }
        self.pending = operation;
    }
    pub fn commit_pending(&mut self, now: Instant) -> bool {
        if !self.is_pending() || !self.timeline_attached {
            return false;
        }
        let pause = self.pending == PendingOperation::Pause;
        if pause {
            self.clock.hold(now, self.current_seconds(now));
            self.finished_hold = None;
        } else {
            self.clock.update_play_state(now, false);
        }
        self.pending = PendingOperation::None;
        true
    }
    pub fn cancel(&mut self, now: Instant) {
        if self.idle {
            return;
        }
        let local = self.current_seconds(now).unwrap_or(0.0);
        self.clock.seek(now, local);
        self.finished_hold = None;
        self.clock.update_play_state(now, true);
        self.pending_cancel = Some(if self.is_pending() {
            ReadyPromiseUpdate::Reject
        } else {
            ReadyPromiseUpdate::Unchanged
        });
        self.cycle = self.cycle.wrapping_add(1);
        self.idle = true;
        self.pending = PendingOperation::None;
    }
    pub fn finish(&mut self, now: Instant) -> bool {
        let end = self.end_time();
        if (self.clock.rate > 0.0 && !end.is_finite()) || self.clock.rate == 0.0 {
            return false;
        }
        self.idle = false;
        if self.timeline_attached {
            self.pending = PendingOperation::None;
        }
        self.clock
            .seek(now, if self.clock.rate < 0.0 { 0.0 } else { end });
        self.clock.update_play_state(now, !self.timeline_attached);
        self.finished_hold = None;
        self.synchronous_finish = true;
        true
    }
    pub fn is_current(&self, now: Instant) -> bool {
        self.has_effect
            && !self.idle
            && self.current_seconds(now).is_some()
            && match self.current_phase(now) {
                AnimationPhase::Active => self.play_state(now) != "finished",
                AnimationPhase::Before => self.clock.rate > 0.0,
                AnimationPhase::After => self.clock.rate < 0.0,
            }
    }
    pub fn is_in_effect(&self, now: Instant) -> bool {
        self.has_effect
            && !self.idle
            && self.current_seconds(now).is_some()
            && match self.current_phase(now) {
                AnimationPhase::Active => true,
                AnimationPhase::Before => matches!(self.fill, FillMode::Backwards | FillMode::Both),
                AnimationPhase::After => matches!(self.fill, FillMode::Forwards | FillMode::Both),
            }
    }
    pub fn is_relevant(&self, now: Instant) -> bool {
        self.is_current(now) || self.is_in_effect(now)
    }
    pub fn progress(&self, now: Instant) -> Option<EffectProgress> {
        self.progress_for_sample(now, SamplingMode::Presentation)
    }
    /// commitStyles includes the exact effect endpoints, but not times beyond
    /// them. This is deliberately separate from display fill semantics.
    pub fn progress_for_sample(&self, now: Instant, mode: SamplingMode) -> Option<EffectProgress> {
        if !self.has_effect || self.current_seconds(now).is_none() {
            return None;
        }
        let mut phase = self.current_phase(now);
        let local = self.current_seconds(now)?;
        let (before, after) = self.phase_boundaries();
        if mode == SamplingMode::CommitStyles
            && (compare_time(local, before).is_eq() || compare_time(local, after).is_eq())
        {
            phase = AnimationPhase::Active;
        }
        if phase == AnimationPhase::Before
            && !matches!(self.fill, FillMode::Backwards | FillMode::Both)
        {
            return None;
        }
        if phase == AnimationPhase::After
            && !matches!(self.fill, FillMode::Forwards | FillMode::Both)
        {
            return None;
        }
        let elapsed = self.active_elapsed(now).as_secs_f64();
        let iterations = self.iteration_start
            + if self.duration.is_zero() {
                if phase == AnimationPhase::Before {
                    0.0
                } else {
                    self.iterations
                }
            } else {
                elapsed / self.duration.as_secs_f64()
            };
        let iterations = if iterations.is_finite() {
            iterations
        } else {
            self.iteration_start
        };
        let (index, mut progress) = if phase != AnimationPhase::Before
            && self
                .active_duration()
                .is_some_and(|active| compare_time(elapsed, active.as_secs_f64()).is_eq())
            && iterations > 0.0
            && iterations.fract() == 0.0
        {
            (iterations - 1.0, 1.0)
        } else {
            (iterations.floor(), iterations.fract())
        };
        let reverse = match self.direction.as_str() {
            "reverse" => true,
            "alternate" => index % 2.0 >= 1.0,
            "alternate-reverse" => index % 2.0 < 1.0,
            _ => false,
        };
        if reverse {
            progress = 1.0 - progress;
        }
        Some(EffectProgress {
            fraction: progress,
            before: (phase == AnimationPhase::Before && !reverse)
                || (phase == AnimationPhase::After && reverse),
        })
    }
    /// Select a keyframe interval using the easing-adjusted progress from the caller.
    pub fn sample_interval(&self, property: &str, progress: f64) -> Option<PropertySample> {
        let frames = self
            .frames
            .iter()
            .rev()
            .find(|frames| frames.property == property)?;
        let values = &frames.values;
        let first = values.first()?;
        let last = values.last()?;
        let (from, to) = if progress < 0.0 && values.get(1).is_some_and(|v| v.offset == 0.0) {
            (first, first)
        } else if progress >= 1.0
            && values
                .get(values.len().saturating_sub(2))
                .is_some_and(|v| v.offset == 1.0)
        {
            (last, last)
        } else {
            let pair = values
                .windows(2)
                .rev()
                .find(|v| v[0].offset <= progress && v[0].offset < 1.0)
                .or_else(|| values.windows(2).next())?;
            (&pair[0], &pair[1])
        };
        let range = to.offset - from.offset;
        let local = if range == 0.0 {
            1.0
        } else {
            (progress - from.offset) / range
        };
        Some(PropertySample {
            property: property.to_owned(),
            from: from.value.clone(),
            to: to.value.clone(),
            progress: local,
            easing: from.easing.clone(),
        })
    }
    /// Infinite effects remain registered for demand sampling, even though
    /// their next lifecycle boundary is absent and no timer should be armed.
    pub fn needs_timeline_service(&self, now: Instant) -> bool {
        self.timeline_attached
            && (self.next_deadline(now).is_some()
                || (!self.idle
                    && !self.clock.is_paused()
                    && self.clock.rate != 0.0
                    && self.play_state(now) == "running"))
    }
    pub fn next_deadline(&self, now: Instant) -> Option<Instant> {
        if !self.timeline_attached {
            return None;
        }
        if self.is_pending() {
            return self.timeline_attached.then_some(now);
        }
        if self.idle {
            return None;
        }
        // Even a paused effect can have a newly observed phase. Commit it once
        // on the next Document turn; a stable paused effect has no timer.
        if self.clock.is_paused() || self.clock.rate == 0.0 {
            return (self.current_seconds(now).is_some() && self.phase != self.current_phase(now))
                .then_some(now);
        }
        // An overdue wake is necessary until the phase/finish notification has
        // been committed. Afterwards neither playback direction may re-arm it.
        if self.play_state(now) == "finished"
            && self.finish_notified
            && self.phase == self.current_phase(now)
        {
            return None;
        }
        let (before, after) = self.phase_boundaries();
        let boundary = if self.clock.rate < 0.0 {
            match self.phase {
                AnimationPhase::After => after,
                AnimationPhase::Active => before,
                AnimationPhase::Before => 0.0,
            }
        } else {
            match self.phase {
                AnimationPhase::Before => before,
                AnimationPhase::Active => after,
                AnimationPhase::After => self.end_time(),
            }
        };
        let boundary = CssTime::from_seconds(boundary)?;
        self.clock.deadline(boundary)
    }
}
// Deadline resolution is nanoseconds. Comparing finite boundaries at that
// resolution avoids a floating subtraction landing just above a reverse
// boundary. Infinite and out-of-Duration-range times retain their real order.
fn compare_time(left: f64, right: f64) -> std::cmp::Ordering {
    let limit = Duration::MAX.as_secs_f64();
    if left.is_finite()
        && right.is_finite()
        && left.abs() < limit
        && right.abs() < limit
        && let (Some(left), Some(right)) =
            (CssTime::from_seconds(left), CssTime::from_seconds(right))
    {
        return left.cmp(&right);
    }
    left.total_cmp(&right)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaybackNotificationKind {
    Ready,
    Finish,
    Cancel,
    Reset,
    Release,
}
impl PlaybackNotificationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Finish => "finish",
            Self::Cancel => "cancel",
            Self::Reset => "reset",
            Self::Release => "release",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaybackNotification {
    pub kind: PlaybackNotificationKind,
    cycle: u64,
    ready: u64,
    finished: u64,
    synchronous_finish: bool,
    ready_update: ReadyPromiseUpdate,
}
impl PlaybackNotification {
    /// Native cancellation follows the same ready contract as a JS command.
    pub fn ready_update(self) -> ReadyPromiseUpdate {
        self.ready_update
    }
    /// A synchronous finish/cancel has queued its DOM event before a later
    /// state change. An ordinary finish must first pass publication checks.
    pub fn event_is_queued(self) -> bool {
        self.kind == PlaybackNotificationKind::Cancel
            || (self.kind == PlaybackNotificationKind::Finish && self.synchronous_finish)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnimationCommand {
    Play,
    Pause,
    Cancel,
    Finish,
    Seek(f64),
    SetRate(f64),
    Reverse,
    AttachTimeline(bool),
    SetStartTime {
        origin: Instant,
        seconds: Option<f64>,
    },
}
/// The ready-promise transition belongs to the accepted playback operation.
/// Hosts must not infer it from a method name or replace an existing pending promise.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadyPromiseUpdate {
    Unchanged,
    Pending,
    Resolve,
    Reject,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishedPromiseUpdate {
    Unchanged,
    Resolve,
    RejectAndRenew,
}

/// Immediate host actions of an accepted command. Timed playback and CSS
/// notifications remain in advance(); a host must not recreate command rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub struct CommandOutcome {
    pub ready: ReadyPromiseUpdate,
    pub finished: FinishedPromiseUpdate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidPlaybackState;
impl std::fmt::Display for InvalidPlaybackState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the playback operation is invalid in the current animation state")
    }
}
impl std::error::Error for InvalidPlaybackState {}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdvanceMode {
    /// Observe playback notifications without committing CSS phases or pending play tasks.
    Observe,
    /// Service the timeline, including pending play tasks.
    Tick,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhaseEvent {
    Start,
    End,
    Cancel,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnimationEvent {
    Run { elapsed: f64 },
    Phase { kind: PhaseEvent, elapsed: f64 },
    Playback(PlaybackNotification),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimationSample {
    pub id: AnimationId,
    pub phase: AnimationPhase,
    pub play_state: &'static str,
    pub local_time: Option<f64>,
    pub progress: Option<EffectProgress>,
    pub pending: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_late_scheduler_still_services_a_committed_active_phase() {
        let now = Instant::now();
        let mut effect = AnimationState::new(now);
        effect.has_effect = true;
        effect.idle = false;
        effect.duration = Duration::from_millis(80);
        effect.delay = CssTime::from_seconds(-0.04).unwrap();
        effect.phase = AnimationPhase::Active;
        assert_eq!(
            effect.current_phase(now + Duration::from_secs(10)),
            AnimationPhase::After
        );
        assert_eq!(
            effect.next_deadline(now + Duration::from_secs(10)),
            Some(now + Duration::from_millis(40))
        );
        effect
            .clock
            .update_play_state(now + Duration::from_secs(10), false);
        assert_eq!(
            effect.next_deadline(now + Duration::from_secs(10)),
            Some(now + Duration::from_secs(10))
        );
    }
}
