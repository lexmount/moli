use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

use moli_owner_queue::{OwnerReadyTaskRoute, OwnerReadyTaskSource, OwnerTaskReadySignal};

use super::{
    RendererPageRenderingUpdateOwner, RendererPageRenderingUpdateTask,
    RendererPageRenderingUpdateTaskId, RendererPageRenderingUpdateTaskKind,
};
use crate::{
    native_bridge::WindowDocumentTaskTarget,
    page_task_queue::{
        RendererOwnerWakeSender, RendererOwnerWakeSource, RendererPageTaskReadySignal,
        window_document_task_source::RendererPageWindowDocumentTaskRouteClosed,
    },
    resource_ready::{ReadyPageTask, RendererPageTaskReadyMetadata},
    runtime::RendererDocumentToken,
};

#[derive(Debug)]
enum RenderingAdmission {
    Ready(ReadyPageTask<RendererPageRenderingUpdateTask>),
    Scheduled(ReadyPageTask<RendererPageRenderingUpdateTask>),
}

impl RenderingAdmission {
    fn ready(&self) -> &ReadyPageTask<RendererPageRenderingUpdateTask> {
        match self {
            Self::Ready(task) => task,
            Self::Scheduled(_) => unreachable!("scheduled frames must pass deadline admission"),
        }
    }
}

/// Stored in the existing source Arc. Legacy sources allocate neither a
/// second queue nor a deadline slot and retain their immediate FIFO path.
#[derive(Debug)]
struct RenderingReadySignal {
    wake: RendererPageTaskReadySignal,
    // Sticky capability hint, not pending-work truth. Never clearing it avoids
    // racing a producer that is about to publish a scheduled admission.
    accepts_frames: AtomicBool,
}

impl OwnerTaskReadySignal for RenderingReadySignal {
    fn signal_ready(&self) {
        self.wake.signal_ready();
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPageRenderingUpdateRoute {
    route: OwnerReadyTaskRoute<RenderingAdmission, RenderingReadySignal>,
}

#[derive(Clone, Debug)]
pub(crate) struct RendererPageRenderingUpdateSender {
    route: RendererPageRenderingUpdateRoute,
    root_document: RendererDocumentToken,
}

impl RendererPageRenderingUpdateSender {
    pub(crate) fn is_open(&self) -> bool {
        self.route.route.is_open()
    }

    pub(crate) fn new(
        route: RendererPageRenderingUpdateRoute,
        root_document: RendererDocumentToken,
    ) -> Self {
        Self {
            route,
            root_document,
        }
    }

    fn task(
        &self,
        target: WindowDocumentTaskTarget,
        id: RendererPageRenderingUpdateTaskId,
        kind: RendererPageRenderingUpdateTaskKind,
    ) -> RendererPageRenderingUpdateTask {
        RendererPageRenderingUpdateTask::new(
            RendererPageRenderingUpdateOwner::new(self.root_document, target),
            id,
            kind,
        )
    }

    pub(crate) fn send(
        &self,
        target: WindowDocumentTaskTarget,
        id: RendererPageRenderingUpdateTaskId,
        kind: RendererPageRenderingUpdateTaskKind,
    ) -> Result<(), RendererPageWindowDocumentTaskRouteClosed> {
        debug_assert_ne!(kind, RendererPageRenderingUpdateTaskKind::FrameLifecycle);
        self.route
            .route
            .send_and_signal_if_newly_ready(RenderingAdmission::Ready(ReadyPageTask::new(
                self.task(target, id, kind),
            )))
            .map_err(|_| RendererPageWindowDocumentTaskRouteClosed)
    }

    pub(crate) fn schedule_frame(
        &self,
        target: WindowDocumentTaskTarget,
        id: RendererPageRenderingUpdateTaskId,
        deadline: Instant,
    ) -> Result<(), RendererPageWindowDocumentTaskRouteClosed> {
        self.route
            .route
            .signal()
            .accepts_frames
            .store(true, Ordering::Release);
        self.route
            .route
            .send_and_signal_if_newly_ready(RenderingAdmission::Scheduled(ReadyPageTask::at(
                self.task(
                    target,
                    id,
                    RendererPageRenderingUpdateTaskKind::FrameLifecycle,
                ),
                deadline,
            )))
            .map_err(|_| RendererPageWindowDocumentTaskRouteClosed)
    }
}

/// Stable Page owner residence. A future frame is extracted from producer
/// admission before any ready descriptor is exposed. It cannot block the
/// immediate FIFO and is promoted only by the normal owner poll after expiry.
#[derive(Debug)]
pub(crate) struct RendererPageRenderingUpdateSource {
    source: OwnerReadyTaskSource<RenderingAdmission, RenderingReadySignal>,
    scheduled: Option<Box<ReadyPageTask<RendererPageRenderingUpdateTask>>>,
}

impl RendererPageRenderingUpdateSource {
    pub(crate) fn new(
        owner_wake: RendererOwnerWakeSender,
        source: RendererOwnerWakeSource,
    ) -> Self {
        Self {
            source: OwnerReadyTaskSource::new(RenderingReadySignal {
                wake: RendererPageTaskReadySignal::new(owner_wake, source),
                accepts_frames: AtomicBool::new(false),
            }),
            scheduled: None,
        }
    }

    pub(crate) fn route(&self) -> RendererPageRenderingUpdateRoute {
        RendererPageRenderingUpdateRoute {
            route: self.source.route(),
        }
    }

    pub(crate) fn route_matches(&self, route: &RendererPageRenderingUpdateRoute) -> bool {
        route.route.same_source_as(&self.source)
    }

    fn accepts_frames(&self) -> bool {
        self.source.signal().accepts_frames.load(Ordering::Acquire)
    }

    fn with_ready_tasks<R>(
        &mut self,
        now: Option<Instant>,
        operation: impl FnOnce(&mut VecDeque<RenderingAdmission>) -> R,
    ) -> R {
        let accepts_frames = self.accepts_frames();
        let scheduled = &mut self.scheduled;
        self.source.with_tasks_mut(|tasks| {
            // The head check covers the first concurrent admission between
            // reading the sticky hint and acquiring the source lock.
            if accepts_frames || matches!(tasks.front(), Some(RenderingAdmission::Scheduled(_))) {
                accept_scheduled(tasks, scheduled);
                let now = now.unwrap_or_else(Instant::now);
                if scheduled.as_ref().is_some_and(|task| task.ready_at <= now) {
                    let task = *scheduled.take().expect("due frame");
                    tasks.push_back(RenderingAdmission::Ready(task));
                }
            }
            operation(tasks)
        })
    }

    pub(crate) fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        self.with_ready_tasks(None, |tasks| {
            tasks.front().map(|task| task.ready().metadata())
        })
    }

    pub(crate) fn next_ready_head(&mut self) -> Option<RendererPageRenderingUpdateTask> {
        self.with_ready_tasks(None, |tasks| {
            tasks.front().map(|task| *task.ready().value())
        })
    }

    pub(crate) fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageRenderingUpdateTask,
    )> {
        self.with_ready_tasks(None, |tasks| {
            tasks.pop_front().map(|task| match task {
                RenderingAdmission::Ready(task) => task.into_parts(),
                RenderingAdmission::Scheduled(_) => unreachable!("unaccepted frame"),
            })
        })
    }

    pub(crate) fn has_ready_task(&mut self) -> bool {
        self.next_ready_metadata().is_some()
    }

    pub(crate) fn next_deadline(
        &mut self,
        current: Option<RendererPageRenderingUpdateTask>,
    ) -> Option<Instant> {
        if !self.accepts_frames() {
            return None;
        }
        let scheduled = &mut self.scheduled;
        self.source.with_tasks_mut(|tasks| {
            accept_scheduled(tasks, scheduled);
            if scheduled
                .as_ref()
                .is_some_and(|task| Some(task.value) != current)
            {
                *scheduled = None;
            }
            tasks.retain(|task| {
                let value = task.ready().value;
                value.kind() != RendererPageRenderingUpdateTaskKind::FrameLifecycle
                    || Some(value) == current
            });
            frame_deadline(tasks, scheduled.as_deref(), current)
        })
    }

    #[cfg(any(test, debug_assertions))]
    pub(crate) fn local_deadline(
        &self,
        current: Option<RendererPageRenderingUpdateTask>,
    ) -> Option<Instant> {
        if !self.accepts_frames() {
            return None;
        }
        self.source
            .with_local_tasks(|tasks| frame_deadline(tasks, self.scheduled.as_deref(), current))
    }

    pub(crate) fn clear(&mut self) {
        self.scheduled = None;
        self.source.clear_local();
    }

    #[cfg(test)]
    pub(crate) fn admit_due_at_for_test(&mut self, now: Instant) {
        self.with_ready_tasks(Some(now), |_| ());
    }
}

fn accept_scheduled(
    tasks: &mut VecDeque<RenderingAdmission>,
    scheduled: &mut Option<Box<ReadyPageTask<RendererPageRenderingUpdateTask>>>,
) {
    // Only the serialized Page Host schedules frames. A later ticket replaces
    // a canceled ticket or retired Document; it never authorizes that owner.
    for _ in 0..tasks.len() {
        match tasks.pop_front().expect("accepted admission") {
            RenderingAdmission::Ready(task) => tasks.push_back(RenderingAdmission::Ready(task)),
            RenderingAdmission::Scheduled(task) => *scheduled = Some(Box::new(task)),
        }
    }
}

fn frame_deadline(
    tasks: &VecDeque<RenderingAdmission>,
    scheduled: Option<&ReadyPageTask<RendererPageRenderingUpdateTask>>,
    current: Option<RendererPageRenderingUpdateTask>,
) -> Option<Instant> {
    let future = scheduled
        .filter(|task| Some(task.value) == current)
        .map(|task| task.ready_at);
    let ready = tasks
        .iter()
        .filter_map(|task| {
            let task = task.ready();
            (task.value.kind() == RendererPageRenderingUpdateTaskKind::FrameLifecycle
                && Some(task.value) == current)
                .then_some(task.ready_at)
        })
        .min();
    future.into_iter().chain(ready).min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PageId,
        frame_owner_model::{
            DocumentId, FrameDocumentTaskOwner, FrameSchedulerLaneId, LocalWindowId,
        },
        native_bridge::{OwnerDispatchScope, WindowDocumentOwner},
        runtime::RendererPageToken,
    };
    use std::time::Duration;

    fn fixture() -> (
        RendererPageRenderingUpdateSource,
        RendererPageRenderingUpdateSender,
        WindowDocumentTaskTarget,
    ) {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let page = PageId::new_for_testing(73);
        let source = RendererPageRenderingUpdateSource::new(
            RendererOwnerWakeSender::new(tx, RendererPageToken::new_for_testing(page)),
            RendererOwnerWakeSource::RenderingUpdateTask,
        );
        let sender = RendererPageRenderingUpdateSender::new(
            source.route(),
            RendererDocumentToken::new_for_testing(page, 1),
        );
        let target = WindowDocumentTaskTarget::new(
            WindowDocumentOwner::Frame(FrameDocumentTaskOwner::new(
                FrameSchedulerLaneId(3),
                LocalWindowId(4),
                DocumentId(5),
            )),
            OwnerDispatchScope::Top,
        );
        (source, sender, target)
    }

    #[test]
    fn realtime_future_frame_does_not_block_immediate_rendering_fifo() {
        let (mut source, sender, target) = fixture();
        let deadline = Instant::now() + Duration::from_secs(60);
        let frame_id = RendererPageRenderingUpdateTaskId::Frame(1);
        sender.schedule_frame(target, frame_id, deadline).unwrap();
        for id in [1, 2] {
            sender
                .send(
                    target,
                    RendererPageRenderingUpdateTaskId::from_raw(id),
                    RendererPageRenderingUpdateTaskKind::DocumentScrollEvents,
                )
                .unwrap();
        }
        for id in [1, 2] {
            assert_eq!(
                source.pop_front().unwrap().1.task_id(),
                RendererPageRenderingUpdateTaskId::from_raw(id)
            );
        }
        let frame = sender.task(
            target,
            frame_id,
            RendererPageRenderingUpdateTaskKind::FrameLifecycle,
        );
        assert!(!source.has_ready_task());
        assert_eq!(source.next_deadline(Some(frame)), Some(deadline));
        source.admit_due_at_for_test(deadline - Duration::from_nanos(1));
        assert!(!source.has_ready_task());
        source.admit_due_at_for_test(deadline);
        let (metadata, task) = source.pop_front().unwrap();
        assert_eq!(task, frame);
        assert_eq!(metadata.ready_at, deadline);
        assert!(source.next_deadline(None).is_none());
        assert!(!source.has_ready_task());
    }

    #[test]
    fn realtime_canceled_frame_and_replaced_namespace_lose_their_deadlines() {
        let (mut source, sender, target) = fixture();
        let deadline = Instant::now() + Duration::from_secs(60);
        let frame_id = RendererPageRenderingUpdateTaskId::Frame(1);
        sender.schedule_frame(target, frame_id, deadline).unwrap();
        let replacement = RendererPageRenderingUpdateTask::new(
            RendererPageRenderingUpdateOwner::new(
                RendererDocumentToken::new_for_testing(PageId::new_for_testing(73), 2),
                target,
            ),
            frame_id,
            RendererPageRenderingUpdateTaskKind::FrameLifecycle,
        );
        assert_eq!(source.next_deadline(Some(replacement)), None);
        assert_eq!(source.local_deadline(Some(replacement)), None);
        source.admit_due_at_for_test(deadline);
        assert!(source.pop_front().is_none());
        sender
            .schedule_frame(
                target,
                RendererPageRenderingUpdateTaskId::Frame(2),
                deadline,
            )
            .unwrap();
        source.admit_due_at_for_test(deadline);
        assert_eq!(
            source.next_deadline(None),
            None,
            "cancellation also retires a ready frame"
        );
        assert!(source.pop_front().is_none());
    }

    #[test]
    fn realtime_late_opportunity_runs_once_and_route_closure_rejects_new_work() {
        let (mut source, sender, target) = fixture();
        let deadline = Instant::now() + Duration::from_secs(60);
        assert!(!source.accepts_frames());
        assert!(source.next_deadline(None).is_none());
        assert!(source.scheduled.is_none());
        sender
            .schedule_frame(
                target,
                RendererPageRenderingUpdateTaskId::Frame(1),
                deadline,
            )
            .unwrap();
        source.admit_due_at_for_test(deadline + Duration::from_secs(3600));
        assert!(source.pop_front().is_some());
        assert!(source.pop_front().is_none());
        assert!(source.next_deadline(None).is_none());
        sender
            .schedule_frame(
                target,
                RendererPageRenderingUpdateTaskId::Frame(2),
                deadline,
            )
            .unwrap();
        source.clear();
        assert!(source.next_deadline(None).is_none());
        drop(source);
        assert!(
            sender
                .schedule_frame(
                    target,
                    RendererPageRenderingUpdateTaskId::Frame(3),
                    deadline
                )
                .is_err()
        );
    }
}
