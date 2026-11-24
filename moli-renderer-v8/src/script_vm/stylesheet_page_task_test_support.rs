//! Body-only stylesheet support for standalone `ScriptVm` domain fixtures.
//!
//! This module may claim concrete payloads from production typed sources, but
//! it never owns selected-task completion. Tests for checkpoint, child sync,
//! runtime follow-up, replacement, or complete HTML-task ordering belong in
//! the `PageVm` exact selected-task harness.

use super::ScriptVm;
use crate::page_task_queue::{
    RendererPageDomManipulationOwner, RendererPageDomManipulationTask, RendererPageNetworkingOwner,
    RendererPageNetworkingTask, RendererPageReadyDescriptor, RendererPageSchedulerTask,
};

impl ScriptVm {
    /// Prime and apply every immediately ready stylesheet lifecycle body.
    ///
    /// This convenience remains body-only for standalone CSP, focus, and
    /// lifecycle-domain fixtures. It does not emulate the selected Page-task
    /// completion loop.
    pub(super) fn apply_connected_style_lifecycle_bodies_for_test(&mut self) -> bool {
        self.prime_document_lifecycle_and_publish_stylesheets();
        while self.apply_next_stylesheet_networking_body_for_test() {}
        let mut dispatched_any = false;
        while self.apply_next_connected_style_event_body_for_test() {
            dispatched_any = true;
        }
        dispatched_any
    }

    /// Apply one stylesheet Networking body without completing an HTML task.
    ///
    /// Standalone `ScriptVm` domain fixtures have no `PageVm` selected-task
    /// dispatcher. This support hook is intentionally limited to the
    /// stylesheet terminal body: it may install the fetched source and publish
    /// a later element event, but it never performs a task-end checkpoint,
    /// child synchronization, or runtime follow-up. Tests that claim complete
    /// task semantics must use the `PageVm` exact selected-task harness.
    pub(crate) fn apply_next_stylesheet_networking_body_for_test(&mut self) -> bool {
        let residence = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .expect("stylesheet fixture must retain its production Page source");
        let source = residence.task_sources();
        let root_document = residence.root_document();
        let Some(task) = source.take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                RendererPageReadyDescriptor::Networking {
                    owner: RendererPageNetworkingOwner::StylesheetCompletion(_),
                    ..
                }
            )
        }) else {
            return false;
        };
        let RendererPageSchedulerTask::Networking(
            RendererPageNetworkingTask::StylesheetCompletion(task),
        ) = task
        else {
            unreachable!("stylesheet Networking descriptor must dequeue its own task")
        };
        let _ = self.apply_page_stylesheet_networking_task(root_document, task);
        true
    }

    /// Wait for and apply one stylesheet Networking body.
    ///
    /// The stable production source supplies the readiness signal. This avoids
    /// polling or `yield_now()` in asynchronous domain fixtures while retaining
    /// the same body-only contract as
    /// [`Self::apply_next_stylesheet_networking_body_for_test`].
    pub(crate) async fn wait_for_and_apply_stylesheet_networking_body_for_test(&mut self) -> bool {
        if self.apply_next_stylesheet_networking_body_for_test() {
            return true;
        }
        self.wait_for_page_task_executor_work_arrival_for_test()
            .await
            && self.apply_next_stylesheet_networking_body_for_test()
    }

    /// Take one connected-style body payload from its production typed source.
    ///
    /// Returning the domain payload is only for exact lease/lifecycle tests.
    /// The caller must not describe this as a selected Page task: no
    /// completion authority accompanies the payload.
    pub(crate) fn take_next_connected_style_event_body_for_test(
        &mut self,
    ) -> Option<crate::page_task_queue::RendererPageConnectedStyleEventTask> {
        let residence = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .expect("stylesheet fixture must retain its production Page source");
        let source = residence.task_sources();
        let style_task = source.take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                RendererPageReadyDescriptor::Networking {
                    owner: RendererPageNetworkingOwner::StyleElementEvent(_),
                    ..
                }
            )
        });
        if let Some(task) = style_task {
            let RendererPageSchedulerTask::Networking(
                RendererPageNetworkingTask::StyleElementEvent(task),
            ) = task
            else {
                unreachable!("style-element descriptor must dequeue its Networking task")
            };
            return Some(task);
        }

        let link_task = source.take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                RendererPageReadyDescriptor::DomManipulation {
                    owner: RendererPageDomManipulationOwner::ConnectedStyleEvent(_),
                    ..
                }
            )
        })?;
        let RendererPageSchedulerTask::DomManipulation(
            RendererPageDomManipulationTask::ConnectedStyleEvent(task),
        ) = link_task
        else {
            unreachable!("link-element descriptor must dequeue its DOM-manipulation task")
        };
        Some(task)
    }

    /// Apply one connected-style event body without completing an HTML task.
    ///
    /// This is a domain-fixture support hook, not a parallel task executor.
    /// It deliberately stops before microtask checkpoint, child
    /// synchronization, and runtime follow-up. Full behavior coverage lives in
    /// `runtime::page_vm::tests::stylesheet_task` and enters the production
    /// selected-task dispatcher.
    pub(crate) fn apply_next_connected_style_event_body_for_test(&mut self) -> bool {
        let root_document = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .expect("stylesheet fixture must retain its production Page source")
            .root_document();
        let Some(task) = self.take_next_connected_style_event_body_for_test() else {
            return false;
        };
        let _ = self.apply_page_connected_style_event_task_body(root_document, task);
        true
    }
}
