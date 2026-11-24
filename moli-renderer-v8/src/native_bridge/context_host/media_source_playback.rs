use std::sync::Arc;

use anyhow::{Result, anyhow, ensure};
use bytes::Bytes;
use moli_storage_key::MoliStorageKey;
use moli_video::{AudioPlayback, FragmentAppend, VideoWorker};
use moli_web_mime::MediaSourceTracks;
use parking_lot::Mutex;

use super::media_element_events::PendingMediaElementEventPayload;
use super::video_playback::{FragmentPlayback, MAX_VIDEO_BYTES};
use super::{MediaLoadSequenceId, OwnerDispatchScope, PageScriptHost};
use crate::context_bootstrap::media_source_playback as bindings;
use crate::document_runtime::DomHandle;
use crate::page_task_queue::RendererPageMediaElementEventTaskKind;

const EVENT_KIND: RendererPageMediaElementEventTaskKind =
    RendererPageMediaElementEventTaskKind::MediaSourceEvent;

pub(super) struct MediaSourceUrl {
    source: v8::Global<v8::Object>,
    creator_document: DomHandle,
    storage_key: MoliStorageKey,
}

type SourceBufferResult = Arc<Mutex<Option<Result<FragmentAppend>>>>;

pub(super) enum MediaSourceEvent {
    Event {
        target: v8::Global<v8::Object>,
        current_load: Option<(DomHandle, MediaLoadSequenceId)>,
        name: &'static str,
    },
    Append {
        media: DomHandle,
        sequence: MediaLoadSequenceId,
        buffer: v8::Global<v8::Object>,
        tracks: MediaSourceTracks,
        result: SourceBufferResult,
    },
}

impl PageScriptHost {
    pub(crate) fn create_media_source_url<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        source: v8::Local<'s, v8::Object>,
        storage_key: MoliStorageKey,
    ) -> Option<String> {
        let identity = self.current_runtime_window_execution_context_identity(scope)?;
        let creator_document = match identity.dispatch_scope() {
            OwnerDispatchScope::Top | OwnerDispatchScope::LightweightPopup(_) => {
                self.document_handle()
            }
            OwnerDispatchScope::Child(frame) => {
                self.child_browsing_context_document_handle(frame)?
            }
        };
        let mut token = [0; 16];
        moli_crypto::fill_secure_random(&mut token).ok()?;
        let url = format!(
            "blob:{}/{}",
            storage_key.origin(),
            uuid::Builder::from_random_bytes(token).into_uuid()
        );
        self.media_source_urls.insert(
            url.clone(),
            MediaSourceUrl {
                source: v8::Global::new(scope, source),
                creator_document,
                storage_key,
            },
        );
        Some(url)
    }

    pub(crate) fn revoke_media_source_url(&mut self, url: &str, storage_key: &MoliStorageKey) {
        if self
            .media_source_urls
            .get(url)
            .is_some_and(|entry| &entry.storage_key == storage_key)
        {
            self.media_source_urls.remove(url);
        }
    }

    pub(super) fn retire_media_source_urls(&mut self, document: DomHandle) {
        self.media_source_urls
            .retain(|_, url| url.creator_document != document);
    }

    pub(crate) fn owns_media_source<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        media: DomHandle,
        source: v8::Local<'s, v8::Object>,
    ) -> bool {
        self.current_video(media)
            .and_then(|video| video.fragment.as_ref())
            .is_some_and(|fragment| v8::Local::new(scope, &fragment.source) == source)
    }

    pub(crate) fn try_attach_media_source(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
        sequence: MediaLoadSequenceId,
        url: &url::Url,
    ) -> Result<bool> {
        if url.scheme() != "blob" {
            return Ok(false);
        }
        let mut key = url.clone();
        key.set_fragment(None);
        let Some(entry) = self.media_source_urls.get(key.as_str()) else {
            return Ok(false);
        };
        let source = entry.source.clone();
        let creator_storage_key = entry.storage_key.clone();
        let owner = self
            .owner_dispatch_scope_for_node(media)
            .ok_or_else(|| anyhow!("media element has no active owner"))?;
        let storage = match owner {
            OwnerDispatchScope::Top => self.top_document_storage_context(),
            OwnerDispatchScope::Child(frame) => self
                .storage_context_for_child_browsing_context(frame)
                .ok_or_else(|| anyhow!("media element has no active storage context"))?,
            OwnerDispatchScope::LightweightPopup(id) => self
                .storage_context_for_lightweight_popup(id)
                .ok_or_else(|| anyhow!("media element has no active storage context"))?,
        };
        ensure!(
            *storage.storage_key() == creator_storage_key,
            "MediaSource object URL is unavailable"
        );
        ensure!(
            !self.video_decoder_limit_reached(),
            "too many active video decoders"
        );
        let local = v8::Local::new(scope, &source);
        ensure!(
            bindings::attached_media(scope, local)
                .is_none_or(|media| !self.owns_media_source(scope, media, local)),
            "MediaSource is already attached"
        );
        let worker = VideoWorker::open_fragmented()?;
        let video = self
            .video_playbacks
            .get_mut(&media)
            .filter(|video| video.sequence == sequence)
            .ok_or_else(|| anyhow!("MediaSource attachment requires an active HTMLVideoElement"))?;
        video.worker = Some(worker);
        video.fragment = Some(FragmentPlayback {
            source: source.clone(),
            ranges: Vec::new(),
            video_ranges: Vec::new(),
            audio_ranges: Vec::new(),
            has_video: false,
            audio: None,
            duration: None,
            waiting: true,
        });
        bindings::attach(scope, local, media);
        ensure!(
            self.queue_source_event(scope, media, Some(sequence), source, "sourceopen"),
            "retired MediaSource attachment"
        );
        Ok(true)
    }

    pub(super) fn detach_media_source(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
        source: v8::Global<v8::Object>,
    ) {
        let local = v8::Local::new(scope, &source);
        bindings::detach(scope, local);
        self.queue_source_event(scope, media, None, source, "sourceclose");
    }

    fn queue_source_event(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
        sequence: Option<MediaLoadSequenceId>,
        target: v8::Global<v8::Object>,
        name: &'static str,
    ) -> bool {
        let event = MediaSourceEvent::Event {
            target,
            current_load: sequence.map(|sequence| (media, sequence)),
            name,
        };
        let Some((target, id)) = self.prepare_media_element_event(
            scope,
            media,
            EVENT_KIND,
            PendingMediaElementEventPayload::MediaSource(event),
        ) else {
            return false;
        };
        if self
            .page_media_element_event_sender()
            .send(target, id, EVENT_KIND)
            .is_ok()
        {
            true
        } else {
            self.media_element_events.remove(id);
            false
        }
    }

    pub(crate) fn add_media_source_tracks(
        &mut self,
        media: DomHandle,
        tracks: MediaSourceTracks,
    ) -> bool {
        let Some(video) = self.video_playbacks.get_mut(&media) else {
            return false;
        };
        let (Some(fragment), Some(worker)) = (&mut video.fragment, &video.worker) else {
            return false;
        };
        if (tracks.has_video() && fragment.has_video)
            || (tracks.has_audio() && fragment.audio.is_some())
        {
            return false;
        }
        if tracks.has_audio() {
            let audio = AudioPlayback::default();
            if worker.enable_audio(audio.clone()).is_err() {
                return false;
            }
            fragment.audio = Some(audio);
        }
        fragment.has_video |= tracks.has_video();
        fragment.update_ranges(false);
        true
    }

    pub(crate) fn media_source_buffered(
        &self,
        media: DomHandle,
        tracks: MediaSourceTracks,
    ) -> Vec<(f64, f64)> {
        self.current_video(media)
            .and_then(|video| video.fragment.as_ref())
            .map(|fragment| match tracks {
                MediaSourceTracks::Video => fragment.video_ranges.clone(),
                MediaSourceTracks::Audio => fragment.audio_ranges.clone(),
                MediaSourceTracks::VideoAndAudio => fragment.ranges.clone(),
            })
            .unwrap_or_default()
    }

    pub(crate) fn append_media_source_buffer<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        media: DomHandle,
        buffer: v8::Local<'s, v8::Object>,
        tracks: MediaSourceTracks,
        bytes: Bytes,
        offset: f64,
    ) -> bool {
        let Some(video) = self.current_video(media) else {
            return false;
        };
        if video.fragment.is_none() || video.worker.is_none() || bytes.len() > MAX_VIDEO_BYTES {
            return false;
        }
        let sequence = video.sequence;
        let result = SourceBufferResult::default();
        let event = MediaSourceEvent::Append {
            media,
            sequence,
            buffer: v8::Global::new(scope, buffer),
            tracks,
            result: result.clone(),
        };
        let Some((target, id)) = self.prepare_media_element_event(
            scope,
            media,
            EVENT_KIND,
            PendingMediaElementEventPayload::MediaSource(event),
        ) else {
            return false;
        };
        self.queue_source_event(
            scope,
            media,
            Some(sequence),
            v8::Global::new(scope, buffer),
            "updatestart",
        );
        let sender = self.page_media_element_event_sender().clone();
        let complete = move |append| {
            *result.lock() = Some(append);
            let _ = sender.send(target, id, EVENT_KIND);
        };
        let worker = self
            .video_playbacks
            .get(&media)
            .and_then(|video| video.worker.as_ref())
            .expect("active decoder");
        if worker.append(tracks, bytes, offset, complete).is_err() {
            self.media_element_events.remove(id);
            return false;
        }
        true
    }

    pub(crate) fn end_media_source(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) -> Option<f64> {
        let video = self.video_playbacks.get_mut(&media)?;
        video.worker.as_ref()?.end().ok()?;
        let fragment = video.fragment.as_mut()?;
        fragment.waiting = false;
        let duration = fragment.end_time();
        fragment.update_ranges(true);
        fragment.duration = Some(duration);
        let source = fragment.source.clone();
        let sequence = video.sequence;
        self.queue_source_event(scope, media, Some(sequence), source, "sourceended");
        self.update_video_clock(scope, media);
        Some(duration)
    }

    pub(crate) fn set_media_source_duration(&mut self, media: DomHandle, duration: f64) {
        if let Some(fragment) = self
            .video_playbacks
            .get_mut(&media)
            .and_then(|video| video.fragment.as_mut())
        {
            fragment.duration = Some(duration);
        }
    }
}

pub(super) fn apply_media_source_event(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    event: MediaSourceEvent,
) -> bool {
    let host = unsafe { &mut *host_ptr };
    match event {
        MediaSourceEvent::Event {
            target,
            current_load,
            name,
        } => {
            if current_load.is_some_and(|(media, sequence)| {
                !host
                    .current_video(media)
                    .is_some_and(|video| video.sequence == sequence)
            }) {
                return false;
            }
            let target = v8::Local::new(scope, &target);
            bindings::dispatch_event(scope, target, name);
        }
        MediaSourceEvent::Append {
            media,
            sequence,
            buffer,
            tracks,
            result,
        } => {
            let Some(video) = host
                .video_playbacks
                .get_mut(&media)
                .filter(|video| video.sequence == sequence)
            else {
                return false;
            };
            let Some(fragment) = video.fragment.as_mut() else {
                return false;
            };
            let Some(result) = result.lock().take() else {
                return false;
            };
            let success = match result {
                Ok(info) => {
                    if tracks.has_video() {
                        fragment.video_ranges = info.ranges;
                    }
                    if let Some(audio_ranges) = info.audio_ranges {
                        fragment.audio_ranges = audio_ranges;
                    }
                    if fragment.duration.is_none()
                        && let Some(metadata) = info.metadata
                    {
                        fragment.duration = Some(metadata.duration.unwrap_or(f64::INFINITY));
                    }
                    fragment.update_ranges(false);
                    fragment.waiting = fragment.ranges.is_empty();
                    true
                }
                Err(error) => {
                    tracing::debug!(?error, "MediaSource fragment append failed");
                    false
                }
            };
            let buffer = v8::Local::new(scope, &buffer);
            let source = v8::Local::new(scope, &fragment.source);
            if let Some(duration) = fragment.duration {
                bindings::publish_duration(scope, source, duration);
            }
            bindings::set_updating(scope, buffer, false);
            if success {
                host.update_video_clock(scope, media);
                bindings::dispatch_event(scope, buffer, "update");
            } else {
                bindings::dispatch_event(scope, buffer, "error");
            }
            bindings::dispatch_event(scope, buffer, "updateend");
        }
    }
    true
}
