//! Pending play requests and playback tasks share the media element's native
//! identity. Promise realms belong to media objects; task ownership belongs to the
//! element's Window and exact Document.

use std::{
    cell::Cell,
    collections::HashMap,
    rc::{Rc, Weak},
};

use super::{JsContextHost, WindowDocumentTaskTarget};
use crate::{
    document_runtime::DomHandle, native_bridge::element,
    page_task_queue::RendererPageMediaElementEventTaskKind,
};

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
    target: Option<WindowDocumentTaskTarget>,
    task_document: Option<DomHandle>,
    epoch: u64,
    can_autoplay: bool,
    pending: Vec<Rc<PlayPromise>>,
    outstanding: Vec<Weak<PlayPromise>>,
    selection: Option<u64>,
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
        }
    }
}

#[derive(Default)]
pub(super) struct MediaPlaybackState {
    elements: HashMap<DomHandle, PlaybackElementState>,
    stable_work: HashMap<u64, StableWork>,
    next_stable_id: u64,
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
    target: WindowDocumentTaskTarget,
    epoch: u64,
    operation: StableOperation,
}

enum PlaybackOperation {
    Emptied,
    VolumeChange,
    Play,
    Waiting,
    Playing,
    Resolve,
    Pause,
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
    fn media_playback_state_for(&mut self, media: DomHandle) -> Option<&mut PlaybackElementState> {
        let document = self.dom_host().node(media)?.owner_document()?;
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
            let _ = self.set_media_paused(media, true);
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

    pub(crate) fn retire_media_playback_for_document(&mut self, document: DomHandle) {
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
            Some(dispatch_scope) => {
                self.current_window_document_task_target_for_dispatch_scope(dispatch_scope)
            }
            None => {
                // A windowless Document is not an active browsing context, but its
                // platform objects still have a relevant global for event tasks.
                let Some(context) = receiver.get_creation_context(scope) else {
                    return false;
                };
                let scope = &mut v8::ContextScope::new(scope, context);
                self.current_renderer_window_document_source(scope)
                    .map(|source| source.0)
            }
        };
        let Some(target) = target else {
            return false;
        };
        let Some(resolved) = self.resolve_authorized_window_document_task_context(scope, target)
        else {
            return false;
        };
        let Some(state) = self.media_playback_state_for(media) else {
            return false;
        };
        state.target = Some(target);
        state.task_document = Some(resolved.document_handle);
        true
    }

    pub(super) fn media_element_task_target(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) -> Option<WindowDocumentTaskTarget> {
        if let Some(dispatch_scope) = self.owner_dispatch_scope_for_node(media) {
            return self.current_window_document_task_target_for_dispatch_scope(dispatch_scope);
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
        self.current_renderer_window_document_source(scope)
            .map(|source| source.0)
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
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        media: DomHandle,
        resolver: v8::Local<'s, v8::PromiseResolver>,
    ) {
        let request = Rc::new(PlayPromise {
            resolver: v8::Global::new(scope, resolver),
            realm: v8::Global::new(scope, scope.get_current_context()),
            settled: Cell::new(false),
            queued_result: Cell::new(None),
        });
        if self
            .media_playback
            .elements
            .get(&media)
            .and_then(|state| state.target)
            .is_none()
        {
            request.settle(scope, Some(MediaPlayError::NotAllowed));
            return;
        }
        if self
            .dom_host()
            .node(media)
            .and_then(crate::dom::native::Node::as_element)
            .is_some_and(|element| element.media_error_code() == Some(4))
        {
            request.settle(scope, Some(MediaPlayError::NotSupported));
            return;
        }
        let Some(state) = self.media_playback_state_for(media) else {
            return;
        };
        state.outstanding.retain(|value| value.strong_count() > 0);
        state.outstanding.push(Rc::downgrade(&request));
        state.pending.push(request);
        state.can_autoplay = false;
        element::queue_media_load_for_explicit_request(scope, self, media);
        let changed = self.set_media_paused(media, false);
        let ready = self
            .dom_host()
            .node(media)
            .and_then(crate::dom::native::Node::as_element)
            .map_or(0, |element| element.media_ready_state());
        if changed {
            self.queue_playback(scope, media, PlaybackOperation::Play, Vec::new());
        }
        if ready >= 3 {
            if changed {
                self.notify_media_playing(scope, media);
            } else {
                let promises = self.take_media_play_promises(media);
                self.queue_playback(scope, media, PlaybackOperation::Resolve, promises);
            }
        } else if changed {
            self.queue_playback(scope, media, PlaybackOperation::Waiting, Vec::new());
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
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) {
        let can_autoplay = self
            .media_playback
            .elements
            .get(&media)
            .is_none_or(|state| state.can_autoplay);
        let eligible = can_autoplay
            && self
                .dom_host()
                .node(media)
                .and_then(crate::dom::native::Node::as_element)
                .is_some_and(|element| {
                    element.media_paused() && element.attribute("autoplay").is_some()
                });
        if eligible && self.set_media_paused(media, false) {
            self.queue_playback(scope, media, PlaybackOperation::Play, Vec::new());
            self.notify_media_playing(scope, media);
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
        let resolved = self.resolve_authorized_window_document_task_context(scope, target)?;
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
            || self.current_window_document_task_target_for_dispatch_scope(
                work.target.dispatch_scope(),
            ) != Some(work.target)
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
