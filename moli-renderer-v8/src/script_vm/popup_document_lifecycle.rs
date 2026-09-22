use anyhow::Result;

use super::ScriptVm;
use crate::{
    host::PopupWindowEventTarget,
    native_bridge::{LightweightPopupNavigationTaskToken, PopupDocumentParserCompletion},
    page_task_queue::{PopupDocumentLifecycleEvent, RendererPagePopupDocumentLifecycleOwner},
    runtime::AuthorizedCurrentPagePopupDocumentLifecycle,
    style_engine::StyleInvalidationTurnExitBoundary,
};

/// One continuation of the already-selected popup lifecycle task. PageVm must
/// finish each checkpoint before resuming; the individual event helpers never
/// execute microtasks or start a second HTML task.
#[must_use]
pub(crate) enum PopupDocumentLifecycleStep {
    Completed,
    Checkpoint(PopupDocumentLifecycleCheckpoint),
}

pub(crate) struct PopupDocumentLifecycleCheckpoint {
    window: PopupWindowEventTarget,
    task: LightweightPopupNavigationTaskToken,
    continuation: PopupDocumentLifecycleContinuation,
}

enum PopupDocumentLifecycleContinuation {
    FinishInteractive(PopupDocumentParserCompletion),
    FinishDomContentLoaded,
    ContinueLoad,
    ContinuePageshow,
    FinishLoad,
}

impl ScriptVm {
    pub(crate) fn current_popup_document_lifecycle_owner(
        &self,
        expected: LightweightPopupNavigationTaskToken,
        root_document: crate::runtime::RendererDocumentToken,
        event: PopupDocumentLifecycleEvent,
    ) -> Option<RendererPagePopupDocumentLifecycleOwner> {
        let host = self._context_host.borrow();
        let task = match event {
            PopupDocumentLifecycleEvent::DomContentLoaded => {
                host.current_lightweight_popup_dom_content_loaded_task(expected)
            }
            PopupDocumentLifecycleEvent::Load => {
                host.current_lightweight_popup_load_event_task(expected)
            }
        };
        task.map(|target| {
            RendererPagePopupDocumentLifecycleOwner::new(root_document, target, event)
        })
    }

    /// Begin the exact admitted popup lifecycle phase. DOMContentLoaded waits
    /// for parser completion; load additionally waits for child-frame blockers.
    pub(crate) fn begin_current_popup_document_lifecycle_body(
        &mut self,
        authorization: AuthorizedCurrentPagePopupDocumentLifecycle,
    ) -> Result<PopupDocumentLifecycleStep> {
        let task = authorization.into_task();
        let target = task.owner().target();
        let event = task.owner().event();
        let window = {
            let mut host = self._context_host.borrow_mut();
            if event == PopupDocumentLifecycleEvent::Load {
                host.take_current_lightweight_popup_load_event_task(target);
            }
            host.current_popup_window_event_target(target.popup_id())
                .expect("authorized popup lifecycle must retain its exact Window")
        };
        self.with_default_context_scope(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            match event {
                PopupDocumentLifecycleEvent::DomContentLoaded => {
                    host.dispatch_lightweight_popup_dom_content_loaded(scope, target)
                }
                PopupDocumentLifecycleEvent::Load => host
                    .dispatch_lightweight_popup_document_readiness(
                        scope,
                        window,
                        crate::dom::native::DocumentReadyState::Complete,
                    ),
            }
            Ok(())
        })?;
        Ok(PopupDocumentLifecycleStep::Checkpoint(
            PopupDocumentLifecycleCheckpoint {
                window,
                task: target,
                continuation: match event {
                    PopupDocumentLifecycleEvent::DomContentLoaded => {
                        PopupDocumentLifecycleContinuation::FinishDomContentLoaded
                    }
                    PopupDocumentLifecycleEvent::Load => {
                        PopupDocumentLifecycleContinuation::ContinueLoad
                    }
                },
            },
        ))
    }

    pub(crate) fn begin_popup_document_interactive(
        &mut self,
        completion: PopupDocumentParserCompletion,
    ) -> Result<PopupDocumentLifecycleStep> {
        let entered = self.with_default_context_scope(|scope, host_ptr| {
            Ok(unsafe { &mut *host_ptr }.begin_lightweight_popup_interactive(scope, completion))
        })?;
        Ok(if entered {
            PopupDocumentLifecycleStep::Checkpoint(PopupDocumentLifecycleCheckpoint {
                window: completion.window,
                task: completion.task,
                continuation: PopupDocumentLifecycleContinuation::FinishInteractive(completion),
            })
        } else {
            PopupDocumentLifecycleStep::Completed
        })
    }

    pub(crate) fn take_completed_popup_javascript_url_parsers(
        &mut self,
    ) -> Vec<PopupDocumentParserCompletion> {
        self._context_host
            .borrow_mut()
            .take_completed_popup_javascript_url_parsers()
    }

    pub(crate) fn resume_popup_document_lifecycle_after_checkpoint(
        &mut self,
        checkpoint: PopupDocumentLifecycleCheckpoint,
    ) -> Result<PopupDocumentLifecycleStep> {
        let window = checkpoint.window;
        // The shell can survive Document replacement, including replacement
        // triggered by a Promise reaction. The concrete Window identity must
        // still match before delivering a successor or marking load complete.
        if !self
            ._context_host
            .borrow()
            .popup_window_event_target_is_current(window)
            || !self
                ._context_host
                .borrow()
                .lightweight_popup_committed_navigation_task_is_current(checkpoint.task)
        {
            return Ok(PopupDocumentLifecycleStep::Completed);
        }
        match checkpoint.continuation {
            PopupDocumentLifecycleContinuation::FinishInteractive(completion) => {
                self._context_host
                    .borrow_mut()
                    .finish_lightweight_popup_interactive(completion);
                Ok(PopupDocumentLifecycleStep::Completed)
            }
            PopupDocumentLifecycleContinuation::FinishDomContentLoaded => {
                self._context_host
                    .borrow_mut()
                    .finish_lightweight_popup_dom_content_loaded(window);
                Ok(PopupDocumentLifecycleStep::Completed)
            }
            PopupDocumentLifecycleContinuation::ContinueLoad => {
                let dispatched = self.with_default_context_scope(|scope, host_ptr| {
                    Ok(
                        unsafe { &mut *host_ptr }.dispatch_lightweight_popup_load_event(
                            scope,
                            window.document_owner.popup_id(),
                        ),
                    )
                })?;
                Ok(PopupDocumentLifecycleStep::Checkpoint(
                    PopupDocumentLifecycleCheckpoint {
                        window,
                        task: checkpoint.task,
                        continuation: if dispatched {
                            PopupDocumentLifecycleContinuation::ContinuePageshow
                        } else {
                            PopupDocumentLifecycleContinuation::FinishLoad
                        },
                    },
                ))
            }
            PopupDocumentLifecycleContinuation::ContinuePageshow => {
                self.with_default_context_scope(|scope, host_ptr| {
                    unsafe { &mut *host_ptr }.dispatch_lightweight_popup_pageshow_event(
                        scope,
                        window.document_owner.popup_id(),
                    );
                    Ok(())
                })?;
                Ok(PopupDocumentLifecycleStep::Checkpoint(
                    PopupDocumentLifecycleCheckpoint {
                        window,
                        task: checkpoint.task,
                        continuation: PopupDocumentLifecycleContinuation::FinishLoad,
                    },
                ))
            }
            PopupDocumentLifecycleContinuation::FinishLoad => {
                self._context_host
                    .borrow_mut()
                    .note_lightweight_popup_load_event_finished(window.document_owner);
                Ok(PopupDocumentLifecycleStep::Completed)
            }
        }
    }

    pub(crate) fn finish_popup_document_lifecycle_checkpoint(&mut self) -> Result<()> {
        self.perform_owner_lane_task_microtask_checkpoints()?;
        self.sync_child_browsing_context_records();
        Ok(())
    }

    pub(crate) fn finish_popup_document_lifecycle_turn<T>(&mut self, result: T) -> T {
        self.finish_runtime_turn_with_style_drain(
            StyleInvalidationTurnExitBoundary::NonScriptPageTask,
            result,
        )
    }

    pub(crate) fn discard_stale_popup_document_lifecycle_task(
        &mut self,
        stale: LightweightPopupNavigationTaskToken,
    ) {
        self._context_host
            .borrow_mut()
            .discard_stale_lightweight_popup_load_event_task(stale);
    }
}
