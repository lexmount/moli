use super::*;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum MediaPlaybackBoundary {
    ForwardEnd,
    BackwardStart,
}

impl JsContextHost {
    pub(super) fn media_playback_boundary(
        &self,
        media: DomHandle,
    ) -> Option<MediaPlaybackBoundary> {
        let element = self.dom_host().node(media)?.as_element()?;
        if element.media_ready_state() < 1 || element.media_seeking() {
            return None;
        }
        if element.media_playback_rate() < 0.0 {
            return (element.media_current_time() == 0.0)
                .then_some(MediaPlaybackBoundary::BackwardStart);
        }
        let duration = self.media_duration(media)?;
        (duration.is_finite() && element.media_current_time() >= duration)
            .then_some(MediaPlaybackBoundary::ForwardEnd)
    }

    pub(crate) fn media_ended(&self, media: DomHandle) -> bool {
        self.media_playback_boundary(media) == Some(MediaPlaybackBoundary::ForwardEnd)
            && self
                .dom_host()
                .node(media)
                .and_then(crate::dom::native::Node::as_element)
                .is_some_and(|element| element.attribute("loop").is_none())
    }

    pub(super) fn clear_media_boundary_if_moved(&mut self, media: DomHandle) {
        let boundary = self.media_playback_boundary(media);
        if let Some(state) = self.media_playback.elements.get_mut(&media)
            && state.boundary_queued != boundary
        {
            state.boundary_queued = None;
        }
    }

    pub(super) fn queue_media_boundary_event(&mut self, media: DomHandle) {
        let Some(boundary) = self.media_playback_boundary(media) else {
            self.clear_media_boundary_if_moved(media);
            return;
        };
        if boundary == MediaPlaybackBoundary::ForwardEnd
            && self
                .dom_host()
                .node(media)
                .and_then(crate::dom::native::Node::as_element)
                .is_some_and(|element| element.attribute("loop").is_some())
        {
            return;
        }
        let Some(state) = self.media_playback.elements.get_mut(&media) else {
            return;
        };
        if state.boundary_queued == Some(boundary) {
            return;
        }
        let Some(target) = state.target else {
            return;
        };
        state.boundary_queued = Some(boundary);
        let Some(task) = self.capture_media_playback_task(
            media,
            PlaybackOperation::Boundary(boundary),
            Vec::new(),
        ) else {
            return;
        };
        self.queue_media_element_event_for_target(
            target,
            RendererPageMediaElementEventTaskKind::Playback,
            super::super::media_element_events::PendingMediaElementEventPayload::Playback(task),
        );
    }

    // Seeking updates DOM and text-track state; do not retain a host borrow
    // across those updates when a loop starts from a clock or seek task.
    pub(crate) fn process_media_playback_boundary(
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut Self,
        media: DomHandle,
    ) {
        let looping = unsafe { &*host }.media_playback_boundary(media)
            == Some(MediaPlaybackBoundary::ForwardEnd)
            && unsafe { &*host }
                .dom_host()
                .node(media)
                .and_then(crate::dom::native::Node::as_element)
                .is_some_and(|element| element.attribute("loop").is_some());
        if looping {
            element::seek_media_to(scope, host, media, 0.0);
        } else {
            unsafe { &mut *host }.queue_media_boundary_event(media);
        }
    }

    pub(crate) fn rewind_media_for_play(
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut Self,
        media: DomHandle,
    ) {
        if unsafe { &*host }.media_playback_boundary(media)
            == Some(MediaPlaybackBoundary::ForwardEnd)
        {
            // A loop attribute can make ended false while the position is
            // still at EOF. Resuming playback must rewind in either case.
            element::seek_media_to(scope, host, media, 0.0);
        }
    }

    pub(crate) fn notify_media_playing_after_seek(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) {
        if self
            .media_playback
            .elements
            .get(&media)
            .is_some_and(|state| !state.pending.is_empty())
            && self
                .dom_host()
                .node(media)
                .and_then(crate::dom::native::Node::as_element)
                .is_some_and(|element| element.media_ready_state() >= 3)
        {
            self.notify_media_playing(scope, media);
        }
    }
}

pub(super) fn dispatch_media_boundary(
    scope: &mut v8::PinScope<'_, '_>,
    host: *mut JsContextHost,
    media: DomHandle,
    document: DomHandle,
    epoch: u64,
    boundary: MediaPlaybackBoundary,
) -> bool {
    if !unsafe { &*host }.playback_is_current(media, document, epoch) {
        return false;
    }
    fire(scope, host, media, "timeupdate");
    if !unsafe { &*host }.playback_is_current(media, document, epoch) {
        return true;
    }
    if boundary == MediaPlaybackBoundary::BackwardStart {
        return true;
    }
    let pause = unsafe { &*host }.media_ended(media)
        && unsafe { &*host }
            .dom_host()
            .node(media)
            .and_then(crate::dom::native::Node::as_element)
            .is_some_and(|element| !element.media_paused());
    if pause {
        unsafe { &mut *host }.set_media_paused(media, true);
        fire(scope, host, media, "pause");
        if !unsafe { &*host }.playback_is_current(media, document, epoch) {
            return true;
        }
        let promises = unsafe { &mut *host }.take_media_play_promises(media);
        for promise in promises {
            promise.settle(scope, Some(MediaPlayError::Abort));
        }
    }
    if unsafe { &*host }.playback_is_current(media, document, epoch) {
        fire(scope, host, media, "ended");
    }
    true
}
