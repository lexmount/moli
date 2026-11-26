#[cfg(feature = "devtools")]
use crate::runtime::RendererDevToolsPageCommand;
#[cfg(feature = "molio")]
use crate::runtime::{RendererPageToken, RendererViewportWheelInput};
use anyhow::{Result, anyhow};
use parking_lot::Mutex;
use std::sync::{Arc, Weak};
use std::thread::{self, JoinHandle};
use tokio::runtime::LocalOptions;
use tokio::sync::{mpsc, oneshot};

use super::devtools::ingress::io::RendererInspectorIoOwnerWake;
#[cfg(feature = "devtools")]
use super::devtools::ingress::main::RendererInspectorMainOwnerWake;
use super::page_task_queue::RendererOwnerWake;
#[cfg(feature = "devtools")]
use super::runtime::RendererPageCommand;
use super::runtime::{RendererOwnerCommand, RendererOwnerHandle, RendererOwnerReply};
use super::service_worker_runtime::ServiceWorkerRuntimeOwnerWake;
use super::shared_worker_runtime::SharedWorkerRuntimeOwnerWake;

// The renderer runs bounded recursive layout on top of async and V8 frames.
// Reserve stack space for the maximum admitted layout depth.
pub(crate) const RENDER_RUNTIME_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;

#[cfg(feature = "molio")]
type PendingViewportWheel = Mutex<Option<RendererViewportWheelInput>>;

pub(crate) enum RenderRuntimeEnvelope {
    Command {
        command: Box<RendererOwnerCommand>,
        reply_tx: oneshot::Sender<Result<RendererOwnerReply>>,
    },
    #[cfg(feature = "molio")]
    ViewportWheel {
        token: RendererPageToken,
        pending: Arc<PendingViewportWheel>,
    },
    #[cfg(feature = "devtools")]
    InspectorMainReceiverWake(RendererInspectorMainOwnerWake),
}

#[derive(Clone)]
pub(crate) struct RenderRuntimeHandle {
    state: Weak<RenderRuntimeState>,
}

struct RenderRuntimeState {
    admission: Mutex<RenderRuntimeAdmission>,
}

struct RenderRuntimeAdmission {
    tx: mpsc::UnboundedSender<RenderRuntimeEnvelope>,
    terminal: bool,
    #[cfg(feature = "molio")]
    viewport_wheel_tail: Option<(RendererPageToken, Weak<PendingViewportWheel>)>,
}

impl RenderRuntimeAdmission {
    fn new(tx: mpsc::UnboundedSender<RenderRuntimeEnvelope>) -> Self {
        Self {
            tx,
            terminal: false,
            #[cfg(feature = "molio")]
            viewport_wheel_tail: None,
        }
    }

    fn send_ordered(
        &mut self,
        envelope: RenderRuntimeEnvelope,
    ) -> std::result::Result<(), mpsc::error::SendError<RenderRuntimeEnvelope>> {
        #[cfg(feature = "molio")]
        {
            // Merging must use the same order as ordinary command admission.
            self.viewport_wheel_tail = None;
        }
        self.tx.send(envelope)
    }
}

pub(crate) struct RenderRuntimeEnqueueError {
    command: Box<RendererOwnerCommand>,
    error: anyhow::Error,
}

impl RenderRuntimeEnqueueError {
    pub(crate) fn into_parts(self) -> (RendererOwnerCommand, anyhow::Error) {
        (*self.command, self.error)
    }

    fn into_error(self) -> anyhow::Error {
        self.error
    }
}

impl std::fmt::Debug for RenderRuntimeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderRuntimeHandle")
            .finish_non_exhaustive()
    }
}

pub(crate) struct RenderRuntimeOwner {
    state: Option<Arc<RenderRuntimeState>>,
    render_join: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for RenderRuntimeOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderRuntimeOwner")
            .field("has_state", &self.state.is_some())
            .field("has_render_join", &self.render_join.is_some())
            .finish()
    }
}

impl Drop for RenderRuntimeOwner {
    fn drop(&mut self) {
        drop(self.state.take());
        let _ = join_thread(&mut self.render_join);
    }
}

fn join_thread<T>(handle: &mut Option<JoinHandle<T>>) -> Option<thread::Result<T>> {
    if let Some(handle) = handle.take()
        && handle.thread().id() != thread::current().id()
    {
        return Some(handle.join());
    }
    None
}

impl RenderRuntimeOwner {
    pub(crate) fn spawn(
        owner: RendererOwnerHandle,
        page_wake_rx: mpsc::UnboundedReceiver<RendererOwnerWake>,
        inspector_io_wake_rx: mpsc::UnboundedReceiver<RendererInspectorIoOwnerWake>,
        shared_worker_wake_rx: mpsc::UnboundedReceiver<SharedWorkerRuntimeOwnerWake>,
        service_worker_wake_rx: mpsc::UnboundedReceiver<ServiceWorkerRuntimeOwnerWake>,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel::<RenderRuntimeEnvelope>();
        let render_join = std::thread::Builder::new()
            .name("render_runtime".to_owned())
            .stack_size(RENDER_RUNTIME_STACK_SIZE_BYTES)
            .spawn(move || {
                let mut runtime_builder = tokio::runtime::Builder::new_current_thread();
                runtime_builder
                    .max_blocking_threads(
                        crate::tokio_blocking_budget::tokio_blocking_thread_budget(),
                    )
                    .enable_all();
                let runtime = runtime_builder
                    .build_local(LocalOptions::default())
                    .expect("failed to build render runtime");
                runtime.block_on(render_runtime_main_loop(
                    owner,
                    rx,
                    page_wake_rx,
                    inspector_io_wake_rx,
                    shared_worker_wake_rx,
                    service_worker_wake_rx,
                ));
            })
            .expect("failed to spawn render runtime thread");
        Self {
            state: Some(Arc::new(RenderRuntimeState {
                admission: Mutex::new(RenderRuntimeAdmission::new(tx)),
            })),
            render_join: Some(render_join),
        }
    }

    pub(crate) fn handle(&self) -> RenderRuntimeHandle {
        RenderRuntimeHandle {
            state: self.state.as_ref().map(Arc::downgrade).unwrap_or_default(),
        }
    }
}

impl RenderRuntimeHandle {
    pub(crate) fn disconnected() -> Self {
        Self { state: Weak::new() }
    }

    fn enqueue_command(
        &self,
        command: RendererOwnerCommand,
    ) -> std::result::Result<oneshot::Receiver<Result<RendererOwnerReply>>, RenderRuntimeEnqueueError>
    {
        #[cfg(feature = "devtools")]
        if matches!(
            &command,
            RendererOwnerCommand::RunAsyncPageCommand {
                command: RendererPageCommand::DevTools(RendererDevToolsPageCommand::Inspector(_)),
                ..
            } | RendererOwnerCommand::RunProtocolPageCommand {
                command: RendererPageCommand::DevTools(RendererDevToolsPageCommand::Inspector(_)),
                ..
            }
        ) {
            return Err(RenderRuntimeEnqueueError {
                command: Box::new(command),
                error: anyhow!(
                    "Inspector Page commands must enter through RendererPageHandle ingress"
                ),
            });
        }
        if moli_trace::cdp_nav_timing_enabled() {
            let page_command = match &command {
                RendererOwnerCommand::RunAsyncPageCommand { command, .. }
                | RendererOwnerCommand::RunProtocolPageCommand { command, .. } => Some(command),
                _ => None,
            };
            if let Some(command_label) =
                page_command.and_then(|command| command.cdp_nav_timing_label())
            {
                tracing::info!(
                    target: "moli_cdp_nav_timing",
                    command = command_label,
                    stage = "owner_command_enqueued",
                );
            }
        }
        let Some(state) = self.state.upgrade() else {
            return Err(RenderRuntimeEnqueueError {
                command: Box::new(command),
                error: anyhow!("render runtime thread has shut down"),
            });
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        let mut admission = state.admission.lock();
        if admission.terminal {
            return Err(RenderRuntimeEnqueueError {
                command: Box::new(command),
                error: anyhow!("render runtime command admission has shut down"),
            });
        }
        if let Err(error) = admission.send_ordered(RenderRuntimeEnvelope::Command {
            command: Box::new(command),
            reply_tx,
        }) {
            let command = match error.0 {
                RenderRuntimeEnvelope::Command { command, .. } => command,
                #[cfg(feature = "molio")]
                RenderRuntimeEnvelope::ViewportWheel { .. } => {
                    unreachable!("command admission sent a command envelope")
                }
                #[cfg(feature = "devtools")]
                RenderRuntimeEnvelope::InspectorMainReceiverWake(_) => {
                    unreachable!("command admission must recover the command envelope it sent")
                }
            };
            return Err(RenderRuntimeEnqueueError {
                command,
                error: anyhow!("render runtime thread has shut down"),
            });
        }
        Ok(reply_rx)
    }

    #[cfg(feature = "molio")]
    pub(crate) fn enqueue_viewport_wheel(
        &self,
        token: RendererPageToken,
        input: RendererViewportWheelInput,
    ) -> Result<()> {
        let state = self
            .state
            .upgrade()
            .ok_or_else(|| anyhow!("render runtime thread has shut down"))?;
        let mut admission = state.admission.lock();
        anyhow::ensure!(
            !admission.terminal && !admission.tx.is_closed(),
            "render runtime command admission has shut down"
        );
        if let Some((tail_token, tail)) = &admission.viewport_wheel_tail
            && *tail_token == token
            && let Some(pending) = tail.upgrade()
            && let Some(previous) = pending.lock().as_mut()
            && previous.coalesce(&input)
        {
            return Ok(());
        }

        let pending = Arc::new(Mutex::new(Some(input)));
        admission
            .tx
            .send(RenderRuntimeEnvelope::ViewportWheel {
                token,
                pending: Arc::clone(&pending),
            })
            .map_err(|_| anyhow!("render runtime thread has shut down"))?;
        admission.viewport_wheel_tail = Some((token, Arc::downgrade(&pending)));
        Ok(())
    }

    #[cfg(feature = "devtools")]
    pub(crate) fn enqueue_inspector_main_receiver_wake(
        &self,
        wake: RendererInspectorMainOwnerWake,
    ) -> Result<()> {
        let Some(state) = self.state.upgrade() else {
            return Err(anyhow!("render runtime thread has shut down"));
        };
        let mut admission = state.admission.lock();
        if admission.terminal {
            return Err(anyhow!("render runtime command admission has shut down"));
        }
        admission
            .send_ordered(RenderRuntimeEnvelope::InspectorMainReceiverWake(wake))
            .map_err(|_| anyhow!("render runtime thread has shut down"))
    }

    pub(crate) fn enqueue_owned(
        &self,
        command: RendererOwnerCommand,
    ) -> std::result::Result<oneshot::Receiver<Result<RendererOwnerReply>>, RenderRuntimeEnqueueError>
    {
        self.enqueue_command(command)
    }

    pub(crate) fn enqueue(
        &self,
        command: RendererOwnerCommand,
    ) -> Result<oneshot::Receiver<Result<RendererOwnerReply>>> {
        self.enqueue_command(command)
            .map_err(RenderRuntimeEnqueueError::into_error)
    }

    pub(crate) async fn dispatch(
        &self,
        command: RendererOwnerCommand,
    ) -> Result<RendererOwnerReply> {
        let reply_rx = self.enqueue(command)?;
        reply_rx
            .await
            .map_err(|_| anyhow!("render runtime reply channel closed"))?
    }

    pub(crate) fn close_admission(&self) {
        if let Some(state) = self.state.upgrade() {
            state.admission.lock().terminal = true;
        }
    }

    pub(crate) fn dispatch_detached(&self, command: RendererOwnerCommand) -> Result<()> {
        let _reply_rx = self
            .enqueue_command(command)
            .map_err(RenderRuntimeEnqueueError::into_error)?;
        Ok(())
    }
}

async fn render_runtime_main_loop(
    owner: RendererOwnerHandle,
    rx: mpsc::UnboundedReceiver<RenderRuntimeEnvelope>,
    page_wake_rx: mpsc::UnboundedReceiver<RendererOwnerWake>,
    inspector_io_wake_rx: mpsc::UnboundedReceiver<RendererInspectorIoOwnerWake>,
    shared_worker_wake_rx: mpsc::UnboundedReceiver<SharedWorkerRuntimeOwnerWake>,
    service_worker_wake_rx: mpsc::UnboundedReceiver<ServiceWorkerRuntimeOwnerWake>,
) {
    owner
        .run_render_runtime_loop(
            rx,
            page_wake_rx,
            inspector_io_wake_rx,
            shared_worker_wake_rx,
            service_worker_wake_rx,
        )
        .await;
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use super::*;

    #[cfg(feature = "molio")]
    #[test]
    fn viewport_wheel_admission_coalesces_pending_input_between_commands() -> Result<()> {
        use crate::runtime::{
            PageId, RendererDocumentLifecycleIdentity, RendererDocumentToken, RendererFrameToken,
            RendererLifecycleEpoch, RendererViewportWheelPhase,
        };

        let (tx, mut rx) = mpsc::unbounded_channel();
        let state = Arc::new(RenderRuntimeState {
            admission: Mutex::new(RenderRuntimeAdmission::new(tx)),
        });
        let handle = RenderRuntimeHandle {
            state: Arc::downgrade(&state),
        };
        let page_id = PageId::new_for_testing(1);
        let token = RendererPageToken::new_for_testing(page_id);
        let wheel = RendererViewportWheelInput {
            document: RendererDocumentLifecycleIdentity {
                frame: RendererFrameToken { page_id },
                document: RendererDocumentToken::new_for_testing(page_id, 1),
                epoch: RendererLifecycleEpoch(1),
            },
            viewport_revision: 1,
            position: (16., 16.),
            buttons: 0,
            delta: (0., 2.),
            modifiers: 0,
            phase: RendererViewportWheelPhase::Moved,
            precise: true,
        };
        handle.enqueue_viewport_wheel(token, wheel)?;
        let RenderRuntimeEnvelope::ViewportWheel {
            pending: in_flight, ..
        } = rx.try_recv()?
        else {
            panic!("first wheel should be eligible for immediate dispatch");
        };
        let first = in_flight.lock().take().expect("first wheel input");
        for _ in 0..120 {
            handle.enqueue_viewport_wheel(token, wheel)?;
        }
        let _navigation_reply = handle.enqueue(RendererOwnerCommand::RunProtocolPageCommand {
            token,
            command: RendererPageCommand::NavigateViewport(None),
        })?;
        handle.enqueue_viewport_wheel(token, wheel)?;
        let reversed = RendererViewportWheelInput {
            delta: (0., -2.),
            ..wheel
        };
        handle.enqueue_viewport_wheel(token, reversed)?;
        handle.enqueue_viewport_wheel(
            token,
            RendererViewportWheelInput {
                phase: RendererViewportWheelPhase::Ended,
                ..reversed
            },
        )?;

        let RenderRuntimeEnvelope::ViewportWheel {
            pending: merged, ..
        } = rx.try_recv()?
        else {
            panic!("expected one queued wheel before navigation");
        };
        assert_eq!(first.delta, (0., 2.));
        assert!(in_flight.lock().is_none());
        assert_eq!(
            merged.lock().take().expect("merged wheel input").delta,
            (0., 240.)
        );
        assert!(matches!(
            rx.try_recv()?, RenderRuntimeEnvelope::Command { command, .. }
                if matches!(*command, RendererOwnerCommand::RunProtocolPageCommand {
                    command: RendererPageCommand::NavigateViewport(None), ..
                })
        ));
        for expected in [
            wheel,
            reversed,
            RendererViewportWheelInput {
                phase: RendererViewportWheelPhase::Ended,
                ..reversed
            },
        ] {
            let RenderRuntimeEnvelope::ViewportWheel { pending, .. } = rx.try_recv()? else {
                panic!("expected a separate wheel at an input boundary");
            };
            let input = pending.lock().take().expect("queued wheel input");
            assert_eq!(input.delta, expected.delta);
            assert_eq!(input.phase, expected.phase);
        }
        assert!(matches!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        Ok(())
    }

    #[cfg(feature = "devtools")]
    #[test]
    fn raw_owner_admission_rejects_inspector_page_commands() {
        let handle = RenderRuntimeHandle::disconnected();
        let result = handle.enqueue(RendererOwnerCommand::RunProtocolPageCommand {
            token: crate::runtime::RendererPageToken::new_for_testing(
                crate::runtime::PageId::new_for_testing(1),
            ),
            command: RendererPageCommand::dispatch_runtime_protocol_message(
                None,
                r#"{"id":1,"method":"Runtime.getProperties","params":{"objectId":"1"}}"#.to_owned(),
            ),
        });
        let error = match result {
            Ok(_) => panic!("raw owner Inspector admission must be rejected"),
            Err(error) => error,
        };
        assert_eq!(
            error.to_string(),
            "Inspector Page commands must enter through RendererPageHandle ingress"
        );
    }

    #[test]
    fn dropping_render_runtime_owner_closes_and_joins_render_thread() {
        let render_exited = Arc::new(AtomicBool::new(false));

        let (render_tx, mut render_rx) = mpsc::unbounded_channel::<RenderRuntimeEnvelope>();
        let render_exited_for_thread = Arc::clone(&render_exited);
        let render_join = thread::Builder::new()
            .name("render_runtime_test".to_owned())
            .spawn(move || {
                while render_rx.blocking_recv().is_some() {}
                render_exited_for_thread.store(true, Ordering::SeqCst);
            })
            .expect("failed to spawn render runtime test thread");

        let state = Arc::new(RenderRuntimeState {
            admission: Mutex::new(RenderRuntimeAdmission::new(render_tx)),
        });
        let weak_handle = RenderRuntimeHandle {
            state: Arc::downgrade(&state),
        };
        let owner = RenderRuntimeOwner {
            state: Some(state),
            render_join: Some(render_join),
        };

        drop(owner);

        assert!(
            render_exited.load(Ordering::SeqCst),
            "render runtime worker should exit before owner drop returns"
        );
        assert!(
            weak_handle.state.upgrade().is_none(),
            "weak render runtime handle should not keep the runtime alive"
        );
    }
}
