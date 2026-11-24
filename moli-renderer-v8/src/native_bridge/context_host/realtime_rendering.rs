use std::time::{Duration, Instant};

use moli_layout::LayoutFlushReason;

use super::{OwnerDispatchScope, PageScriptHost, WindowDocumentOwner, WindowDocumentTaskTarget};
use crate::frame_owner_model::FrameDocumentTaskOwner;
use crate::page_task_queue::RendererPageRenderingUpdateTaskId;

fn realtime_frame_interval(max_fps: u32) -> Option<Duration> {
    let fps = std::num::NonZeroU32::new(max_fps)?;
    Some(Duration::from_nanos(
        1_000_000_000u64.div_ceil(u64::from(fps.get())),
    ))
}

fn next_frame_deadline(
    previous: Option<Instant>,
    now: Instant,
    interval: Option<Duration>,
) -> Instant {
    let Some(interval) = interval else {
        return now;
    };
    let next = previous.unwrap_or(now) + interval;
    if now <= next {
        return next;
    }
    // Preserve the active burst's cadence while skipping elapsed ticks in
    // constant time. A tick exactly at `now` is eligible; don't replay any of
    // the earlier opportunities missed by a long script or layout pass.
    let period = interval.as_nanos();
    let phase = now.duration_since(next).as_nanos() % period;
    now + Duration::from_nanos(((period - phase) % period) as u64)
}

#[derive(Clone, Copy, Debug)]
struct ScheduledRealtimeFrame {
    id: RendererPageRenderingUpdateTaskId,
    target: WindowDocumentTaskTarget,
    deadline: Instant,
    sample_all_intersections: bool,
}

struct RunningRealtimeFrame {
    ticket: (RendererPageRenderingUpdateTaskId, WindowDocumentTaskTarget),
    // The root is first. Only an explicit document.open transaction may move
    // these exact targets: it preserves the Web Document and Window, while
    // replacing Moli's task owner. Navigation never transfers this batch.
    documents: Vec<WindowDocumentTaskTarget>,
    sample_all_intersections: bool,
    source_changed: bool,
}

#[derive(Default)]
enum FramePhase {
    #[default]
    Idle,
    Scheduled(ScheduledRealtimeFrame),
    Running(RunningRealtimeFrame),
}

#[cfg(feature = "molio")]
struct ViewportDisplayClock {
    revision: u64,
    requests: tokio::sync::watch::Sender<Option<u64>>,
    requested: bool,
    tick: Option<Instant>,
    consumed: Option<Instant>,
}

#[cfg(feature = "molio")]
impl ViewportDisplayClock {
    fn set_requested(&mut self, requested: bool) {
        if self.requested == requested {
            return;
        }
        self.requested = requested;
        if !requested {
            self.tick = None;
            self.consumed = None;
        }
        self.requests
            .send_replace(requested.then_some(self.revision));
    }
}

#[cfg(feature = "molio")]
impl Drop for ViewportDisplayClock {
    fn drop(&mut self) {
        self.set_requested(false);
    }
}

#[derive(Default)]
pub(super) struct RealtimeFrameState {
    next_id: u64,
    phase: FramePhase,
    // Realtime-only metadata for an active burst. Idle/canceled/retired work
    // drops the cadence without creating a persistent wake-up source.
    last_deadline: Option<Instant>,
    max_fps: Option<u32>,
    #[cfg(feature = "molio")]
    display_clock: Option<ViewportDisplayClock>,
    source_requested: bool,
    #[cfg(test)]
    now_for_test: Option<Instant>,
    #[cfg(test)]
    completed_for_test: Option<tokio::sync::oneshot::Sender<()>>,
}

impl RealtimeFrameState {
    fn sampled_now(&self) -> Instant {
        #[cfg(test)]
        if let Some(now) = self.now_for_test {
            return now;
        }
        Instant::now()
    }

    fn next_frame_deadline(&self) -> Instant {
        let now = self.sampled_now();
        #[cfg(feature = "molio")]
        if let Some(clock) = &self.display_clock
            && let Some(tick) = clock.tick
        {
            if clock.consumed != Some(tick) {
                return now;
            }
            // Display callbacks can stop during a native resize or occlusion.
            // Resume timer pacing until the window publishes another tick.
            let timeout = tick + Duration::from_millis(250);
            if timeout > now {
                return timeout;
            }
        }
        let max_fps = self
            .max_fps
            .unwrap_or(moli_page_types::DEFAULT_REALTIME_MAX_FPS);
        next_frame_deadline(self.last_deadline, now, realtime_frame_interval(max_fps))
    }

    pub(super) fn is_idle(&self) -> bool {
        matches!(self.phase, FramePhase::Idle)
    }

    fn scheduled(&self) -> Option<ScheduledRealtimeFrame> {
        match self.phase {
            FramePhase::Scheduled(frame) => Some(frame),
            _ => None,
        }
    }

    fn running(&self) -> Option<&RunningRealtimeFrame> {
        match &self.phase {
            FramePhase::Running(frame) => Some(frame),
            _ => None,
        }
    }

    #[cfg(feature = "molio")]
    fn publish_display_tick_request(&mut self) {
        let requested = match &self.phase {
            FramePhase::Idle => false,
            FramePhase::Scheduled(frame) => frame.sample_all_intersections,
            FramePhase::Running(frame) => frame.sample_all_intersections,
        };
        if let Some(clock) = &mut self.display_clock {
            clock.set_requested(requested);
        }
    }

    fn clear_cadence(&mut self) {
        self.last_deadline = None;
        #[cfg(feature = "molio")]
        self.publish_display_tick_request();
    }

    pub(super) fn retire_document(&mut self) {
        // Do not reuse ids across document.open(), or a stale claimed task
        // could retire the replacement's payload during discard.
        // The running body can outlive its Document through author callbacks.
        // Only that body's settlement may release the non-reentrancy guard.
        if matches!(self.phase, FramePhase::Scheduled(_)) {
            self.phase = FramePhase::Idle;
        }
        self.clear_cadence();
        self.source_requested = false;
        #[cfg(test)]
        self.completed_for_test.take();
    }
}

impl PageScriptHost {
    pub(super) fn rebind_rendering_document_after_open(
        &mut self,
        retired: FrameDocumentTaskOwner,
        current: FrameDocumentTaskOwner,
    ) {
        debug_assert_eq!(retired.local_window_id, current.local_window_id);
        let retired = WindowDocumentOwner::Frame(retired);
        let current = WindowDocumentOwner::Frame(current);
        let pending_media = self.media_queries.borrow_mut().rebind(retired, current);
        let Some(state) = &self.realtime_layout_state else {
            // The old exact-owner environment task cannot carry sampling work
            // into its successor. Re-admit it through the existing driver.
            if pending_media {
                self.queue_environment_change();
            }
            return;
        };
        state.environment.borrow_mut().rebind(retired, current);
        self.rebind_realtime_font_readiness(retired, current);
        self.rebind_realtime_rendering_events(retired, current);
        self.rebind_realtime_animation_frame_document(retired, current);
        crate::observer_runtime::rebind_realtime_resize_observer_document(self, retired, current);
        {
            let mut frame = state.frame.borrow_mut();
            // Unstarted tasks still need a new exact-owner ticket. A running
            // body keeps its original ticket and non-reentrancy guard, but
            // follows the explicitly preserved Document in later phases.
            match &mut frame.phase {
                FramePhase::Scheduled(scheduled) if scheduled.target.owner() == retired => {
                    frame.phase = FramePhase::Idle;
                }
                FramePhase::Running(running) => {
                    for target in &mut running.documents {
                        if target.owner() == retired {
                            *target =
                                WindowDocumentTaskTarget::new(current, target.dispatch_scope());
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn retire_realtime_rendering_document(&self, retired: WindowDocumentOwner) {
        if self.realtime_layout_state.is_none() {
            return;
        }
        self.retire_realtime_animation_frame_document(retired);
        self.retire_realtime_font_readiness(retired);
        if let Some(state) = &self.realtime_layout_state {
            state
                .environment
                .borrow_mut()
                .retain(|target| target.owner() != retired);
            state
                .rendering_events
                .borrow_mut()
                .retain(|pending| pending.target().owner() != retired);
        }
        crate::observer_runtime::retire_realtime_resize_observer_document(self, retired);
        self.refresh_realtime_rendering_opportunity();
    }

    #[cfg(test)]
    pub(crate) fn set_realtime_opportunity_clock_for_test(&self, now: Instant) {
        self.realtime_layout_state
            .as_ref()
            .expect("Realtime test clock")
            .frame
            .borrow_mut()
            .now_for_test = Some(now);
    }

    #[cfg(test)]
    pub(crate) fn observe_next_realtime_opportunity_for_test(
        &self,
        completed: tokio::sync::oneshot::Sender<()>,
    ) {
        self.realtime_layout_state
            .as_ref()
            .expect("Realtime test observer")
            .frame
            .borrow_mut()
            .completed_for_test = Some(completed);
    }

    /// Source commits register semantic demand; the stable Page source owns
    /// deadline residence. This neither computes layout nor calls page JS.
    pub(crate) fn request_realtime_rendering_opportunity(&self) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        let already_requested = {
            let mut frame = state.frame.borrow_mut();
            if let FramePhase::Running(running) = &mut frame.phase {
                running.sample_all_intersections = true;
                running.source_changed = true;
            }
            std::mem::replace(&mut frame.source_requested, true)
        };
        if !already_requested {
            crate::observer_runtime::request_realtime_resize_observer_checks(self);
        }
        self.refresh_realtime_rendering_opportunity();
    }

    #[cfg(feature = "molio")]
    pub(crate) fn set_viewport_display_clock(
        &self,
        clock: Option<(u64, tokio::sync::watch::Sender<Option<u64>>)>,
    ) {
        if let Some(state) = &self.realtime_layout_state {
            let mut frame = state.frame.borrow_mut();
            frame.display_clock = clock.map(|(revision, requests)| ViewportDisplayClock {
                revision,
                requests,
                requested: false,
                tick: None,
                consumed: None,
            });
            frame.publish_display_tick_request();
        }
    }

    /// Returns whether the owner needs to reindex the frame deadline.
    #[cfg(feature = "molio")]
    pub(crate) fn accept_viewport_display_tick(&self, time: Instant) -> bool {
        let Some(state) = &self.realtime_layout_state else {
            return false;
        };
        let mut frame = state.frame.borrow_mut();
        let Some(clock) = frame.display_clock.as_mut().filter(|clock| clock.requested) else {
            return false;
        };
        if clock.tick.is_some_and(|previous| previous >= time) {
            return false;
        }
        clock.tick = Some(time);
        let now = frame.sampled_now();
        if !frame
            .scheduled()
            .is_some_and(|scheduled| scheduled.sample_all_intersections && scheduled.deadline > now)
        {
            return false;
        }
        // Replace a timer deadline with this display opportunity.
        frame.phase = FramePhase::Idle;
        drop(frame);
        self.refresh_realtime_rendering_opportunity();
        true
    }

    pub(crate) fn set_realtime_max_fps(&self, max_fps: u32) {
        if let Some(state) = &self.realtime_layout_state {
            let mut frame = state.frame.borrow_mut();
            if frame.max_fps != Some(max_fps) {
                frame.max_fps = Some(max_fps);
                if matches!(frame.phase, FramePhase::Scheduled(_)) {
                    frame.phase = FramePhase::Idle;
                }
                frame.clear_cadence();
            }
            drop(frame);
            self.refresh_realtime_rendering_opportunity();
        }
    }

    /// Reconcile semantic demand with the single deadline residence. rAF
    /// registration/cancellation does not manufacture a layout source change.
    pub(crate) fn refresh_realtime_rendering_opportunity(&self) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        // Policy is installed before Page task capabilities during bootstrap.
        // The capability installation requests the initial opportunity once
        // the stable owner route exists; source commits still update inputs.
        if self.page_task_capabilities.get().is_none() {
            return;
        }
        let Some(target) =
            self.current_window_document_task_target_for_dispatch_scope(OwnerDispatchScope::Top)
        else {
            return;
        };
        if matches!(state.frame.borrow().phase, FramePhase::Running(_)) {
            // This also covers document.open during a callback. There can
            // never be a second opportunity nested inside the running body.
            return;
        }
        let callbacks_pending = self.has_runnable_realtime_animation_frames();
        let resize_pending =
            crate::observer_runtime::has_pending_realtime_resize_observers(self, None);
        let environment_pending = self.has_realtime_environment_notifications();
        let events_pending = self.has_realtime_rendering_events();
        let mut frame = state.frame.borrow_mut();
        if !self.document_activity().visible {
            frame.phase = FramePhase::Idle;
            frame.clear_cadence();
            return;
        }
        let sample_all_intersections = frame.source_requested
            || callbacks_pending
            || resize_pending
            || environment_pending
            || events_pending;
        let pending = frame.scheduled().filter(|pending| pending.target == target);
        if sample_all_intersections
            && pending.is_some_and(|pending| pending.sample_all_intersections)
        {
            return;
        }
        let deadline = if sample_all_intersections {
            let tick = frame.next_frame_deadline();
            Some(pending.map_or(tick, |pending| tick.min(pending.deadline)))
        } else {
            crate::observer_runtime::next_realtime_intersection_deadline(self)
        };
        let Some(deadline) = deadline else {
            frame.phase = FramePhase::Idle;
            frame.clear_cadence();
            return;
        };
        if let Some(mut pending) = pending.filter(|pending| pending.deadline == deadline) {
            pending.sample_all_intersections = sample_all_intersections;
            frame.phase = FramePhase::Scheduled(pending);
            frame.last_deadline = sample_all_intersections.then_some(deadline);
            #[cfg(feature = "molio")]
            frame.publish_display_tick_request();
            return;
        }
        frame.phase = FramePhase::Idle;
        let Some(next_id) = frame.next_id.checked_add(1) else {
            frame.clear_cadence();
            tracing::error!("realtime rendering opportunity id exhausted");
            return;
        };
        frame.next_id = next_id;
        let id = RendererPageRenderingUpdateTaskId::Frame(next_id);
        frame.phase = FramePhase::Scheduled(ScheduledRealtimeFrame {
            id,
            target,
            deadline,
            sample_all_intersections,
        });
        if self
            .page_rendering_update_sender()
            .schedule_frame(target, id, deadline)
            .is_err()
        {
            frame.phase = FramePhase::Idle;
            frame.last_deadline = None;
            state.rendering_events.borrow_mut().retain(|_| false);
            tracing::debug!(
                ?target,
                ?id,
                "retired realtime opportunity after route closure"
            );
        } else {
            // Only a burst of ordinary work sets the cadence. An IO deadline
            // can have an arbitrary offset from a previous rendering tick.
            frame.last_deadline = sample_all_intersections.then_some(deadline);
        }
        #[cfg(feature = "molio")]
        frame.publish_display_tick_request();
    }

    #[cfg(all(test, feature = "devtools"))]
    pub(crate) fn realtime_frame_is_idle_for_test(&self) -> bool {
        self.realtime_layout_state.as_ref().is_some_and(|state| {
            let frame = state.frame.borrow();
            frame.is_idle() && !frame.source_requested
        })
    }

    pub(crate) fn current_realtime_rendering_opportunity(
        &self,
    ) -> Option<(RendererPageRenderingUpdateTaskId, WindowDocumentTaskTarget)> {
        let pending = self
            .realtime_layout_state
            .as_ref()?
            .frame
            .borrow()
            .scheduled()?;
        self.window_document_owner_is_current_for_dispatch_scope(
            pending.target.owner(),
            pending.target.dispatch_scope(),
        )
        .then_some((pending.id, pending.target))
    }

    pub(crate) fn discard_realtime_rendering_opportunity(
        &self,
        id: RendererPageRenderingUpdateTaskId,
    ) -> bool {
        let Some(state) = &self.realtime_layout_state else {
            return false;
        };
        let mut frame = state.frame.borrow_mut();
        if frame.scheduled().is_some_and(|pending| pending.id == id) {
            frame.phase = FramePhase::Idle;
            #[cfg(feature = "molio")]
            frame.publish_display_tick_request();
            true
        } else {
            false
        }
    }

    pub(crate) fn begin_realtime_rendering_opportunity(
        &self,
        id: RendererPageRenderingUpdateTaskId,
        target: WindowDocumentTaskTarget,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(
            self.current_realtime_rendering_opportunity() == Some((id, target)),
            "authorized realtime opportunity lost its exact payload"
        );
        let state = self.realtime_layout_state.as_ref().expect("Realtime frame");
        let mut frame = state.frame.borrow_mut();
        let scheduled = frame.scheduled().expect("authorized rendering opportunity");
        frame.phase = FramePhase::Idle;
        if !self.document_activity().visible {
            frame.clear_cadence();
            return Ok(false);
        }
        let documents = self.realtime_animation_frame_documents();
        if documents.first() != Some(&target) {
            frame.clear_cadence();
            anyhow::bail!("rendering root changed before its batch");
        }
        #[cfg(feature = "molio")]
        if scheduled.sample_all_intersections
            && let Some(clock) = &mut frame.display_clock
        {
            clock.consumed = clock.tick;
        }
        frame.phase = FramePhase::Running(RunningRealtimeFrame {
            ticket: (id, target),
            documents,
            sample_all_intersections: scheduled.sample_all_intersections,
            source_changed: frame.source_requested,
        });
        Ok(true)
    }

    pub(crate) fn realtime_intersection_sample(&self) -> (Instant, bool) {
        let frame = self
            .realtime_layout_state
            .as_ref()
            .expect("Realtime frame")
            .frame
            .borrow();
        let running = frame.running().expect("running Realtime frame");
        (frame.sampled_now(), running.sample_all_intersections)
    }

    pub(crate) fn realtime_opportunity_has_source_changes(&self) -> bool {
        self.realtime_layout_state
            .as_ref()
            .expect("Realtime frame state")
            .frame
            .borrow()
            .running()
            .expect("running Realtime frame")
            .source_changed
    }

    pub(crate) fn realtime_rendering_opportunity_is_current(
        &self,
        id: RendererPageRenderingUpdateTaskId,
        target: WindowDocumentTaskTarget,
    ) -> bool {
        let Some(root) = self.realtime_rendering_document(id, target, 0) else {
            return false;
        };
        self.window_document_owner_is_current_for_dispatch_scope(
            root.owner(),
            root.dispatch_scope(),
        )
    }

    pub(crate) fn realtime_rendering_document(
        &self,
        id: RendererPageRenderingUpdateTaskId,
        root: WindowDocumentTaskTarget,
        index: usize,
    ) -> Option<WindowDocumentTaskTarget> {
        let frame = self.realtime_layout_state.as_ref()?.frame.borrow();
        let running = frame.running()?;
        (running.ticket == (id, root))
            .then(|| running.documents.get(index).copied())
            .flatten()
    }

    /// No page callbacks are invoked while native style/layout borrows exist.
    pub(crate) fn run_realtime_layout_phase(
        &self,
        id: RendererPageRenderingUpdateTaskId,
        target: WindowDocumentTaskTarget,
    ) -> anyhow::Result<()> {
        if !self.realtime_rendering_opportunity_is_current(id, target) {
            return Ok(());
        }
        // This phase consumes all source changes preceding it, including
        // writes in rAF and its microtasks. Synchronous getters did not consume
        // lifecycle demand, but their geometry can satisfy this phase.
        self.realtime_layout_state
            .as_ref()
            .expect("Realtime frame")
            .frame
            .borrow_mut()
            .source_requested = false;
        // The shared freshness gate also publishes an empty result when the
        // root was removed, retiring the previous geometry and computation.
        self.ensure_current_screen_layout(LayoutFlushReason::RenderingOpportunity)?;
        #[cfg(test)]
        if let Some(completed) = self
            .realtime_layout_state
            .as_ref()
            .and_then(|state| state.frame.borrow_mut().completed_for_test.take())
        {
            let _ = completed.send(());
        }
        Ok(())
    }

    pub(crate) fn finish_realtime_rendering_opportunity(
        &self,
        id: RendererPageRenderingUpdateTaskId,
        target: WindowDocumentTaskTarget,
        succeeded: bool,
    ) {
        let Some(state) = &self.realtime_layout_state else {
            return;
        };
        {
            let mut frame = state.frame.borrow_mut();
            let Some(running) = frame
                .running()
                .filter(|running| running.ticket == (id, target))
            else {
                // Callback navigation may already have installed a new
                // document's request. Never settle it with the old ticket.
                return;
            };
            let current_root = running.documents[0];
            frame.phase = FramePhase::Idle;
            if !succeeded
                && self.window_document_owner_is_current_for_dispatch_scope(
                    current_root.owner(),
                    current_root.dispatch_scope(),
                )
            {
                // Retain uncalled callbacks but require new explicit demand
                // after a fatal layout/checkpoint/watchdog failure.
                frame.source_requested = false;
                frame.clear_cadence();
                return;
            }
        }
        self.prune_realtime_animation_frames();
        self.prune_media_queries();
        self.prune_realtime_environment();
        self.prune_realtime_rendering_events();
        crate::observer_runtime::prune_realtime_resize_observers(self);
        self.refresh_realtime_rendering_opportunity();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_frame_deadline_accepts_current_ticks_and_skips_elapsed_ticks() {
        let anchor = Instant::now();
        let period = realtime_frame_interval(moli_page_types::DEFAULT_REALTIME_MAX_FPS).unwrap();
        assert_eq!(period, Duration::from_nanos(16_666_667));
        assert_eq!(
            next_frame_deadline(None, anchor, Some(period)),
            anchor + period
        );
        for (elapsed, expected) in [
            (Duration::ZERO, period),
            (period, period),
            (period + Duration::from_nanos(1), period * 2),
            (period * 60_000, period * 60_000),
            (period * 60_000 + Duration::from_nanos(1), period * 60_001),
        ] {
            assert_eq!(
                next_frame_deadline(Some(anchor), anchor + elapsed, Some(period)),
                anchor + expected,
                "elapsed: {elapsed:?}"
            );
        }
    }

    #[test]
    fn configured_realtime_frame_interval_supports_high_refresh_and_unlimited() {
        let now = Instant::now();
        let fast = realtime_frame_interval(120);
        assert_eq!(fast, Some(Duration::from_nanos(8_333_334)));
        assert_eq!(next_frame_deadline(None, now, fast), now + fast.unwrap());
        assert_eq!(
            next_frame_deadline(Some(now), now, realtime_frame_interval(0)),
            now
        );
    }

    #[cfg(feature = "molio")]
    #[test]
    fn viewport_display_clock_consumes_each_tick_once_and_resumes_timer_pacing() {
        let tick = Instant::now();
        let (requests, needed) = tokio::sync::watch::channel(None);
        let mut frame = RealtimeFrameState {
            now_for_test: Some(tick),
            display_clock: Some(ViewportDisplayClock {
                revision: 1,
                requests,
                requested: true,
                tick: Some(tick),
                consumed: None,
            }),
            ..Default::default()
        };
        assert_eq!(frame.next_frame_deadline(), tick);
        frame.display_clock.as_mut().unwrap().consumed = Some(tick);
        let fallback = tick + Duration::from_millis(250);
        assert_eq!(frame.next_frame_deadline(), fallback);
        frame.now_for_test = Some(fallback);
        assert_eq!(
            frame.next_frame_deadline(),
            fallback + Duration::from_nanos(16_666_667)
        );
        let resumed = fallback + Duration::from_millis(10);
        frame.now_for_test = Some(resumed);
        frame.display_clock.as_mut().unwrap().tick = Some(resumed);
        assert_eq!(frame.next_frame_deadline(), resumed);

        frame.clear_cadence();
        assert_eq!(*needed.borrow(), None);
        assert_eq!(
            frame.next_frame_deadline(),
            resumed + Duration::from_nanos(16_666_667)
        );
    }
}
