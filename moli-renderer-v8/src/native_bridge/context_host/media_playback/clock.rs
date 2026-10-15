use super::*;
use crate::{
    document_runtime::DocumentRuntime,
    native_bridge::context_host::{
        media_element_events::PendingMediaElementEventPayload,
        window_document_tasks::PendingExactWindowDocumentTask,
    },
};
use std::time::{Duration, Instant};

const UPDATE_INTERVAL: Duration = Duration::from_millis(250);

pub(super) struct MediaPlaybackClock {
    anchor: Instant,
    position: f64,
    rate: f64,
    running: bool,
    pending_task: Option<RendererPageMediaElementEventTaskId>,
    wakeup: Option<tokio::task::JoinHandle<()>>,
}

impl MediaPlaybackClock {
    fn new(position: f64, rate: f64, running: bool) -> Self {
        Self {
            anchor: Instant::now(),
            position,
            rate,
            running,
            pending_task: None,
            wakeup: None,
        }
    }

    fn position_at(&self, now: Instant) -> f64 {
        if self.running {
            (self.position + now.saturating_duration_since(self.anchor).as_secs_f64() * self.rate)
                .clamp(0.0, f64::MAX)
        } else {
            self.position
        }
    }

    fn rebase(&mut self, position: f64, rate: f64, running: bool, now: Instant) {
        self.anchor = now;
        self.position = position;
        self.rate = rate;
        self.running = running;
    }

    pub(super) fn is_current_tick(&self, task: RendererPageMediaElementEventTaskId) -> bool {
        self.running && self.rate != 0.0 && self.pending_task == Some(task)
    }
}

impl Drop for MediaPlaybackClock {
    fn drop(&mut self) {
        if let Some(wakeup) = self.wakeup.take() {
            wakeup.abort();
        }
    }
}

impl JsContextHost {
    fn media_clock_controls(&self, media: DomHandle) -> Option<(f64, f64, bool)> {
        let element = self.dom_host().node(media)?.as_element()?;
        Some((
            element.media_current_time(),
            element.media_playback_rate(),
            !element.media_paused() && !element.media_seeking() && element.media_ready_state() >= 3,
        ))
    }

    pub(super) fn start_media_clock(&mut self, media: DomHandle) {
        self.synchronize_media_clock(media);
        let Some((position, rate, running)) = self.media_clock_controls(media) else {
            return;
        };
        let Some(state) = self.media_playback.elements.get_mut(&media) else {
            return;
        };
        if state.clock.is_none() {
            state.clock = Some(MediaPlaybackClock::new(position, rate, running));
        }
        self.arm_media_clock_tick(media);
    }

    pub(crate) fn synchronize_media_clock(&mut self, media: DomHandle) {
        let Some((_, rate, running)) = self.media_clock_controls(media) else {
            return;
        };
        let Some(clock) = self
            .media_playback
            .elements
            .get_mut(&media)
            .and_then(|state| state.clock.as_mut())
        else {
            return;
        };
        let now = Instant::now();
        let position = clock.position_at(now);
        clock.rebase(position, rate, running, now);
        let _ =
            unsafe { DocumentRuntime::set_media_current_time(&mut *self.runtime, media, position) };
        if !running || rate == 0.0 {
            self.cancel_media_clock_tick(media);
        }
    }

    pub(crate) fn rebase_media_clock_from_element(&mut self, media: DomHandle) {
        let Some((position, rate, running)) = self.media_clock_controls(media) else {
            return;
        };
        let Some(clock) = self
            .media_playback
            .elements
            .get_mut(&media)
            .and_then(|state| state.clock.as_mut())
        else {
            return;
        };
        clock.rebase(position, rate, running, Instant::now());
        if !running || rate == 0.0 {
            self.cancel_media_clock_tick(media);
        } else {
            self.arm_media_clock_tick(media);
        }
    }

    fn cancel_media_clock_tick(&mut self, media: DomHandle) {
        let Some(clock) = self
            .media_playback
            .elements
            .get_mut(&media)
            .and_then(|state| state.clock.as_mut())
        else {
            return;
        };
        if let Some(wakeup) = clock.wakeup.take() {
            wakeup.abort();
        }
        if let Some(task) = clock.pending_task.take() {
            let _ = self.media_element_events.remove(task);
        }
    }

    pub(super) fn stop_media_clock(&mut self, media: DomHandle) {
        self.synchronize_media_clock(media);
        self.cancel_media_clock_tick(media);
        if let Some(state) = self.media_playback.elements.get_mut(&media) {
            state.clock = None;
        }
    }

    fn arm_media_clock_tick(&mut self, media: DomHandle) {
        let Some(state) = self.media_playback.elements.get(&media) else {
            return;
        };
        let Some(clock) = state.clock.as_ref() else {
            return;
        };
        if !clock.running || clock.rate == 0.0 || clock.pending_task.is_some() {
            return;
        }
        let Some(target) = state.target else {
            return;
        };
        if self.current_window_document_task_target_for_dispatch_scope(target.dispatch_scope())
            != Some(target)
        {
            return;
        }
        let Some(loader) = self.document_resource_loader_for_window_owner(target.owner()) else {
            return;
        };
        let task_id = self
            .media_element_events
            .allocate_task_id(RendererPageMediaElementEventTaskId::from_raw);
        let Some(task) = self.capture_media_playback_task(
            media,
            PlaybackOperation::ClockTick(task_id),
            Vec::new(),
        ) else {
            return;
        };
        let kind = RendererPageMediaElementEventTaskKind::Playback;
        self.media_element_events
            .push(PendingExactWindowDocumentTask::new(
                task_id,
                target,
                kind,
                PendingMediaElementEventPayload::Playback(task),
            ));
        let sender = self.page_media_element_event_sender().clone();
        let wakeup = loader.task_runner().spawn_abortable(async move {
            tokio::time::sleep(UPDATE_INTERVAL).await;
            let _ = sender.send(target, task_id, kind);
        });
        let clock = self
            .media_playback
            .elements
            .get_mut(&media)
            .and_then(|state| state.clock.as_mut())
            .expect("clock was captured on the owner thread");
        clock.pending_task = Some(task_id);
        clock.wakeup = Some(wakeup);
    }

    pub(crate) fn set_media_current_time(&mut self, media: DomHandle, time: f64) -> bool {
        self.synchronize_media_clock(media);
        let changed =
            unsafe { DocumentRuntime::set_media_current_time(&mut *self.runtime, media, time) };
        self.rebase_media_clock_from_element(media);
        changed
    }

    pub(crate) fn set_media_playback_rate(&mut self, media: DomHandle, rate: f64) -> bool {
        self.synchronize_media_clock(media);
        let changed =
            unsafe { DocumentRuntime::set_media_playback_rate(&mut *self.runtime, media, rate) };
        self.rebase_media_clock_from_element(media);
        changed
    }

    pub(crate) fn set_media_ready_state(&mut self, media: DomHandle, ready: u32) -> bool {
        self.synchronize_media_clock(media);
        let changed =
            unsafe { DocumentRuntime::set_media_ready_state(&mut *self.runtime, media, ready) };
        self.rebase_media_clock_from_element(media);
        changed
    }
}

pub(super) fn dispatch_media_clock_tick(
    scope: &mut v8::PinScope<'_, '_>,
    host: *mut JsContextHost,
    media: DomHandle,
    task_id: RendererPageMediaElementEventTaskId,
) -> bool {
    let (changed, document, epoch) = {
        let runtime = unsafe { &mut *host };
        let Some(clock) = runtime
            .media_playback
            .elements
            .get_mut(&media)
            .and_then(|state| state.clock.as_mut())
        else {
            return false;
        };
        if !clock.is_current_tick(task_id) {
            return false;
        }
        let previous_position = clock.position;
        clock.pending_task = None;
        clock.wakeup = None;
        runtime.synchronize_media_clock(media);
        let changed = runtime
            .dom_host()
            .node(media)
            .and_then(crate::dom::native::Node::as_element)
            .is_some_and(|element| element.media_current_time() != previous_position);
        let state = runtime
            .media_playback
            .elements
            .get(&media)
            .expect("current clock");
        (changed, state.document, state.epoch)
    };
    // Author callbacks can reenter the host, so no mutable host reference may
    // span cue or timeupdate dispatch.
    if changed {
        element::refresh_media_active_text_track_cues(scope, host, media);
        if unsafe { &*host }.playback_is_current(media, document, epoch) {
            fire(scope, host, media, "timeupdate");
        }
    }
    if unsafe { &*host }.playback_is_current(media, document, epoch) {
        unsafe { &mut *host }.arm_media_clock_tick(media);
    }
    changed
}
