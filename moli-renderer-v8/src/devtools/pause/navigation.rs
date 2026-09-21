#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RendererProvisionalLoad {
    WaitingForResponse(u64),
    ReplacingDocument(u64),
}

impl RendererProvisionalLoad {
    fn request_id(self) -> u64 {
        match self {
            Self::WaitingForResponse(id) | Self::ReplacingDocument(id) => id,
        }
    }
}

impl crate::devtools::target::RendererDevToolsTargetHandle {
    /// Records the current navigation without disturbing a paused document.
    /// The caller owns one current request and supplies its unique identity.
    pub fn navigation_started(&self, request_id: u64) {
        let mut state = self.pause_ref().shared.state.lock();
        if !state.target_closed
            && state
                .provisional_load
                .is_none_or(|load| load.request_id() != request_id)
        {
            state.provisional_load = Some(RendererProvisionalLoad::WaitingForResponse(request_id));
        }
    }

    /// Unblocks the old document only when this response will replace it.
    /// A canceled or superseded load job cannot reactivate this policy.
    pub fn navigation_response_ready(&self, request_id: u64) {
        let pause = self.pause_ref();
        let mut state = pause.shared.state.lock();
        if !state.target_closed
            && state.provisional_load
                == Some(RendererProvisionalLoad::WaitingForResponse(request_id))
        {
            state.provisional_load = Some(RendererProvisionalLoad::ReplacingDocument(request_id));
            pause.shared.pause_loop_wake.notify_all();
        }
    }

    /// Ends the matching provisional load at commit or cancellation. Retained
    /// target handles have no effect on this document's debugger policy.
    pub fn navigation_finished(&self, request_id: u64) {
        let mut state = self.pause_ref().shared.state.lock();
        if state
            .provisional_load
            .is_some_and(|load| load.request_id() == request_id)
        {
            state.provisional_load = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{RendererInspectorPauseBridge, RendererInspectorPauseExitReason};
    use crate::devtools::{
        ingress::{io::RendererInspectorIoIngress, main::RendererInspectorMainIngress},
        route::RendererInspectorSessionExecutorRouteId,
        target::RendererDevToolsTargetHandle,
    };

    fn target() -> RendererDevToolsTargetHandle {
        let pause = RendererInspectorPauseBridge::default();
        let main = RendererInspectorMainIngress::new(
            RendererInspectorSessionExecutorRouteId::new(1),
            pause.pause_loop_wake(),
        );
        let io = RendererInspectorIoIngress::new(pause.pause_loop_wake(), None);
        RendererDevToolsTargetHandle::new(pause, main, io)
    }

    fn assert_debugging_enabled(target: &RendererDevToolsTargetHandle) {
        assert_eq!(target.pause_ref().wait_for_pause_work(|| Some(42)), Ok(42));
    }

    fn assert_navigation_exit(target: &RendererDevToolsTargetHandle) {
        assert_eq!(
            target.pause_ref().wait_for_pause_work(|| Some(42)),
            Err(RendererInspectorPauseExitReason::Navigation)
        );
    }

    #[test]
    fn navigation_waits_for_response_and_duplicate_notifications_are_idempotent() {
        let target = target();
        assert_debugging_enabled(&target);
        target.navigation_started(1);
        assert_debugging_enabled(&target);
        target.navigation_response_ready(1);
        assert_navigation_exit(&target);
        target.navigation_response_ready(1);
        target.navigation_started(1);
        assert_navigation_exit(&target);
        target.navigation_finished(1);
        target.navigation_finished(1);
        assert_debugging_enabled(&target);
    }

    #[test]
    fn canceled_load_job_cannot_reactivate_pauses_even_with_a_retained_handle() {
        for response_ready in [false, true] {
            let target = target();
            let job = target.clone();
            target.navigation_started(1);
            if response_ready {
                job.navigation_response_ready(1);
            }
            target.navigation_finished(1);
            assert_debugging_enabled(&job);
            job.navigation_response_ready(1);
            assert_debugging_enabled(&target);
        }
    }

    #[test]
    fn superseded_response_and_completion_cannot_change_the_current_navigation() {
        let target = target();
        target.navigation_started(1);
        target.navigation_response_ready(1);
        target.navigation_started(2);
        target.navigation_response_ready(1);
        assert_debugging_enabled(&target);
        target.navigation_response_ready(2);
        target.navigation_finished(1);
        assert_navigation_exit(&target);
        target.navigation_finished(2);
        assert_debugging_enabled(&target);
    }

    #[test]
    fn navigation_policy_is_document_local_and_handle_drop_does_not_finish_it() {
        let first = target();
        let second = target();
        first.navigation_started(1);
        first.navigation_response_ready(1);
        drop(first.clone());
        assert_navigation_exit(&first);
        assert_debugging_enabled(&second);
        second.navigation_finished(1);
        assert_navigation_exit(&first);
    }

    #[test]
    fn pause_exit_reason_is_preserved_after_navigation_completion() {
        let target = target();
        target.navigation_started(1);
        target.navigation_response_ready(1);
        let reason = target.pause_ref().wait_for_pause_work(|| Some(42));
        target.navigation_finished(1);
        assert_eq!(reason, Err(RendererInspectorPauseExitReason::Navigation));
        assert_debugging_enabled(&target);
    }

    #[test]
    fn closing_target_takes_precedence_and_late_notifications_cannot_reopen_it() {
        let target = target();
        target.navigation_started(1);
        target.navigation_response_ready(1);
        target.pause_ref().close_target();
        target.navigation_finished(1);
        target.navigation_started(2);
        target.navigation_response_ready(2);
        assert_eq!(
            target.pause_ref().wait_for_pause_work(|| Some(42)),
            Err(RendererInspectorPauseExitReason::TargetClosed)
        );
    }

    #[test]
    fn response_ready_wakes_the_pause_loop_and_covers_later_pauses() {
        let target = target();
        target.navigation_started(1);
        let pause = target.pause();
        let (waiting_tx, waiting_rx) = std::sync::mpsc::channel();
        let (exited_tx, exited_rx) = std::sync::mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let mut waiting_tx = Some(waiting_tx);
            exited_tx
                .send(pause.wait_for_pause_work(|| {
                    if let Some(waiting_tx) = waiting_tx.take() {
                        waiting_tx.send(()).unwrap();
                    }
                    None::<()>
                }))
                .unwrap();
        });
        let waiting = waiting_rx.recv_timeout(std::time::Duration::from_secs(30));
        target.navigation_response_ready(1);
        let exited = exited_rx.recv_timeout(std::time::Duration::from_secs(30));
        if exited.is_err() {
            target.pause_ref().close_target();
        }
        waiter.join().unwrap();
        waiting.expect("waiter must enter the pause wait before response readiness");
        assert_eq!(
            exited.expect("response readiness must wake the pause loop"),
            Err(RendererInspectorPauseExitReason::Navigation)
        );
        assert_navigation_exit(&target);
        target.navigation_finished(1);
        assert_debugging_enabled(&target);
    }
}
