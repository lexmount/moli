use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use moli_video::{
    AudioPlayback, DecodeRequest, DecodeResult, VideoFrame, VideoMetadata, VideoWorker,
};
use parking_lot::Mutex;

use super::media_element_events::PendingMediaElementEventPayload;
use super::{MediaLoadSequenceId, PageScriptHost, WindowDocumentTaskTarget};
use crate::document_runtime::DomHandle;
use crate::dom::native::Node;
use crate::native_bridge::element::{
    dispatch_text_control_event, queue_media_load_network_terminal_followup,
    refresh_media_active_text_track_cues,
};
use crate::page_task_queue::{
    RendererPageMediaElementEventSender, RendererPageMediaElementEventTaskId,
    RendererPageMediaElementEventTaskKind,
};

pub(super) const MAX_VIDEO_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy)]
struct PlaybackClock {
    anchor: Instant,
    position: f64,
    rate: f64,
}

pub(super) struct VideoPlayback {
    document: DomHandle,
    pub(super) sequence: MediaLoadSequenceId,
    pub(super) worker: Option<VideoWorker>,
    pub(super) fragment: Option<FragmentPlayback>,
    network_request: Option<u64>,
    metadata: Option<VideoMetadata>,
    shown: Option<VideoFrame>,
    queued: Option<VideoFrame>,
    clock: Option<PlaybackClock>,
    position: f64,
    at_end: bool,
    drained: bool,
    seek: Option<(f64, u64)>,
    task: Option<RendererPageMediaElementEventTaskId>,
    timer: Option<tokio::task::AbortHandle>,
    play_promises: Vec<v8::Global<v8::PromiseResolver>>,
    last_timeupdate: Option<Instant>,
}

pub(super) struct FragmentPlayback {
    pub(super) source: v8::Global<v8::Object>,
    pub(super) ranges: Vec<(f64, f64)>,
    pub(super) video_ranges: Vec<(f64, f64)>,
    pub(super) audio_ranges: Vec<(f64, f64)>,
    pub(super) has_video: bool,
    pub(super) audio: Option<AudioPlayback>,
    pub(super) duration: Option<f64>,
    pub(super) waiting: bool,
}

impl FragmentPlayback {
    pub(super) fn update_ranges(&mut self, end_of_stream: bool) {
        self.ranges.clear();
        if self.audio.is_none() {
            self.ranges.extend_from_slice(&self.video_ranges);
        } else if !self.has_video {
            self.ranges.extend_from_slice(&self.audio_ranges);
        } else {
            let end = end_of_stream.then(|| self.end_time());
            let mut video = self.video_ranges.iter().peekable();
            let mut audio = self.audio_ranges.iter().peekable();
            let mut left = video.next();
            let mut right = audio.next();
            while let (Some(&(vs, ve)), Some(&(as_, ae))) = (left, right) {
                let ve = if video.peek().is_none() {
                    end.unwrap_or(ve)
                } else {
                    ve
                };
                let ae = if audio.peek().is_none() {
                    end.unwrap_or(ae)
                } else {
                    ae
                };
                let start = vs.max(as_);
                let end = ve.min(ae);
                if start < end {
                    self.ranges.push((start, end));
                }
                if ve <= ae {
                    left = video.next();
                } else {
                    right = audio.next();
                }
            }
        }
    }

    pub(super) fn end_time(&self) -> f64 {
        self.video_ranges
            .last()
            .map_or(0.0, |(_, end)| *end)
            .max(self.audio_ranges.last().map_or(0.0, |(_, end)| *end))
    }
}

impl VideoPlayback {
    fn new(document: DomHandle, sequence: MediaLoadSequenceId) -> Self {
        Self {
            document,
            sequence,
            worker: None,
            fragment: None,
            network_request: None,
            metadata: None,
            shown: None,
            queued: None,
            clock: None,
            position: 0.0,
            at_end: false,
            drained: false,
            seek: None,
            task: None,
            timer: None,
            play_promises: Vec::new(),
            last_timeupdate: None,
        }
    }

    fn duration(&self) -> Option<f64> {
        if let Some(fragment) = &self.fragment {
            return fragment.duration;
        }
        self.metadata.and_then(|metadata| metadata.duration)
    }

    fn current_time(&self) -> f64 {
        let position = self.clock.map_or(self.position, |clock| {
            clock.position + clock.anchor.elapsed().as_secs_f64() * clock.rate
        });
        let mut end = self.duration().unwrap_or(f64::INFINITY);
        if let Some((_, buffered_end)) = self
            .fragment
            .as_ref()
            .and_then(|fragment| fragment.ranges.last())
        {
            end = end.min((*buffered_end).max(self.position));
        }
        position.min(end)
    }

    fn update_clock(&mut self, playing: bool, rate: f64) {
        self.position = self.current_time();
        self.clock = (playing && rate > 0.0).then(|| PlaybackClock {
            anchor: Instant::now(),
            position: self.position,
            rate,
        });
        if let Some(audio) = self
            .fragment
            .as_ref()
            .and_then(|fragment| fragment.audio.as_ref())
        {
            audio.set_clock(
                self.clock
                    .map(|clock| (clock.anchor, clock.position, clock.rate)),
            );
        }
    }
}

impl Drop for VideoPlayback {
    fn drop(&mut self) {
        if let Some(audio) = self
            .fragment
            .as_ref()
            .and_then(|fragment| fragment.audio.as_ref())
        {
            audio.set_clock(None);
        }
        if let Some(timer) = self.timer.take() {
            timer.abort();
        }
    }
}

type VideoUpdateSlot = Arc<Mutex<Option<VideoUpdate>>>;
const UPDATE_KIND: RendererPageMediaElementEventTaskKind =
    RendererPageMediaElementEventTaskKind::VideoUpdate;

pub(super) struct PendingVideoUpdate {
    media: DomHandle,
    sequence: MediaLoadSequenceId,
    slot: VideoUpdateSlot,
}

#[derive(Clone)]
struct VideoUpdateTask {
    target: WindowDocumentTaskTarget,
    id: RendererPageMediaElementEventTaskId,
    sender: RendererPageMediaElementEventSender,
    slot: VideoUpdateSlot,
}

enum VideoUpdate {
    Decode {
        result: anyhow::Result<DecodeResult>,
        seek: Option<u64>,
    },
    Present,
}

impl VideoUpdateTask {
    fn complete(self, update: VideoUpdate) {
        *self.slot.lock() = Some(update);
        let _ = self.sender.send(self.target, self.id, UPDATE_KIND);
    }
}

impl PageScriptHost {
    pub(crate) fn register_video_load(&mut self, media: DomHandle, sequence: MediaLoadSequenceId) {
        let Some(document) = self
            .dom_host()
            .node(media)
            .filter(|node| {
                node.as_element()
                    .is_some_and(|el| el.is_html_element("video"))
            })
            .and_then(Node::owner_document)
        else {
            return;
        };
        self.video_playbacks
            .insert(media, VideoPlayback::new(document, sequence));
    }

    pub(crate) fn reset_video_playback(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) {
        if let Some(mut video) = self.remove_video_playback(media) {
            video.update_clock(false, 0.0);
            if let Some(fragment) = video.fragment.take() {
                self.detach_media_source(scope, media, fragment.source);
            }
            settle_play_promises(
                scope,
                std::mem::take(&mut video.play_promises),
                Some("AbortError"),
            );
            self.set_media_paused(media, true);
            self.set_media_seeking(media, false);
            self.set_media_current_time(media, 0.0);
            if video.shown.is_some() {
                self.note_video_frame(true);
            }
        }
    }

    pub(crate) fn reset_adopted_video(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) -> bool {
        if self.video_playbacks.contains_key(&media) && !self.manages_video(media) {
            self.reset_video_playback(scope, media);
            return true;
        }
        false
    }

    pub(crate) fn discard_video_playbacks_for_document(&mut self, document: DomHandle) {
        let media: Vec<_> = self
            .video_playbacks
            .iter()
            .filter_map(|(&media, video)| (video.document == document).then_some(media))
            .collect();
        for media in media {
            self.remove_video_playback(media);
        }
        self.retire_media_source_urls(document);
    }

    pub(super) fn discard_video_playbacks_for_teardown(&mut self) {
        // Retained document wrappers can keep the host alive past page teardown.
        self.video_playbacks.clear();
        self.media_source_urls.clear();
        self.media_element_events.retain(|pending| {
            !matches!(
                pending.payload(),
                PendingMediaElementEventPayload::VideoUpdate(_)
                    | PendingMediaElementEventPayload::MediaSource(_)
            )
        });
    }

    fn remove_video_playback(&mut self, media: DomHandle) -> Option<VideoPlayback> {
        let mut video = self.video_playbacks.remove(&media)?;
        if let Some(task) = video.task.take() {
            self.media_element_events.remove(task);
        }
        Some(video)
    }

    pub(super) fn discard_video_update(&mut self, update: PendingVideoUpdate) {
        if self
            .video_playbacks
            .get(&update.media)
            .is_some_and(|video| video.sequence == update.sequence)
        {
            self.remove_video_playback(update.media);
            self.cancel_pending_media_load_sequence_if_matches(update.media, update.sequence);
        }
    }

    fn prepare_video_update(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) -> Option<VideoUpdateTask> {
        let video = self.video_playbacks.get(&media)?;
        debug_assert!(video.task.is_none());
        let slot = VideoUpdateSlot::default();
        let payload = PendingMediaElementEventPayload::VideoUpdate(PendingVideoUpdate {
            media,
            sequence: video.sequence,
            slot: slot.clone(),
        });
        let (target, id) = self.prepare_media_element_event(scope, media, UPDATE_KIND, payload)?;
        self.video_playbacks.get_mut(&media)?.task = Some(id);
        Some(VideoUpdateTask {
            target,
            id,
            sender: self.page_media_element_event_sender().clone(),
            slot,
        })
    }

    pub(crate) fn start_buffered_video(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
        sequence: MediaLoadSequenceId,
        request: Option<u64>,
        len: usize,
        take_bytes: impl FnOnce() -> Bytes,
    ) -> bool {
        if !self.pending_media_load_sequence_is_current(media, sequence)
            || !self.video_playbacks.get(&media).is_some_and(|video| {
                video.sequence == sequence && video.worker.is_none() && video.task.is_none()
            })
        {
            return false;
        }
        let at_limit = self.video_decoder_limit_reached();
        let Some(task) = self.prepare_video_update(scope, media) else {
            return false;
        };
        let video = self.video_playbacks.get_mut(&media).expect("video target");
        video.network_request = request;
        let open = if len > MAX_VIDEO_BYTES || at_limit {
            Err(anyhow::anyhow!("video decode resource limit exceeded"))
        } else {
            let completion = task.clone();
            VideoWorker::open(take_bytes(), move |result| {
                completion.complete(VideoUpdate::Decode { result, seek: None });
            })
        };
        match open {
            Ok(worker) => video.worker = Some(worker),
            Err(error) => task.complete(VideoUpdate::Decode {
                result: Err(error),
                seek: None,
            }),
        }
        true
    }

    pub(super) fn video_decoder_limit_reached(&self) -> bool {
        self.video_playbacks
            .values()
            .filter(|video| video.worker.is_some())
            .count()
            >= 8
    }

    pub(crate) fn video_image(&self, media: DomHandle) -> Option<Arc<moli_image::RgbaImage>> {
        self.current_video(media)?
            .shown
            .as_ref()
            .map(|frame| frame.image.clone())
    }

    pub(crate) fn video_dimensions(&self, media: DomHandle) -> Option<(u32, u32)> {
        let video = self.current_video(media)?;
        if let Some(frame) = &video.shown {
            Some((frame.image.width, frame.image.height))
        } else {
            video
                .metadata
                .map(|metadata| (metadata.width, metadata.height))
        }
    }

    pub(crate) fn video_duration(&self, media: DomHandle) -> Option<f64> {
        self.current_video(media)?.duration()
    }

    pub(crate) fn video_buffered(&self, media: DomHandle) -> Vec<(f64, f64)> {
        let Some(video) = self.current_video(media) else {
            return Vec::new();
        };
        if let Some(fragment) = &video.fragment {
            return fragment.ranges.clone();
        }
        video
            .duration()
            .map(|duration| vec![(0.0, duration)])
            .unwrap_or_default()
    }

    pub(crate) fn video_current_time(&self, media: DomHandle) -> Option<f64> {
        self.current_video(media).map(VideoPlayback::current_time)
    }

    pub(crate) fn video_ended(&self, media: DomHandle) -> bool {
        self.current_video(media).is_some_and(|video| video.at_end)
    }

    pub(crate) fn manages_video(&self, media: DomHandle) -> bool {
        self.current_video(media).is_some()
    }

    pub(super) fn current_video(&self, media: DomHandle) -> Option<&VideoPlayback> {
        let video = self.video_playbacks.get(&media)?;
        (self.dom_host().node(media).and_then(Node::owner_document) == Some(video.document))
            .then_some(video)
    }

    fn note_video_frame(&self, intrinsic_changed: bool) {
        self.image_resources.bump_visual_generation();
        if intrinsic_changed {
            self.note_realtime_layout_input_change();
        } else {
            self.request_realtime_rendering_opportunity();
        }
    }

    pub(crate) fn wait_for_video_play(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
        resolver: v8::Local<'_, v8::PromiseResolver>,
    ) {
        if let Some(video) = self.video_playbacks.get_mut(&media) {
            video.play_promises.push(v8::Global::new(scope, resolver));
        }
    }

    pub(crate) fn stop_video_clock(&mut self, scope: &mut v8::PinScope<'_, '_>, media: DomHandle) {
        self.cancel_video_timer(media);
        let Some(video) = self.video_playbacks.get_mut(&media) else {
            return;
        };
        video.update_clock(false, 0.0);
        settle_play_promises(
            scope,
            std::mem::take(&mut video.play_promises),
            Some("AbortError"),
        );
    }

    pub(crate) fn fail_video_load(&mut self, scope: &mut v8::PinScope<'_, '_>, media: DomHandle) {
        self.cancel_video_timer(media);
        if let Some(video) = self.video_playbacks.get_mut(&media) {
            video.update_clock(false, 0.0);
            video.worker = None;
            settle_play_promises(
                scope,
                std::mem::take(&mut video.play_promises),
                Some("NotSupportedError"),
            );
            self.set_media_paused(media, true);
        }
    }

    pub(crate) fn update_video_clock(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) {
        if !self.manages_video(media) {
            return;
        }
        self.cancel_video_timer(media);
        let Some(element) = self.dom_host().node(media).and_then(Node::as_element) else {
            return;
        };
        let playing =
            !element.media_paused() && !element.media_seeking() && element.media_ready_state() >= 2;
        let rate = element.media_playback_rate();
        self.update_video_audio_gain(media);
        let Some(video) = self.video_playbacks.get_mut(&media) else {
            return;
        };
        let awaiting_data = video
            .fragment
            .as_ref()
            .is_some_and(|fragment| fragment.waiting);
        video.update_clock(playing && video.metadata.is_some() && !awaiting_data, rate);
        self.pump_video(scope, media);
    }

    pub(crate) fn update_video_audio_gain(&self, media: DomHandle) {
        let Some(audio) = self
            .current_video(media)
            .and_then(|video| video.fragment.as_ref())
            .and_then(|fragment| fragment.audio.as_ref())
        else {
            return;
        };
        if let Some(element) = self.dom_host().node(media).and_then(Node::as_element) {
            audio.set_gain(if element.media_muted() {
                0.0
            } else {
                element.media_volume() as f32
            });
        }
    }

    pub(crate) fn seek_video(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
        time: f64,
        token: u64,
    ) -> bool {
        if !self.manages_video(media) {
            return false;
        }
        self.cancel_video_timer(media);
        if self
            .dom_host()
            .node(media)
            .and_then(Node::as_element)
            .and_then(|element| element.media_error_code())
            .is_some()
        {
            return false;
        }
        let video = self.video_playbacks.get_mut(&media).expect("video target");
        video.update_clock(false, 0.0);
        video.position = time;
        video.at_end = false;
        video.drained = false;
        video.queued = None;
        video.seek = Some((time, token));
        self.pump_video(scope, media);
        true
    }

    fn cancel_video_timer(&mut self, media: DomHandle) {
        if let Some(video) = self.video_playbacks.get_mut(&media)
            && let Some(timer) = video.timer.take()
        {
            timer.abort();
            if let Some(task) = video.task.take() {
                self.media_element_events.remove(task);
            }
        }
    }

    pub(super) fn pump_video(&mut self, scope: &mut v8::PinScope<'_, '_>, media: DomHandle) {
        let Some(video) = self.video_playbacks.get_mut(&media) else {
            return;
        };
        if video.task.is_some() || video.worker.is_none() {
            return;
        }
        if video.metadata.is_some()
            && let Some((position, token)) = video.seek.take()
        {
            self.request_video_decode(scope, media, DecodeRequest::Seek(position), Some(token));
            return;
        }
        if let Some(fragment) = &video.fragment {
            if fragment.waiting {
                return;
            }
            if video.metadata.is_none() {
                self.request_video_decode(scope, media, DecodeRequest::NextFrame, None);
                return;
            }
        } else if video.metadata.is_none() {
            return;
        }
        let Some(clock) = video.clock else { return };
        let deadline = video
            .queued
            .as_ref()
            .map(|frame| frame.timestamp)
            .or_else(|| {
                video.drained.then(|| {
                    video
                        .duration()
                        .or_else(|| {
                            video
                                .shown
                                .as_ref()
                                .map(|frame| frame.timestamp + frame.duration)
                        })
                        .unwrap_or(video.position)
                })
            });
        if let Some(deadline) = deadline {
            let delay = ((deadline - video.current_time()) / clock.rate).clamp(0.0, 60.0);
            let Some(task) = self.prepare_video_update(scope, media) else {
                return;
            };
            let timer = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs_f64(delay)).await;
                task.complete(VideoUpdate::Present);
            });
            if let Some(video) = self.video_playbacks.get_mut(&media) {
                video.timer = Some(timer.abort_handle());
            }
        } else {
            self.request_video_decode(scope, media, DecodeRequest::NextFrame, None);
        }
    }

    fn request_video_decode(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
        request: DecodeRequest,
        seek: Option<u64>,
    ) {
        let Some(task) = self.prepare_video_update(scope, media) else {
            return;
        };
        let video = self.video_playbacks.get(&media).expect("video target");
        let completion = task.clone();
        let result =
            video
                .worker
                .as_ref()
                .expect("video decoder")
                .request(request, move |result| {
                    completion.complete(VideoUpdate::Decode { result, seek });
                });
        if let Err(error) = result {
            task.complete(VideoUpdate::Decode {
                result: Err(error),
                seek,
            });
        }
    }
}

pub(super) fn apply_video_update(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    PendingVideoUpdate {
        media,
        sequence,
        slot,
    }: PendingVideoUpdate,
) -> bool {
    let Some(update) = slot.lock().take() else {
        return false;
    };
    let host = unsafe { &mut *host_ptr };
    if !host
        .current_video(media)
        .is_some_and(|video| video.sequence == sequence)
    {
        host.discard_video_update(PendingVideoUpdate {
            media,
            sequence,
            slot,
        });
        return false;
    }
    let video = host.video_playbacks.get_mut(&media).expect("current video");
    video.task = None;
    video.timer = None;
    match update {
        VideoUpdate::Decode {
            result: Ok(DecodeResult::Waiting),
            seek,
        } => {
            if let Some(fragment) = &mut video.fragment {
                fragment.waiting = true;
                video.update_clock(false, 0.0);
            }
            if let Some(token) = seek {
                let _ = host.queue_media_seek_completion(scope, media, token);
            }
        }
        VideoUpdate::Decode { result, seek } if video.metadata.is_none() => {
            let network_request = video.network_request;
            let success = match result {
                Ok(DecodeResult::Opened {
                    metadata,
                    first_frame,
                }) => {
                    video.metadata = Some(metadata);
                    video.shown = Some(first_frame);
                    host.note_video_frame(true);
                    true
                }
                Ok(DecodeResult::Frame(_) | DecodeResult::Waiting) => false,
                Err(error) => {
                    tracing::debug!(?error, "MP4 video decoder could not open the input");
                    video.worker = None;
                    false
                }
            };
            debug_assert!(seek.is_none());
            let followup = if let Some(request) = network_request {
                host.complete_pending_media_load_network_request_if_matches(
                    media, sequence, request, success,
                )
            } else {
                host.complete_pending_media_load_local_resource_if_matches(media, sequence, success)
            };
            queue_media_load_network_terminal_followup(scope, host_ptr, media, sequence, followup);
            if success {
                unsafe { &mut *host_ptr }.pump_video(scope, media);
            }
        }
        VideoUpdate::Decode {
            result: Ok(DecodeResult::Frame(frame)),
            seek,
        } => {
            if let Some(token) = seek {
                if host.dom_host().media_seek_token(media) == Some(token) {
                    if let Some(frame) = frame {
                        if let Some(fragment) = &mut host
                            .video_playbacks
                            .get_mut(&media)
                            .expect("current video")
                            .fragment
                        {
                            fragment.waiting = false;
                        }
                        present_frame(host, media, frame);
                    }
                    let _ = host.queue_media_seek_completion(scope, media, token);
                }
            } else if video.seek.is_none() {
                video.drained = frame.is_none();
                video.queued = frame;
            }
            host.pump_video(scope, media);
        }
        VideoUpdate::Decode { result, .. } => {
            if let Err(error) = result {
                tracing::debug!(?error, "H.264 video decode failed");
            }
            host.fail_video_load(scope, media);
            host.set_media_error_code(media, Some(3));
            host.set_media_seeking(media, false);
            dispatch_text_control_event(scope, host_ptr, media, "error");
        }
        VideoUpdate::Present => present_video(scope, host_ptr, media),
    }
    true
}

fn present_frame(host: &mut PageScriptHost, media: DomHandle, frame: VideoFrame) {
    let video = host.video_playbacks.get_mut(&media).expect("current video");
    let dimensions = (frame.image.width, frame.image.height);
    let resized = video
        .shown
        .as_ref()
        .is_none_or(|shown| (shown.image.width, shown.image.height) != dimensions);
    video.shown = Some(frame);
    host.note_video_frame(resized);
}

fn present_video(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    media: DomHandle,
) {
    let host = unsafe { &mut *host_ptr };
    let video = host.video_playbacks.get_mut(&media).expect("current video");
    if video.clock.is_none() {
        return;
    }
    let position = video.current_time();
    if video
        .queued
        .as_ref()
        .is_some_and(|frame| frame.timestamp > position + 0.001)
    {
        host.pump_video(scope, media);
        return;
    }
    let frame = video.queued.take();
    let ended = video.drained
        && frame.is_none()
        && video
            .duration()
            .is_none_or(|duration| position + 0.001 >= duration);
    let now = Instant::now();
    let timeupdate = ended
        || video
            .last_timeupdate
            .is_none_or(|last| now - last >= Duration::from_millis(250));
    if timeupdate {
        video.last_timeupdate = Some(now);
    }
    if ended {
        video.update_clock(false, 0.0);
        video.position = video.duration().unwrap_or(position);
        video.at_end = true;
        host.set_media_paused(media, true);
    }
    if let Some(frame) = frame {
        present_frame(host, media, frame);
    }
    host.set_media_current_time(media, position);
    host.pump_video(scope, media);
    if timeupdate {
        refresh_media_active_text_track_cues(scope, host_ptr, media);
        dispatch_text_control_event(scope, host_ptr, media, "timeupdate");
    }
    if ended && unsafe { &*host_ptr }.video_ended(media) {
        dispatch_text_control_event(scope, host_ptr, media, "ended");
    }
}

pub(crate) fn start_video_if_playing(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    media: DomHandle,
) {
    let host = unsafe { &mut *host_ptr };
    let Some(element) = host.dom_host().node(media).and_then(Node::as_element) else {
        return;
    };
    if element.media_paused() || element.media_seeking() || element.media_ready_state() < 2 {
        return;
    }
    let Some(video) = host.video_playbacks.get_mut(&media) else {
        return;
    };
    if video.metadata.is_none() {
        return;
    }
    let already_playing = video.clock.is_some();
    settle_play_promises(scope, std::mem::take(&mut video.play_promises), None);
    host.update_video_clock(scope, media);
    if !already_playing {
        dispatch_text_control_event(scope, host_ptr, media, "playing");
    }
}

fn settle_play_promises(
    scope: &mut v8::PinScope<'_, '_>,
    promises: Vec<v8::Global<v8::PromiseResolver>>,
    error: Option<&str>,
) {
    if promises.is_empty() {
        return;
    }
    let value = if let Some(name) = error {
        crate::native_bridge::abort::dom_exception_value(
            scope,
            "Video playback was interrupted.",
            name,
        )
    } else {
        v8::undefined(scope).into()
    };
    for promise in promises {
        let promise = v8::Local::new(scope, promise);
        if error.is_some() {
            let _ = promise.reject(scope, value);
        } else {
            let _ = promise.resolve(scope, value);
        }
    }
}
