use moli_protocol::{
    CompletedDevToolsNavigationCommandDispatch, DevToolsNavigationCommandTaskStep,
    PendingDevToolsNavigationCommandDispatch,
    devtools_runtime::{DevToolsCommand, DevToolsCommandContext, DevToolsNavigationWait},
};

use super::{
    CdpScheduler, CdpSchedulerEventReceivers, DevToolsCommandExecution, ProtocolOutputSequence,
    devtools_navigation_lifecycle_milestone, devtools_navigation_wait,
};
use futures_util::StreamExt;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// Moves from the socket to its scheduler on detach without cancelling work.
pub(crate) struct DevToolsNavigationCommandWait(
    tokio::task::JoinHandle<CompletedDevToolsNavigationExecution>,
);

impl DevToolsNavigationCommandWait {
    pub(crate) fn new(pending: PendingDevToolsNavigationExecution) -> Self {
        Self(tokio::task::spawn_local(pending.wait()))
    }
}

impl Future for DevToolsNavigationCommandWait {
    type Output = Result<CompletedDevToolsNavigationExecution, tokio::task::JoinError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(context)
    }
}

impl Drop for DevToolsNavigationCommandWait {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) enum AdapterOwnerInput {
    Browser(super::browser_events::BrowserEventInput),
    Navigation(Result<Box<CompletedDevToolsNavigationExecution>, tokio::task::JoinError>),
}

struct NavigationCommandState {
    context: DevToolsCommandContext,
    wait: Option<DevToolsNavigationWait>,
    validate_root_document_lifecycle: bool,
    output: ProtocolOutputSequence,
}

pub(crate) struct PendingDevToolsNavigationExecution {
    state: NavigationCommandState,
    pending: PendingDevToolsNavigationCommandDispatch,
}

pub(crate) struct CompletedDevToolsNavigationExecution {
    state: NavigationCommandState,
    completed: CompletedDevToolsNavigationCommandDispatch,
}

pub(crate) enum DevToolsNavigationCommandProgress {
    Complete(Box<DevToolsCommandExecution>),
    Pending(Box<PendingDevToolsNavigationExecution>),
    PendingLifecycle {
        pending: Box<PendingDevToolsNavigationLifecycle>,
        protocol_output: ProtocolOutputSequence,
    },
}

pub(crate) struct PendingDevToolsNavigationLifecycle {
    context: DevToolsCommandContext,
    key: moli_protocol::DevToolsDocumentLifecycleWaitKey,
    result: Result<
        moli_protocol::devtools_runtime::DevToolsCommandResult,
        moli_protocol::devtools_runtime::DevToolsError,
    >,
}

pub(crate) struct DevToolsNavigationReplyWait {
    context: DevToolsCommandContext,
    loader_id: String,
    milestone: moli_core::page::RendererDocumentLifecycleMilestone,
    key: Option<moli_protocol::DevToolsDocumentLifecycleWaitKey>,
    lifecycle_projected: bool,
}

impl DevToolsNavigationReplyWait {
    pub(crate) fn observe_lifecycle_event(
        &mut self,
        event: &moli_protocol::devtools_runtime::AutomationEvent,
    ) {
        use moli_core::page::RendererDocumentLifecycleMilestone;
        use moli_protocol::devtools_runtime::AutomationEvent;
        let event = match (self.milestone, event) {
            (RendererDocumentLifecycleMilestone::Load, AutomationEvent::Load(event))
            | (
                RendererDocumentLifecycleMilestone::DomContentLoaded,
                AutomationEvent::DomContentLoaded(event),
            ) => event,
            _ => return,
        };
        self.lifecycle_projected |= self.context.target_id.as_ref() == Some(&event.target_id)
            && event
                .loader_id
                .as_ref()
                .is_some_and(|loader| loader.as_str() == self.loader_id);
    }
}

impl PendingDevToolsNavigationExecution {
    pub(crate) async fn wait(self) -> CompletedDevToolsNavigationExecution {
        CompletedDevToolsNavigationExecution {
            state: self.state,
            completed: self.pending.wait().await,
        }
    }
}

impl CdpScheduler {
    pub(crate) fn advance_devtools_navigation_lifecycle(
        &mut self,
        pending: Box<PendingDevToolsNavigationLifecycle>,
        protocol_output: ProtocolOutputSequence,
    ) -> DevToolsNavigationCommandProgress {
        use moli_protocol::DevToolsDocumentLifecycleWaitState;
        let state = self
            .conn
            .devtools_document_lifecycle_wait_state(&pending.context, &pending.key);
        if state == DevToolsDocumentLifecycleWaitState::Pending
            || (state == DevToolsDocumentLifecycleWaitState::Reached
                && (!self
                    .conn
                    .devtools_document_lifecycle_wait_is_visible(&pending.context, &pending.key)
                    || (pending.key.milestone()
                        == moli_core::page::RendererDocumentLifecycleMilestone::Load
                        && self.has_deferred_main_document_load_completion_for_devtools_context(
                            &pending.context,
                        ))))
        {
            return DevToolsNavigationCommandProgress::PendingLifecycle {
                pending,
                protocol_output,
            };
        }
        let result = super::devtools_document_lifecycle_wait_error(state, pending.key.milestone())
            .map_or(pending.result, Err);
        self.conn
            .release_devtools_document_lifecycle_wait_key(&pending.context, &pending.key);
        DevToolsNavigationCommandProgress::Complete(Box::new(DevToolsCommandExecution {
            result,
            protocol_output,
        }))
    }

    pub(crate) fn cancel_devtools_navigation_lifecycle(
        &mut self,
        pending: PendingDevToolsNavigationLifecycle,
    ) {
        self.conn
            .release_devtools_document_lifecycle_wait_key(&pending.context, &pending.key);
    }

    pub(crate) fn navigation_reply_wait(
        &mut self,
        target_id: &str,
        navigation_id: Option<&str>,
        wait: DevToolsNavigationWait,
    ) -> Option<DevToolsNavigationReplyWait> {
        let milestone = devtools_navigation_lifecycle_milestone(Some(wait))?;
        let loader_id = navigation_id?.strip_prefix("navigation-")?.to_owned();
        let context = DevToolsCommandContext {
            protocol: moli_protocol::devtools_runtime::DevToolsProtocol::WebDriverBidi,
            session_id: None,
            target_id: Some(target_id.into()),
            browser_context_id: None,
        };
        // Child-frame navigation already owns its lifecycle wait. A missing
        // target, however, must settle as unavailable rather than succeed.
        if !self
            .conn
            .devtools_context_routes_to_top_level_target(&context)
            && self
                .conn
                .devtools_context_document_navigation_state(&context)
                != moli_protocol::DevToolsDocumentNavigationState::Unavailable
        {
            return None;
        }
        Some(DevToolsNavigationReplyWait {
            context,
            loader_id,
            milestone,
            key: None,
            lifecycle_projected: false,
        })
    }

    pub(crate) fn poll_navigation_reply_wait(
        &mut self,
        wait: &mut DevToolsNavigationReplyWait,
    ) -> Option<Result<(), moli_protocol::devtools_runtime::DevToolsError>> {
        use moli_protocol::{
            DevToolsDocumentLifecycleWaitState as Lifecycle,
            DevToolsDocumentNavigationState as Navigation,
        };
        if wait.key.is_none() {
            wait.key = self.conn.capture_devtools_document_lifecycle_wait_key(
                &wait.context,
                &wait.loader_id,
                wait.milestone,
            );
        }
        let state = if let Some(key) = wait.key.as_ref() {
            self.conn
                .devtools_document_lifecycle_wait_state(&wait.context, key)
        } else {
            match self
                .conn
                .devtools_context_document_navigation_state(&wait.context)
            {
                Navigation::Unavailable => Lifecycle::Unavailable,
                Navigation::Committed { loader_id } if loader_id != wait.loader_id => {
                    Lifecycle::Superseded
                }
                _ => Lifecycle::Pending,
            }
        };
        // Native state can advance before its renderer publication reaches
        // the protocol owner. Keep the reply behind that exact occurrence.
        if state == Lifecycle::Pending || (state == Lifecycle::Reached && !wait.lifecycle_projected)
        {
            return None;
        }
        self.cancel_navigation_reply_wait(wait);
        Some(
            super::devtools_document_lifecycle_wait_error(state, wait.milestone)
                .map_or(Ok(()), Err),
        )
    }

    pub(crate) fn cancel_navigation_reply_wait(&mut self, wait: &mut DevToolsNavigationReplyWait) {
        if let Some(key) = wait.key.take() {
            self.conn
                .release_devtools_document_lifecycle_wait_key(&wait.context, &key);
        }
    }

    pub(crate) fn retain_detached_navigation(&mut self, wait: DevToolsNavigationCommandWait) {
        self.detached_navigations.push(wait);
    }

    pub(crate) async fn recv_adapter_owner_input(&mut self) -> AdapterOwnerInput {
        tokio::select! {
            biased;
            event = super::browser_events::recv_browser_event(&mut self.browser_event_rx) => AdapterOwnerInput::Browser(event),
            completed = self.detached_navigations.next(), if !self.detached_navigations.is_empty() => {
                AdapterOwnerInput::Navigation(completed.expect("nonempty navigation wait set").map(Box::new))
            }
        }
    }

    pub(crate) async fn complete_adapter_owner_input(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        input: AdapterOwnerInput,
    ) -> ProtocolOutputSequence {
        match input {
            AdapterOwnerInput::Browser(event) => self.handle_browser_event(event).await,
            AdapterOwnerInput::Navigation(completed) => {
                self.complete_detached_navigation(receivers, completed)
                    .await
            }
        }
    }

    pub(super) async fn complete_detached_navigation(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        completed: Result<Box<CompletedDevToolsNavigationExecution>, tokio::task::JoinError>,
    ) -> ProtocolOutputSequence {
        let completed = match completed {
            Ok(completed) => completed,
            Err(error) => {
                tracing::warn!(?error, "detached DevTools navigation waiter failed");
                return ProtocolOutputSequence::empty();
            }
        };
        match self
            .complete_devtools_navigation_command(receivers, *completed)
            .await
        {
            DevToolsNavigationCommandProgress::Complete(execution) => execution.protocol_output,
            DevToolsNavigationCommandProgress::Pending(pending) => {
                self.retain_detached_navigation(DevToolsNavigationCommandWait::new(*pending));
                ProtocolOutputSequence::empty()
            }
            DevToolsNavigationCommandProgress::PendingLifecycle {
                pending,
                protocol_output,
            } => {
                // The Browser commit is complete. Detach releases only this
                // frontend's final milestone registration, not native work.
                self.cancel_devtools_navigation_lifecycle(*pending);
                protocol_output
            }
        }
    }

    pub(crate) async fn start_devtools_navigation_command(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        command: DevToolsCommand,
        background_command_id: Option<u64>,
    ) -> DevToolsNavigationCommandProgress {
        let context = command.context().clone();
        let wait = devtools_navigation_wait(&command);
        let state = NavigationCommandState {
            validate_root_document_lifecycle: devtools_navigation_lifecycle_milestone(wait)
                .is_some()
                && self
                    .conn
                    .devtools_context_routes_to_top_level_target(&context),
            context,
            wait,
            output: self.drain_browser_events().await,
        };
        let step = self
            .conn
            .start_devtools_navigation_command_dispatch(command, background_command_id)
            .await;
        self.advance_devtools_navigation_command(receivers, state, step)
            .await
    }

    pub(crate) async fn complete_devtools_navigation_command(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        completed: CompletedDevToolsNavigationExecution,
    ) -> DevToolsNavigationCommandProgress {
        let step = self
            .conn
            .complete_devtools_navigation_command_dispatch(completed.completed)
            .await;
        self.advance_devtools_navigation_command(receivers, completed.state, step)
            .await
    }

    async fn advance_devtools_navigation_command(
        &mut self,
        receivers: &mut CdpSchedulerEventReceivers,
        mut state: NavigationCommandState,
        step: DevToolsNavigationCommandTaskStep,
    ) -> DevToolsNavigationCommandProgress {
        match step {
            DevToolsNavigationCommandTaskStep::Pending(mut pending) => {
                self.apply_scheduler_events(pending.take_scheduler_events());
                DevToolsNavigationCommandProgress::Pending(Box::new(
                    PendingDevToolsNavigationExecution {
                        state,
                        pending: *pending,
                    },
                ))
            }
            DevToolsNavigationCommandTaskStep::Complete(outcome) => {
                let (mut result, scheduler_events, protocol_events, predecessor) =
                    outcome.into_complete_parts();
                self.apply_scheduler_events(scheduler_events);
                if let Some(predecessor) = predecessor {
                    match self
                        .project_renderer_output_predecessor_before_devtools_result(
                            receivers,
                            &predecessor,
                        )
                        .await
                    {
                        Ok(output) => state.output.append(output),
                        Err(failure) => {
                            let (output, error) = failure.into_parts();
                            state.output.append(output);
                            result = Err(error);
                        }
                    }
                }
                state.output.append(
                    self.route_background_events_around_inflight_navigation(protocol_events),
                );
                state.output.append(
                    self.complete_ready_protocol_residences_after_command()
                        .await,
                );
                if result.is_ok()
                    && state.validate_root_document_lifecycle
                    && let Some((loader_id, milestone)) =
                        super::devtools_navigation_result_loader_id(&result)
                            .zip(devtools_navigation_lifecycle_milestone(state.wait))
                    && let Some(key) = self.conn.capture_devtools_document_lifecycle_wait_key(
                        &state.context,
                        &loader_id,
                        milestone,
                    )
                {
                    return self.advance_devtools_navigation_lifecycle(
                        Box::new(PendingDevToolsNavigationLifecycle {
                            context: state.context,
                            key,
                            result,
                        }),
                        state.output,
                    );
                }
                DevToolsNavigationCommandProgress::Complete(Box::new(DevToolsCommandExecution {
                    result,
                    protocol_output: state.output,
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_protocol::devtools_runtime::{
        AutomationEvent, DevToolsProtocol, NavigationLifecycleEvent,
    };

    #[test]
    fn navigation_reply_wait_requires_its_exact_projected_lifecycle() {
        let mut wait = DevToolsNavigationReplyWait {
            context: DevToolsCommandContext {
                protocol: DevToolsProtocol::WebDriverBidi,
                session_id: None,
                target_id: Some("target".into()),
                browser_context_id: None,
            },
            loader_id: "loader".to_owned(),
            milestone: moli_core::page::RendererDocumentLifecycleMilestone::Load,
            key: None,
            lifecycle_projected: false,
        };
        let event = |target: &str, loader: &str| NavigationLifecycleEvent {
            target_id: target.into(),
            frame_id: target.into(),
            navigation_id: None,
            loader_id: Some(loader.into()),
            url: "https://example.test/".to_owned(),
            timestamp: 1.0,
        };
        for unrelated in [
            AutomationEvent::Load(event("peer", "loader")),
            AutomationEvent::Load(event("target", "old-loader")),
            AutomationEvent::DomContentLoaded(event("target", "loader")),
        ] {
            wait.observe_lifecycle_event(&unrelated);
            assert!(!wait.lifecycle_projected);
        }
        wait.observe_lifecycle_event(&AutomationEvent::Load(event("target", "loader")));
        assert!(wait.lifecycle_projected);
    }
}
