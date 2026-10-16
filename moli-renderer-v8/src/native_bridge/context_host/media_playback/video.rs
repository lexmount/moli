//! One owned decoder actor per element. Native decoding runs outside the VM;
//! an exact, current media task publishes each immutable frame.

use super::super::{
    decode_budget::{DecodeJobPermit, DecodedBytesPermit},
    media_element_events::PendingMediaElementEvent,
    media_element_events::PendingMediaElementEventPayload,
};
use super::*;
use parking_lot::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

type Completion = Arc<Mutex<Option<Result<VideoSnapshot, String>>>>;

#[cfg(test)]
pub(crate) struct VideoDecodeBudgetProbe {
    budget: super::super::decode_budget::SharedDecodeBudget,
}

#[cfg(test)]
impl VideoDecodeBudgetProbe {
    pub(crate) fn is_released(&self) -> bool {
        self.budget.counters() == (0, 0, 0)
    }
}

#[derive(Clone)]
struct VideoSnapshot {
    metadata: moli_media::VideoMetadata,
    frame: Arc<moli_media::DecodedVideoFrame>,
    // Keep the last published frame charged if the actor stops.
    _charge: Arc<Mutex<DecodedBytesPermit>>,
}

#[derive(Clone, Copy)]
struct VideoLoadBinding {
    sequence: crate::native_bridge::MediaLoadSequenceId,
    network_id: Option<u64>,
}

struct PendingDecode {
    id: RendererPageMediaElementEventTaskId,
    cancelled: Arc<AtomicBool>,
}

impl Drop for PendingDecode {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

struct DecodeRequest {
    position: f64,
    cancelled: Arc<AtomicBool>,
    completion: Completion,
    target: WindowTaskTarget,
    id: RendererPageMediaElementEventTaskId,
}

struct VideoDecoderWorker {
    request: Arc<Mutex<Option<DecodeRequest>>>,
    wake: tokio::sync::mpsc::Sender<()>,
    stopped: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for VideoDecoderWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

struct DecodeControl {
    stopped: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    charge: Arc<Mutex<DecodedBytesPermit>>,
}

struct VideoDecoderState {
    source: moli_media::WebmVideo,
    _charge: Arc<Mutex<DecodedBytesPermit>>,
    _permit: Arc<Mutex<DecodeJobPermit>>,
}

impl moli_media::VideoDecodeControl for DecodeControl {
    fn is_cancelled(&self) -> bool {
        self.stopped.load(Ordering::Acquire) || self.cancelled.load(Ordering::Acquire)
    }
    fn admit_retained_bytes(&mut self, bytes: usize) -> Result<(), moli_media::VideoError> {
        self.charge
            .lock()
            .resize(bytes)
            .map_err(|_| moli_media::VideoError::Limit)
    }
}

impl VideoDecoderWorker {
    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.task.abort();
    }

    fn new(
        runner: crate::network::RendererResourceTaskRunner,
        sender: crate::page_task_queue::RendererPageMediaElementEventSender,
        bytes: Arc<[u8]>,
        permit: DecodeJobPermit,
        charge: DecodedBytesPermit,
    ) -> Self {
        let request = Arc::new(Mutex::new(None::<DecodeRequest>));
        let (wake, mut ready) = tokio::sync::mpsc::channel::<()>(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let requests = request.clone();
        let stop = stopped.clone();
        let task = runner.spawn_abortable(async move {
            let charge = Arc::new(Mutex::new(charge));
            let permit = Arc::new(Mutex::new(permit));
            let mut encoded = Some(bytes);
            let mut source: Option<VideoDecoderState> = None;
            while ready.recv().await.is_some() {
                let Some(request) = requests.lock().take() else {
                    continue;
                };
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let input = encoded.clone();
                let mut control = DecodeControl {
                    stopped: stop.clone(),
                    cancelled: request.cancelled.clone(),
                    charge: charge.clone(),
                };
                let job_permit = permit.clone();
                let position = request.position;
                let decoded = tokio::task::spawn_blocking(move || {
                    // Admission lives until native work exits, even when its
                    // awaiting actor has already been aborted.
                    let permit = job_permit;
                    let mut source = source;
                    let result: Result<VideoSnapshot, moli_media::VideoError> = (|| {
                        if source.is_none() {
                            source = Some(VideoDecoderState {
                                source: moli_media::WebmVideo::open_with_control(
                                    input.as_deref().ok_or(moli_media::VideoError::Empty)?,
                                    &mut control,
                                )?,
                                _charge: control.charge.clone(),
                                _permit: permit.clone(),
                            });
                        }
                        let source = &mut source.as_mut().expect("opened video").source;
                        let frame = source.frame_at_with_control(position, &mut control)?;
                        Ok(VideoSnapshot {
                            metadata: source.metadata(),
                            frame,
                            _charge: control.charge.clone(),
                        })
                    })(
                    );
                    (source, result)
                })
                .await;
                let result = match decoded {
                    Ok((returned, result)) => {
                        source = returned;
                        if source.is_some() {
                            encoded = None;
                            permit.lock().release_encoded_bytes();
                        }
                        result.map_err(|error| error.to_string())
                    }
                    Err(error) => {
                        source = None;
                        Err(format!("video decoder task failed: {error}"))
                    }
                };
                if stop.load(Ordering::Acquire) {
                    break;
                }
                if request.cancelled.load(Ordering::Acquire) {
                    continue;
                }
                *request.completion.lock() = Some(result);
                if sender
                    .send(
                        request.target,
                        request.id,
                        RendererPageMediaElementEventTaskKind::Playback,
                    )
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            request,
            wake,
            stopped,
            task,
        }
    }
}

pub(super) struct VideoElementResource {
    worker: VideoDecoderWorker,
    pending: Option<PendingDecode>,
    cached: Option<VideoSnapshot>,
    origin_clean: bool,
    load: Option<VideoLoadBinding>,
    desired_position: f64,
    decoded_position: Option<f64>,
    seek_token: Option<u64>,
    failed: bool,
}

pub(in crate::native_bridge::context_host) struct MediaVideoDecodeTask {
    media: DomHandle,
    document: DomHandle,
    epoch: u64,
    target: WindowTaskTarget,
    id: RendererPageMediaElementEventTaskId,
    position: f64,
    completion: Completion,
}

impl JsContextHost {
    #[cfg(test)]
    pub(crate) fn video_decode_budget_probe_for_test(&self) -> VideoDecodeBudgetProbe {
        VideoDecodeBudgetProbe {
            budget: self.media_playback.video_decode_budget.clone(),
        }
    }

    pub(super) fn cancel_media_video_resource(&mut self, media: DomHandle) {
        if let Some(video) = self
            .media_playback
            .elements
            .get_mut(&media)
            .and_then(|state| state.video.take())
        {
            if let Some(pending) = &video.pending {
                self.media_element_events.remove(pending.id);
            }
            drop(video);
        }
    }

    pub(super) fn begin_media_video_resource(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
        sequence: crate::native_bridge::MediaLoadSequenceId,
        bytes: &[u8],
        origin_clean: bool,
        network_id: Option<u64>,
    ) -> Result<bool, moli_media::VideoError> {
        if !self.pending_media_load_sequence_is_current(media, sequence) {
            return Err(moli_media::VideoError::Empty);
        }
        let element = self
            .dom_host()
            .node(media)
            .and_then(crate::dom::native::Node::as_element)
            .ok_or(moli_media::VideoError::Empty)?;
        if !element.is_html_element("video") || !bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
            return Ok(false);
        }
        if bytes.len() > moli_media::MAX_ENCODED_VIDEO_BYTES {
            return Err(moli_media::VideoError::Limit);
        }
        let position = element.media_current_time();
        let target = self
            .media_element_task_target(scope, media)
            .ok_or(moli_media::VideoError::Empty)?;
        let task_document = self
            .resolve_media_element_task_context(scope, target)
            .ok_or(moli_media::VideoError::Empty)?
            .document_handle;
        let loader = self
            .document_resource_loader_for_dispatch_scope(target.dispatch_scope())
            .ok_or(moli_media::VideoError::Empty)?;
        let permit = self
            .media_playback
            .video_decode_budget
            .admit_job(bytes.len())
            .map_err(|_| moli_media::VideoError::Limit)?;
        let charge = self
            .media_playback
            .video_decode_budget
            .reserve_decoded(0)
            .map_err(|_| moli_media::VideoError::Limit)?;
        let worker = VideoDecoderWorker::new(
            loader.task_runner(),
            self.page_media_element_event_sender().clone(),
            Arc::from(bytes),
            permit,
            charge,
        );
        self.cancel_media_video_resource(media);
        let state = self
            .media_playback_state_for(media)
            .ok_or(moli_media::VideoError::Empty)?;
        state.target = Some(target);
        state.task_document = Some(task_document);
        state.video = Some(VideoElementResource {
            worker,
            pending: None,
            cached: None,
            origin_clean,
            load: Some(VideoLoadBinding {
                sequence,
                network_id,
            }),
            desired_position: position,
            decoded_position: None,
            seek_token: None,
            failed: false,
        });
        self.request_media_video_frame(media, position, false);
        Ok(true)
    }

    pub(crate) fn request_media_video_frame(
        &mut self,
        media: DomHandle,
        position: f64,
        replace: bool,
    ) {
        let Some(state) = self.media_playback.elements.get_mut(&media) else {
            return;
        };
        let Some(video) = state.video.as_mut() else {
            return;
        };
        if video.failed {
            return;
        }
        video.desired_position = position;
        if video.decoded_position == Some(position) && video.pending.is_none() {
            return;
        }
        if video.pending.is_some() && !replace {
            return;
        }
        if let Some(previous) = video.pending.take() {
            self.media_element_events.remove(previous.id);
        }
        let Some(target) = state.target else { return };
        if self.current_media_window_task_target(target.dispatch_scope()) != Some(target) {
            self.cancel_media_video_resource(media);
            return;
        }
        let id = self
            .media_element_events
            .allocate_task_id(RendererPageMediaElementEventTaskId::from_raw);
        let completion = Arc::new(Mutex::new(None));
        let cancelled = Arc::new(AtomicBool::new(false));
        let state = self
            .media_playback
            .elements
            .get_mut(&media)
            .expect("current media");
        let video = state.video.as_mut().expect("current video");
        let task = MediaVideoDecodeTask {
            media,
            document: state.document,
            epoch: state.epoch,
            target,
            id,
            position,
            completion: completion.clone(),
        };
        self.media_element_events
            .push(PendingMediaElementEvent::new(
                id,
                target,
                RendererPageMediaElementEventTaskKind::Playback,
                PendingMediaElementEventPayload::VideoDecode(task),
            ));
        video.pending = Some(PendingDecode {
            id,
            cancelled: cancelled.clone(),
        });
        *video.worker.request.lock() = Some(DecodeRequest {
            position,
            cancelled,
            completion: completion.clone(),
            target,
            id,
        });
        if let Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) =
            video.worker.wake.try_send(())
        {
            *completion.lock() = Some(Err("video decoder owner was closed".to_owned()));
            let _ = self.page_media_element_event_sender().send(
                target,
                id,
                RendererPageMediaElementEventTaskKind::Playback,
            );
        }
    }

    pub(crate) fn defer_video_seek_completion(&mut self, media: DomHandle, token: u64) -> bool {
        let Some(position) = self
            .dom_host()
            .node(media)
            .and_then(crate::dom::native::Node::as_element)
            .map(|element| element.media_current_time())
        else {
            return false;
        };
        let Some(video) = self
            .media_playback
            .elements
            .get_mut(&media)
            .and_then(|state| state.video.as_mut())
        else {
            return false;
        };
        if video.failed || video.decoded_position == Some(position) && video.pending.is_none() {
            return false;
        }
        video.seek_token = Some(token);
        self.request_media_video_frame(media, position, true);
        true
    }

    pub(crate) fn finish_video_seek_if_ready(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        media: DomHandle,
    ) -> bool {
        if self
            .dom_host()
            .node(media)
            .and_then(crate::dom::native::Node::as_element)
            .is_none_or(|element| element.media_ready_state() < 2)
        {
            return false;
        }
        let Some(video) = self
            .media_playback
            .elements
            .get_mut(&media)
            .and_then(|state| state.video.as_mut())
            .filter(|video| video.seek_token.is_some())
        else {
            return false;
        };
        let token =
            if video.pending.is_none() && video.decoded_position == Some(video.desired_position) {
                video.seek_token.take()
            } else {
                None
            };
        if let Some(token) = token {
            self.queue_media_seek_completion(scope, media, token);
        }
        true
    }

    pub(crate) fn media_video_metadata(
        &self,
        media: DomHandle,
    ) -> Option<moli_media::VideoMetadata> {
        let node = self.dom_host().node(media)?;
        if node.as_element()?.media_ready_state() < 1 {
            return None;
        }
        let state = self.media_playback.elements.get(&media)?;
        if Some(state.document) != node.owner_document() {
            return None;
        }
        Some(state.video.as_ref()?.cached.as_ref()?.metadata)
    }

    pub(crate) fn media_video_frame_for_canvas(
        &self,
        media: DomHandle,
    ) -> Option<(Arc<moli_media::DecodedVideoFrame>, bool)> {
        let node = self.dom_host().node(media)?;
        if node.as_element()?.media_ready_state() < 2 {
            return None;
        }
        let state = self.media_playback.elements.get(&media)?;
        if Some(state.document) != node.owner_document() {
            return None;
        }
        let video = state.video.as_ref()?;
        Some((video.cached.as_ref()?.frame.clone(), video.origin_clean))
    }
}

impl MediaVideoDecodeTask {
    pub(in crate::native_bridge::context_host) fn is_pending(&self, host: &JsContextHost) -> bool {
        host.playback_is_current(self.media, self.document, self.epoch)
            && host
                .media_playback
                .elements
                .get(&self.media)
                .and_then(|state| state.video.as_ref())
                .and_then(|video| video.pending.as_ref())
                .is_some_and(|pending| pending.id == self.id)
            && host.current_media_window_task_target(self.target.dispatch_scope())
                == Some(self.target)
    }

    pub(in crate::native_bridge::context_host) fn is_current(&self, host: &JsContextHost) -> bool {
        self.is_pending(host) && self.completion.lock().is_some()
    }

    pub(in crate::native_bridge::context_host) fn dispatch(
        self,
        scope: &mut v8::PinScope<'_, '_>,
        host: *mut JsContextHost,
    ) -> bool {
        if !self.is_current(unsafe { &*host }) {
            return false;
        }
        let Some(result) = self.completion.lock().take() else {
            return false;
        };
        let (load, successful) = {
            let runtime = unsafe { &mut *host };
            let state = runtime
                .media_playback
                .elements
                .get_mut(&self.media)
                .expect("current decode");
            let video = state.video.as_mut().expect("current video");
            video.pending = None;
            match result {
                Ok(snapshot) => {
                    state.duration = snapshot.metadata.duration;
                    video.cached = Some(snapshot);
                    video.decoded_position = Some(self.position);
                }
                Err(error) => {
                    tracing::debug!(%error, "media video decoding failed");
                    video.failed = true;
                    video.worker.stop();
                }
            }
            (video.load.take(), !video.failed)
        };
        if let Some(load) = load {
            let followup = {
                let runtime = unsafe { &mut *host };
                if let Some(internal_id) = load.network_id {
                    runtime.complete_pending_media_load_network_request_if_matches(
                        self.media,
                        load.sequence,
                        internal_id,
                        successful,
                    )
                } else {
                    runtime.complete_pending_media_load_local_resource_if_matches(
                        self.media,
                        load.sequence,
                        successful,
                    )
                }
            };
            element::queue_media_load_network_terminal_followup(
                scope,
                host,
                self.media,
                load.sequence,
                followup,
            );
        } else if !successful {
            let failure = {
                let runtime = unsafe { &mut *host };
                runtime.stop_media_clock(self.media);
                runtime.set_media_error_code(self.media, Some(3));
                runtime.set_media_paused(self.media, true);
                runtime.set_media_seeking(self.media, false);
                let promises = runtime.take_media_play_promises(self.media);
                runtime.capture_media_playback_task(
                    self.media,
                    PlaybackOperation::Reject(MediaPlayError::Abort),
                    promises,
                )
            };
            fire(scope, host, self.media, "error");
            if let Some(failure) = failure {
                failure.dispatch(scope, host);
            }
        }
        if successful
            && unsafe { &*host }.playback_is_current(self.media, self.document, self.epoch)
        {
            let runtime = unsafe { &mut *host };
            let position = runtime
                .media_playback
                .elements
                .get(&self.media)
                .and_then(|state| state.video.as_ref())
                .expect("current video")
                .desired_position;
            runtime.request_media_video_frame(self.media, position, false);
            runtime.finish_video_seek_if_ready(scope, self.media);
        }
        true
    }
}
