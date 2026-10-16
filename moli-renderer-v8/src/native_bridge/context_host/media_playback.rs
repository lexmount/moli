//! Pending play requests and playback tasks share the media element's native
//! identity. Promise realms belong to media objects; task ownership belongs to the
//! element's LocalWindow, independently of parser replacement.

use std::{
    cell::Cell,
    collections::HashMap,
    rc::{Rc, Weak},
};

use super::{JsContextHost, WindowTaskTarget};
use crate::{
    document_runtime::DomHandle,
    native_bridge::element,
    page_task_queue::{RendererPageMediaElementEventTaskId, RendererPageMediaElementEventTaskKind},
};

mod boundary;
mod clock;
mod played;
mod video;
use boundary::MediaPlaybackBoundary;
use clock::MediaPlaybackClock;
use played::MediaPlayedRanges;
pub(super) use video::MediaVideoDecodeTask;

#[derive(Clone, Copy)]
pub(crate) enum MediaPlayError {
    Abort,
    NotSupported,
    NotAllowed,
}

impl MediaPlayError {
    fn name(self) -> &'static str {
        match self {
            Self::Abort => "AbortError",
            Self::NotSupported => "NotSupportedError",
            Self::NotAllowed => "NotAllowedError",
        }
    }
}

struct PlayPromise {
    resolver: v8::Global<v8::PromiseResolver>,
    realm: v8::Global<v8::Context>,
    settled: Cell<bool>,
    queued_result: Cell<Option<Result<(), MediaPlayError>>>,
}

impl PlayPromise {
    fn settle(&self, scope: &mut v8::PinScope<'_, '_>, error: Option<MediaPlayError>) {
        if self.settled.replace(true) {
            return;
        }
        let realm = v8::Local::new(scope, &self.realm);
        let scope = &mut v8::ContextScope::new(scope, realm);
        let resolver = v8::Local::new(scope, &self.resolver);
        if let Some(error) = error {
            let value = crate::context_bootstrap::new_dom_exception_value(
                scope,
                "The media playback request could not complete.",
                error.name(),
            );
            let _ = resolver.reject(scope, value);
        } else {
            let undefined = v8::undefined(scope);
            let _ = resolver.resolve(scope, undefined.into());
        }
    }
}

struct PlaybackElementState {
    document: DomHandle,
    target: Option<WindowTaskTarget>,
    task_document: Option<DomHandle>,
    epoch: u64,
    can_autoplay: bool,
    pending: Vec<Rc<PlayPromise>>,
    outstanding: Vec<Weak<PlayPromise>>,
    selection: Option<u64>,
    duration: Option<f64>,
    boundary_queued: Option<MediaPlaybackBoundary>,
    played: MediaPlayedRanges,
    clock: Option<MediaPlaybackClock>,
    video: Option<video::VideoElementResource>,
}

impl PlaybackElementState {
    fn new(document: DomHandle, epoch: u64) -> Self {
        Self {
            document,
            target: None,
            task_document: None,
            epoch,
            can_autoplay: true,
            pending: Vec::new(),
            outstanding: Vec::new(),
            selection: None,
            duration: None,
            boundary_queued: None,
            played: MediaPlayedRanges::default(),
            clock: None,
            video: None,
        }
    }
}

#[derive(Default)]
pub(super) struct MediaPlaybackState {
    elements: HashMap<DomHandle, PlaybackElementState>,
    stable_work: HashMap<u64, StableWork>,
    next_stable_id: u64,
    video_decode_budget: super::decode_budget::SharedDecodeBudget,
}

#[derive(Clone, Copy)]
enum StableOperation {
    Select { explicit: bool },
    PauseIfRemoved,
}

struct StableWork {
    media: DomHandle,
    document: DomHandle,
    task_document: DomHandle,
    target: WindowTaskTarget,
    epoch: u64,
    operation: StableOperation,
}

enum PlaybackOperation {
    ClockTick(RendererPageMediaElementEventTaskId),
    Boundary(MediaPlaybackBoundary),
    Emptied,
    VolumeChange,
    Play,
    Waiting,
    Playing,
    Resolve,
    Pause,
    Seeked,
    Reject(MediaPlayError),
}

pub(crate) struct MediaPlaybackTask {
    media: DomHandle,
    document: DomHandle,
    epoch: u64,
    operation: PlaybackOperation,
    promises: Vec<Rc<PlayPromise>>,
}

impl JsContextHost {
    pub(crate) fn begin_media_resource(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
        sequence: super::MediaLoadSequenceId,
        bytes: &[u8],
        origin_clean: bool,
        network_id: Option<u64>,
    ) -> Result<bool, String> {
        if self
            .begin_media_video_resource(scope, media, sequence, bytes, origin_clean, network_id)
            .map_err(|error| error.to_string())?
        {
            return Ok(true);
        }
        let duration = moli_media::ogg_vorbis_duration(bytes).map_err(|error| error.to_string())?;
        self.media_playback_state_for(media)
            .ok_or_else(|| "media resource owner is unavailable".to_owned())?
            .duration = duration;
        Ok(false)
    }

    pub(crate) fn media_duration(&self, media: DomHandle) -> Option<f64> {
        let node = self.dom_host().node(media)?;
        if node.as_element()?.media_ready_state() < 1 {
            return None;
        }
        let state = self.media_playback.elements.get(&media)?;
        if Some(state.document) != node.owner_document() {
            return None;
        }
        Some(state.duration.unwrap_or(f64::INFINITY))
    }

    fn media_playback_state_for(&mut self, media: DomHandle) -> Option<&mut PlaybackElementState> {
        let document = self.dom_host().node(media)?.owner_document()?;
        if self
            .media_playback
            .elements
            .get(&media)
            .is_some_and(|state| state.document != document)
        {
            self.stop_media_clock(media);
            self.cancel_media_video_resource(media);
        }
        let entry = self
            .media_playback
            .elements
            .entry(media)
            .or_insert_with(|| PlaybackElementState::new(document, 1));
        if entry.document != document {
            *entry = PlaybackElementState::new(
                document,
                entry.epoch.checked_add(1).expect("media epoch overflow"),
            );
        }
        Some(entry)
    }

    fn playback_is_current(&self, media: DomHandle, document: DomHandle, epoch: u64) -> bool {
        self.media_playback
            .elements
            .get(&media)
            .is_some_and(|state| state.document == document && state.epoch == epoch)
            && self
                .dom_host()
                .node(media)
                .and_then(crate::dom::native::Node::owner_document)
                == Some(document)
    }

    pub(crate) fn reset_media_after_document_move(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) {
        let document = self
            .dom_host()
            .node(media)
            .and_then(crate::dom::native::Node::owner_document);
        let moved = self
            .pending_media_load_sequence(media)
            .is_some_and(|load| Some(load.owner_document_handle()) != document)
            || self
                .media_playback
                .elements
                .get(&media)
                .is_some_and(|state| Some(state.document) != document);
        if moved {
            self.reset_media_playback_for_load(scope, media);
            let _ = self.cancel_pending_media_load_sequence(media);
            // Selection restarts in the new Document with the same playback intent.
            let _ = self.set_media_error_code(media, None);
            let _ = self.set_media_ready_state(media, 0);
            let _ = self.set_media_network_state(media, 0);
            let _ = self.set_media_current_time(media, 0.0);
            let _ = self.set_media_seeking(media, false);
        }
    }

    pub(crate) fn reset_media_playback_for_load(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) {
        self.stop_media_clock(media);
        self.cancel_media_video_resource(media);
        let _ = self.dom_host_mut().advance_media_seek_token(media);
        let promises = self
            .media_playback
            .elements
            .get_mut(&media)
            .map(|state| {
                state
                    .outstanding
                    .drain(..)
                    .filter_map(|value| value.upgrade())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let Some(state) = self.media_playback_state_for(media) else {
            return;
        };
        state.pending.clear();
        state.selection = None;
        state.duration = None;
        state.boundary_queued = None;
        state.played.clear();
        state.video = None;
        state.can_autoplay = true;
        state.epoch = state.epoch.checked_add(1).expect("media epoch overflow");
        for promise in promises {
            // Complete settlements already queued by play or pause before
            // discarding their tasks. Requests still pending receive AbortError.
            let result = promise
                .queued_result
                .get()
                .unwrap_or(Err(MediaPlayError::Abort));
            promise.settle(scope, result.err());
        }
    }

    pub(super) fn retire_media_playback_for_element(&mut self, media: DomHandle) {
        self.stop_media_clock(media);
        self.cancel_media_video_resource(media);
        if let Some(state) = self.media_playback.elements.get_mut(&media) {
            // Keep the epoch monotonic if author code immediately reuses this node.
            *state = PlaybackElementState::new(
                state.document,
                state.epoch.checked_add(1).expect("media epoch overflow"),
            );
        }
        self.media_playback
            .stable_work
            .retain(|_, work| work.media != media);
    }

    pub(crate) fn retire_media_playback_for_document(&mut self, document: DomHandle) {
        let clocks = self
            .media_playback
            .elements
            .iter()
            .filter_map(|(media, state)| {
                (state.document == document || state.task_document == Some(document))
                    .then_some(*media)
            })
            .collect::<Vec<_>>();
        for media in clocks {
            self.stop_media_clock(media);
            self.cancel_media_video_resource(media);
        }
        self.media_playback
            .elements
            .retain(|_, state| state.document != document && state.task_document != Some(document));
        self.media_playback
            .stable_work
            .retain(|_, work| work.document != document && work.task_document != document);
    }

    pub(crate) fn bind_media_playback_global<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        media: DomHandle,
        receiver: v8::Local<'s, v8::Object>,
    ) -> bool {
        self.reset_media_after_document_move(scope, media);
        let target = match self.owner_dispatch_scope_for_node(media) {
            Some(dispatch_scope) => self.current_media_window_task_target(dispatch_scope),
            None => {
                // A windowless Document is not an active browsing context, but its
                // platform objects still have a relevant global for event tasks.
                let Some(context) = receiver.get_creation_context(scope) else {
                    return false;
                };
                let scope = &mut v8::ContextScope::new(scope, context);
                self.current_runtime_window_execution_context_identity(scope)
                    .map(|identity| {
                        WindowTaskTarget::new(identity.dispatch_scope(), identity.owner())
                    })
            }
        };
        let Some(target) = target else {
            return false;
        };
        let Some(resolved) = self.resolve_media_element_task_context(scope, target) else {
            return false;
        };
        let Some(state) = self.media_playback_state_for(media) else {
            return false;
        };
        state.target = Some(target);
        state.task_document = Some(resolved.document_handle);
        true
    }

    pub(super) fn current_media_window_task_target(
        &self,
        dispatch_scope: super::OwnerDispatchScope,
    ) -> Option<WindowTaskTarget> {
        Some(WindowTaskTarget::new(
            dispatch_scope,
            self.current_window_execution_context_owner(dispatch_scope)?,
        ))
    }

    pub(super) fn resolve_media_element_task_context<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: WindowTaskTarget,
    ) -> Option<super::window_document_tasks::ResolvedWindowDocumentTaskContext<'s>> {
        if !self.window_execution_context_owner_is_current(target.owner(), target.dispatch_scope())
        {
            return None;
        }
        let document_target =
            self.current_window_document_task_target_for_dispatch_scope(target.dispatch_scope())?;
        self.resolve_authorized_window_document_task_context(scope, document_target)
    }

    pub(super) fn media_element_task_target(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) -> Option<WindowTaskTarget> {
        if let Some(dispatch_scope) = self.owner_dispatch_scope_for_node(media) {
            return self.current_media_window_task_target(dispatch_scope);
        }
        if let Some(target) = self
            .media_playback
            .elements
            .get(&media)
            .and_then(|state| state.target)
        {
            return Some(target);
        }
        let context = crate::native_bridge::node_relevant_context_for_handle(scope, self, media)?;
        let scope = &mut v8::ContextScope::new(scope, context);
        self.current_runtime_window_execution_context_identity(scope)
            .map(|identity| WindowTaskTarget::new(identity.dispatch_scope(), identity.owner()))
    }

    fn queue_playback(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
        operation: PlaybackOperation,
        promises: Vec<Rc<PlayPromise>>,
    ) {
        let kind = if matches!(operation, PlaybackOperation::VolumeChange) {
            RendererPageMediaElementEventTaskKind::VolumeChange
        } else {
            RendererPageMediaElementEventTaskKind::Playback
        };
        let Some(task) = self.capture_media_playback_task(media, operation, promises) else {
            return;
        };
        let target = self.media_element_task_target(scope, media);
        if let Some(target) = target {
            let _ = self.queue_media_element_event_for_target(
                target,
                kind,
                super::media_element_events::PendingMediaElementEventPayload::Playback(task),
            );
        }
    }

    fn capture_media_playback_task(
        &mut self,
        media: DomHandle,
        operation: PlaybackOperation,
        promises: Vec<Rc<PlayPromise>>,
    ) -> Option<MediaPlaybackTask> {
        let result = match operation {
            PlaybackOperation::Playing | PlaybackOperation::Resolve => Some(Ok(())),
            PlaybackOperation::Pause => Some(Err(MediaPlayError::Abort)),
            PlaybackOperation::Reject(error) => Some(Err(error)),
            _ => None,
        };
        for promise in &promises {
            promise.queued_result.set(result);
        }
        let state = self.media_playback_state_for(media)?;
        Some(MediaPlaybackTask {
            media,
            document: state.document,
            epoch: state.epoch,
            operation,
            promises,
        })
    }

    pub(crate) fn request_media_play<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        host: *mut Self,
        media: DomHandle,
        resolver: v8::Local<'s, v8::PromiseResolver>,
    ) {
        let request = Rc::new(PlayPromise {
            resolver: v8::Global::new(scope, resolver),
            realm: v8::Global::new(scope, scope.get_current_context()),
            settled: Cell::new(false),
            queued_result: Cell::new(None),
        });
        let (document, epoch) = {
            let runtime = unsafe { &mut *host };
            if runtime
                .media_playback
                .elements
                .get(&media)
                .and_then(|state| state.target)
                .is_none()
            {
                request.settle(scope, Some(MediaPlayError::NotAllowed));
                return;
            }
            if runtime
                .dom_host()
                .node(media)
                .and_then(crate::dom::native::Node::as_element)
                .is_some_and(|element| element.media_error_code() == Some(4))
            {
                request.settle(scope, Some(MediaPlayError::NotSupported));
                return;
            }
            let Some(state) = runtime.media_playback_state_for(media) else {
                return;
            };
            state.outstanding.retain(|value| value.strong_count() > 0);
            state.outstanding.push(Rc::downgrade(&request));
            state.pending.push(request.clone());
            state.can_autoplay = false;
            let identity = (state.document, state.epoch);
            element::queue_media_load_for_explicit_request(scope, runtime, media);
            identity
        };
        Self::rewind_media_for_play(scope, host, media);
        if request.settled.get() || !unsafe { &*host }.playback_is_current(media, document, epoch) {
            return;
        }
        let runtime = unsafe { &mut *host };
        let changed = runtime.set_media_paused(media, false);
        let ready = runtime
            .dom_host()
            .node(media)
            .and_then(crate::dom::native::Node::as_element)
            .map_or(0, |element| element.media_ready_state());
        if changed {
            runtime.queue_playback(scope, media, PlaybackOperation::Play, Vec::new());
            if ready < 3 {
                runtime.queue_playback(scope, media, PlaybackOperation::Waiting, Vec::new());
            } else {
                runtime.notify_media_playing(scope, media);
            }
        } else if ready >= 3 {
            let promises = runtime.take_media_play_promises(media);
            runtime.queue_playback(scope, media, PlaybackOperation::Resolve, promises);
        }
    }

    fn take_media_play_promises(&mut self, media: DomHandle) -> Vec<Rc<PlayPromise>> {
        self.media_playback
            .elements
            .get_mut(&media)
            .map(|state| std::mem::take(&mut state.pending))
            .unwrap_or_default()
    }

    pub(crate) fn pause_media_playback(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) {
        if let Some(state) = self.media_playback_state_for(media) {
            state.can_autoplay = false;
        }
        if self.set_media_paused(media, true) {
            let promises = self.take_media_play_promises(media);
            self.queue_playback(scope, media, PlaybackOperation::Pause, promises);
        }
    }

    pub(crate) fn queue_media_emptied(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) {
        self.queue_playback(scope, media, PlaybackOperation::Emptied, Vec::new());
    }

    pub(crate) fn queue_media_seeked(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) {
        self.queue_playback(scope, media, PlaybackOperation::Seeked, Vec::new());
    }

    pub(crate) fn queue_media_volume_change<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        media: DomHandle,
        receiver: v8::Local<'s, v8::Object>,
    ) {
        if self.bind_media_playback_global(scope, media, receiver) {
            self.queue_playback(scope, media, PlaybackOperation::VolumeChange, Vec::new());
        }
    }

    pub(crate) fn notify_media_playing(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) {
        if self.media_ended(media)
            || self
                .dom_host()
                .node(media)
                .and_then(crate::dom::native::Node::as_element)
                .is_none_or(|element| element.media_paused())
        {
            return;
        }
        self.start_media_clock(media);
        let promises = self.take_media_play_promises(media);
        self.queue_playback(scope, media, PlaybackOperation::Playing, promises);
    }

    pub(crate) fn capture_media_play_failure(
        &mut self,
        media: DomHandle,
    ) -> Option<MediaPlaybackTask> {
        let promises = self.take_media_play_promises(media);
        self.capture_media_playback_task(
            media,
            PlaybackOperation::Reject(MediaPlayError::NotSupported),
            promises,
        )
    }

    pub(crate) fn start_media_autoplay(
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut Self,
        media: DomHandle,
    ) {
        let identity = {
            let runtime = unsafe { &mut *host };
            let Some(state) = runtime.media_playback_state_for(media) else {
                return;
            };
            if !state.can_autoplay {
                return;
            }
            (state.document, state.epoch)
        };
        let eligible = unsafe { &*host }
            .dom_host()
            .node(media)
            .and_then(crate::dom::native::Node::as_element)
            .is_some_and(|element| {
                element.media_paused() && element.attribute("autoplay").is_some()
            });
        if !eligible {
            return;
        }
        Self::rewind_media_for_play(scope, host, media);
        let runtime = unsafe { &mut *host };
        if runtime.playback_is_current(media, identity.0, identity.1)
            && runtime.set_media_paused(media, false)
        {
            runtime.queue_playback(scope, media, PlaybackOperation::Play, Vec::new());
            runtime.notify_media_playing(scope, media);
        }
    }

    pub(crate) fn schedule_media_selection(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
        explicit: bool,
    ) {
        if let Some(token) = self
            .media_playback_state_for(media)
            .and_then(|state| state.selection)
            && let Some(work) = self.media_playback.stable_work.get_mut(&token)
        {
            if let StableOperation::Select { explicit: previous } = &mut work.operation {
                *previous |= explicit;
            }
            return;
        }
        let token =
            self.queue_media_stable_work(scope, media, StableOperation::Select { explicit });
        if let Some(state) = self.media_playback.elements.get_mut(&media) {
            state.selection = token;
        }
    }

    pub(crate) fn schedule_media_pause_if_removed(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) {
        let _ = self.queue_media_stable_work(scope, media, StableOperation::PauseIfRemoved);
    }

    fn queue_media_stable_work(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
        operation: StableOperation,
    ) -> Option<u64> {
        let target = self.media_element_task_target(scope, media)?;
        let resolved = self.resolve_media_element_task_context(scope, target)?;
        let task_document = resolved.document_handle;
        let state = self.media_playback_state_for(media)?;
        state.target = Some(target);
        state.task_document = Some(task_document);
        let (document, epoch) = (state.document, state.epoch);
        self.media_playback.next_stable_id = self
            .media_playback
            .next_stable_id
            .checked_add(1)
            .expect("media stable work id overflow");
        let token = self.media_playback.next_stable_id;
        let scope = &mut v8::ContextScope::new(scope, resolved.context);
        let data = v8::BigInt::new_from_u64(scope, token);
        let callback = v8::Function::builder(media_stable_work_callback)
            .data(data.into())
            .build(scope)?;
        self.media_playback.stable_work.insert(
            token,
            StableWork {
                media,
                document,
                task_document,
                target,
                epoch,
                operation,
            },
        );
        scope.enqueue_microtask(callback);
        Some(token)
    }

    fn apply_media_stable_work(&mut self, scope: &mut v8::PinScope<'_, '_>, token: u64) {
        let Some(work) = self.media_playback.stable_work.remove(&token) else {
            return;
        };
        if !self.playback_is_current(work.media, work.document, work.epoch)
            || self.current_media_window_task_target(work.target.dispatch_scope())
                != Some(work.target)
        {
            return;
        }
        match work.operation {
            StableOperation::Select { explicit } => {
                let state = self
                    .media_playback
                    .elements
                    .get_mut(&work.media)
                    .expect("current media state");
                if state.selection != Some(token) {
                    return;
                }
                state.selection = None;
                element::apply_media_resource_selection(scope, self, work.media, explicit);
            }
            StableOperation::PauseIfRemoved => {
                if self
                    .dom_host()
                    .node(work.media)
                    .is_some_and(|node| !node.is_connected())
                {
                    self.pause_media_playback(scope, work.media);
                }
            }
        }
    }
}

impl MediaPlaybackTask {
    pub(super) fn is_current(&self, host: &JsContextHost) -> bool {
        host.playback_is_current(self.media, self.document, self.epoch)
            && match self.operation {
                PlaybackOperation::ClockTick(task_id) => host
                    .media_playback
                    .elements
                    .get(&self.media)
                    .and_then(|state| state.clock.as_ref())
                    .is_some_and(|clock| clock.is_current_tick(task_id)),
                _ => true,
            }
    }

    pub(crate) fn dispatch(
        self,
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut JsContextHost,
    ) -> bool {
        if !unsafe { &*host }.playback_is_current(self.media, self.document, self.epoch) {
            return false;
        }
        let error = match self.operation {
            PlaybackOperation::ClockTick(task_id) => {
                return clock::dispatch_media_clock_tick(scope, host, self.media, task_id);
            }
            PlaybackOperation::Boundary(boundary) => {
                return boundary::dispatch_media_boundary(
                    scope,
                    host,
                    self.media,
                    self.document,
                    self.epoch,
                    boundary,
                );
            }
            PlaybackOperation::Emptied => {
                fire(scope, host, self.media, "emptied");
                None
            }
            PlaybackOperation::VolumeChange => {
                fire(scope, host, self.media, "volumechange");
                None
            }
            PlaybackOperation::Play => {
                fire(scope, host, self.media, "play");
                None
            }
            PlaybackOperation::Waiting => {
                fire(scope, host, self.media, "waiting");
                None
            }
            PlaybackOperation::Playing => {
                fire(scope, host, self.media, "playing");
                None
            }
            PlaybackOperation::Resolve => None,
            PlaybackOperation::Seeked => {
                fire(scope, host, self.media, "seeked");
                None
            }
            PlaybackOperation::Pause => {
                fire(scope, host, self.media, "timeupdate");
                if !unsafe { &*host }.playback_is_current(self.media, self.document, self.epoch) {
                    return true;
                }
                fire(scope, host, self.media, "pause");
                Some(MediaPlayError::Abort)
            }
            PlaybackOperation::Reject(error) => Some(error),
        };
        if unsafe { &*host }.playback_is_current(self.media, self.document, self.epoch) {
            for promise in self.promises {
                promise.settle(scope, error);
            }
        }
        true
    }
}

fn fire(scope: &mut v8::PinScope<'_, '_>, host: *mut JsContextHost, media: DomHandle, name: &str) {
    if let Some(event) = element::construct_simple_event(scope, name, false, false, false) {
        let _ = element::dispatch_public_event(scope, host, media, event);
    }
}

fn media_stable_work_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(token) = v8::Local::<v8::BigInt>::try_from(args.data()) else {
        return;
    };
    let (token, lossless) = token.u64_value();
    if !lossless {
        return;
    }
    if let Some(host) = crate::util::context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host }.apply_media_stable_work(scope, token);
    }
}
