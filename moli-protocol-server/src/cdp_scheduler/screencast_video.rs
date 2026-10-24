use std::time::Instant;

use moli_core::page::RendererCapturedVideoFrame;
use moli_protocol::{PageScreencastRegistration, PageScreencastSubscriptionStatus};
use moli_screencast_video::{RecorderHandle, RecorderStatus};

use super::CdpScheduler;

pub(super) struct ScreencastVideo {
    recorder: RecorderHandle,
    source: Option<PageScreencastRegistration>,
    needs_first_frame: bool,
}

impl Drop for ScreencastVideo {
    fn drop(&mut self) {
        self.recorder.stop(Instant::now());
    }
}

impl CdpScheduler {
    pub(crate) fn enable_screencast_video(&mut self, recorder: RecorderHandle) {
        recorder.pause(Instant::now());
        self.screencast_video = Some(Box::new(ScreencastVideo {
            recorder,
            source: None,
            needs_first_frame: true,
        }));
    }

    pub(super) fn refresh_screencast_video_source(&mut self) {
        let Some(video) = self.screencast_video.as_mut() else {
            return;
        };
        if self.conn.default_target_is_closed()
            || matches!(
                video.recorder.status(),
                RecorderStatus::Finished(_) | RecorderStatus::Failed(_)
            )
        {
            self.screencast_video = None;
            return;
        }
        let active = self.page_screencasts.values().filter(|schedule| {
            self.conn
                .page_screencast_targets_main_page(&schedule.registration)
                && self
                    .conn
                    .page_screencast_subscription_status(&schedule.registration)
                    != PageScreencastSubscriptionStatus::Inactive
        });
        if let Some(source) = video.source.as_ref()
            && active
                .clone()
                .any(|schedule| same_subscription(source, &schedule.registration))
        {
            return;
        }
        let replacement = active
            .min_by_key(|schedule| (schedule.next_due_at, schedule.registration.session_id()))
            .map(|schedule| schedule.registration.clone());
        video.source = replacement;
        video.needs_first_frame = true;
        if video.source.is_some() {
            video.recorder.resume(Instant::now());
        } else {
            // A subscription may stop and restart during the same Page's
            // lifetime. Keep the file open and cut this inactive interval.
            video.recorder.pause(Instant::now());
        }
    }

    pub(super) fn capture_video_for(&self, registration: &PageScreencastRegistration) -> bool {
        self.screencast_video.as_ref().is_some_and(|video| {
            video
                .source
                .as_ref()
                .is_some_and(|source| same_subscription(source, registration))
        })
    }

    pub(super) fn screencast_video_needs_first_frame(&self) -> bool {
        self.screencast_video
            .as_ref()
            .is_some_and(|video| video.needs_first_frame)
    }

    pub(super) fn accept_screencast_video_frame(
        &mut self,
        session_id: &Option<String>,
        generation: i32,
        frame: RendererCapturedVideoFrame,
    ) {
        let Some(video) = self.screencast_video.as_mut() else {
            return;
        };
        if video.source.as_ref().is_some_and(|source| {
            source.session_id() == session_id.as_deref() && source.generation() == generation
        }) && video.recorder.offer(frame.image, frame.captured_at)
        {
            video.needs_first_frame = false;
        }
    }
}

fn same_subscription(
    left: &PageScreencastRegistration,
    right: &PageScreencastRegistration,
) -> bool {
    left.session_id() == right.session_id() && left.generation() == right.generation()
}
