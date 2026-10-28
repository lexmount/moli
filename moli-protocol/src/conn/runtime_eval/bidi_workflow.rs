use std::{cell::Cell, rc::Rc};

use anyhow::{Context as _, Result};

use super::*;
use crate::conn::{ProtocolStateAccess, ProtocolWorkflowDriver};

#[derive(Debug, thiserror::Error)]
#[error("BiDi channel listener requires a scheduler response hook")]
struct BidiChannelResponseHookUnavailable;

/// Owns a proxy's object group until the same finite operation registers its
/// listener. Cancellation before that handoff publishes the existing cleanup
/// action for the original Page, never for a successor runtime.
pub(crate) struct BidiChannelObjectGroupLease {
    owner: BidiChannelPageOwner,
    group: String,
    access: Option<ProtocolStateAccess>,
}

impl BidiChannelObjectGroupLease {
    pub(crate) fn allocate(conn: &mut CdpConnection, owner: BidiChannelPageOwner) -> Option<Self> {
        owner.can_start_listener(conn).then(|| {
            let group = conn.next_bidi_channel_object_group();
            Self::capture(conn, owner, group)
        })
    }

    fn capture(conn: &CdpConnection, owner: BidiChannelPageOwner, group: String) -> Self {
        Self {
            owner,
            group,
            access: conn.protocol_state_access(),
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.group
    }

    pub(crate) fn release(&mut self, conn: &mut CdpConnection) {
        self.access.take();
        if self.owner.is_current(conn) {
            conn.publish_bidi_channel_object_group_release(self.owner.clone(), self.group.clone());
        }
    }

    pub(crate) fn publish_listener(
        mut self,
        conn: &mut CdpConnection,
        listener: PendingBidiChannelListener,
    ) -> bool {
        assert_eq!(
            self.name(),
            listener.channel_object_group(),
            "a listener owns its proxy group"
        );
        if !self.owner.can_start_listener(conn) {
            self.release(conn);
            return false;
        }
        conn.publish_bidi_channel_listener_start(BidiChannelListenerResidence::new(
            self.owner.clone(),
            listener,
        ));
        self.access.take();
        true
    }
}

impl Drop for BidiChannelObjectGroupLease {
    fn drop(&mut self) {
        if let Some(access) = self.access.take() {
            let owner = self.owner.clone();
            let group = self.group.clone();
            access.defer_cleanup(Box::new(move |conn| {
                if owner.is_current(conn) {
                    conn.publish_bidi_channel_object_group_release(owner, group);
                }
            }));
        }
    }
}

/// Materialized protocol facts and their original renderer ordering lease.
/// This is output ownership, not a new dispatch state machine.
#[derive(Clone, Debug, Default)]
pub(crate) struct BidiChannelOutput {
    pub(crate) events: Vec<BackgroundProtocolEvent>,
    pub(crate) predecessor: Option<moli_core::RendererOutputFence>,
    pub(crate) target_id: Option<String>,
}

impl BidiChannelOutput {
    pub(crate) fn append(&mut self, other: Self) {
        self.events.extend(other.events);
        if let Some(predecessor) = other.predecessor {
            predecessor.merge_into_same_stream_tail(&mut self.predecessor);
        }
    }

    pub(crate) fn publish(self, conn: &mut CdpConnection) -> Vec<BackgroundProtocolEvent> {
        if let Some(predecessor) = self.predecessor {
            conn.publish_renderer_fenced_protocol_output(self.events, predecessor, self.target_id);
            Vec::new()
        } else {
            self.events
        }
    }
}

#[derive(Clone, Copy)]
enum BidiChannelDispatchKind {
    Listener,
    ReleaseObjectGroup,
}

#[derive(Clone)]
pub(crate) struct BidiChannelDispatchIdentity {
    owner: BidiChannelPageOwner,
    correlation: RendererCommandCorrelation,
    group: String,
    kind: BidiChannelDispatchKind,
    settled: Rc<Cell<bool>>,
}

impl BidiChannelDispatchIdentity {
    pub(crate) fn cancel(&self, conn: &mut CdpConnection) {
        if self.settled.replace(true) || !self.owner.owns_current_host(conn) {
            return;
        }
        let owner = self.owner.command_owner();
        let matches = conn
            .target_devtools_session_state_for_owner(owner)
            .and_then(|session| {
                session.renderer_call_for_frontend(self.correlation.frontend_command_id().get())
            })
            == Some(self.correlation);
        match self.kind {
            BidiChannelDispatchKind::Listener if matches => {
                conn.remove_pending_inspector_await_for_cancellation_for_owner(
                    self.correlation.frontend_command_id().get(),
                    owner,
                );
                if self.owner.is_current(conn) {
                    conn.publish_bidi_channel_object_group_release(
                        self.owner.clone(),
                        self.group.clone(),
                    );
                }
            }
            BidiChannelDispatchKind::ReleaseObjectGroup => {
                conn.take_renderer_call_if_correlation_matches_for_owner(owner, self.correlation);
                if self.owner.is_current(conn) {
                    conn.unregister_runtime_remote_object_group_for_owner(owner, &self.group);
                }
            }
            BidiChannelDispatchKind::Listener => {}
        }
    }

    fn is_current(&self, conn: &CdpConnection) -> bool {
        self.owner.is_current(conn)
            && conn
                .target_devtools_session_state_for_owner(self.owner.command_owner())
                .and_then(|session| {
                    session.renderer_call_for_frontend(self.correlation.frontend_command_id().get())
                })
                == Some(self.correlation)
    }

    pub(crate) fn finish_dispatch(
        &self,
        conn: &mut CdpConnection,
        mut completed: CompletedRuntimeProtocolMessageDispatch,
    ) -> Result<BidiChannelDispatchOutput> {
        anyhow::ensure!(self.is_current(conn), NoDocumentLoaded);
        let mut receiver = completed.take_deferred_response_receiver();
        let output = conn.complete_runtime_protocol_message(completed)?;
        let command_id = self.correlation.frontend_command_id().get();
        let owner = self.owner.command_owner();
        let mut events = Vec::new();
        let mut post_response_events = Vec::new();
        let mut renderer_predecessor = None;
        let response_seen = output.is_some_and(|output| {
            let (seen, predecessor) = conn.route_renderer_command_turn_output_for_owner_into(
                output,
                Some(command_id),
                owner,
                &CommandResponseFlushContext::default(),
                &mut events,
                &mut post_response_events,
            );
            renderer_predecessor = predecessor;
            seen
        });
        events.extend(post_response_events);
        if response_seen {
            receiver.take();
        }
        if matches!(self.kind, BidiChannelDispatchKind::Listener) {
            if let Some(receiver) = receiver.take() {
                anyhow::ensure!(
                    conn.start_or_enqueue_registered_runtime_inspector_response_ready(
                        command_id,
                        &self.owner,
                        receiver
                    ),
                    BidiChannelResponseHookUnavailable
                );
            }
            // Handoff and disarming are one synchronous operation. Losing the
            // mailbox result cannot cancel the already registered long waiter.
            self.settled.set(true);
        }
        events.retain(|event| event.protocol_message_id().is_none());
        Ok(BidiChannelDispatchOutput {
            output: BidiChannelOutput {
                events,
                predecessor: renderer_predecessor,
                target_id: self.owner.target_id().map(str::to_owned),
            },
            receiver,
        })
    }

    pub(crate) fn finish_response(
        &self,
        conn: &mut CdpConnection,
        response: moli_core::RendererRuntimeInspectorAsyncCompletion,
    ) -> Result<BidiChannelOutput> {
        anyhow::ensure!(self.is_current(conn), NoDocumentLoaded);
        let command_id = self.correlation.frontend_command_id().get();
        let owner = self.owner.command_owner();
        let ready = RuntimeInspectorResponseReady::for_owner(command_id, owner, Ok(response));
        let mut events = Vec::new();
        let mut renderer_predecessor = None;
        if let Some(response) = conn.resolve_runtime_inspector_response_ready(ready) {
            let (_, output, predecessor) = response.into_renderer_command_output();
            renderer_predecessor = predecessor;
            conn.route_renderer_runtime_command_output_for_owner_into(
                output,
                Some(command_id),
                owner,
                &mut events,
            );
        }
        Ok(BidiChannelOutput {
            events,
            predecessor: renderer_predecessor,
            target_id: self.owner.target_id().map(str::to_owned),
        })
    }
}

struct BidiChannelDispatchCancellation {
    identity: BidiChannelDispatchIdentity,
    access: Option<ProtocolStateAccess>,
}

impl Drop for BidiChannelDispatchCancellation {
    fn drop(&mut self) {
        if !self.identity.settled.get()
            && let Some(access) = self.access.take()
        {
            let identity = self.identity.clone();
            access.defer_cleanup(Box::new(move |conn| identity.cancel(conn)));
        }
    }
}

pub(crate) struct BidiChannelDispatchOutput {
    output: BidiChannelOutput,
    receiver: Option<RuntimeInspectorResponseReceiver>,
}

/// The initial native dispatch is finite. Listener promise replies continue
/// through the original response-ready lane; only object-group release waits
/// for its own private reply here.
pub(crate) struct PreparedBidiChannelAction {
    pending: Option<PendingRuntimeProtocolMessageDispatch>,
    cancellation: BidiChannelDispatchCancellation,
}

impl PreparedBidiChannelAction {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        action: BidiChannelOwnerAction,
    ) -> Option<Self> {
        let (owner, body) = action.into_parts();
        if !owner.is_current(conn) {
            return None;
        }
        match body {
            BidiChannelOwnerActionBody::StartListener(listener) => {
                let lease = BidiChannelObjectGroupLease::capture(
                    conn,
                    owner,
                    listener.channel_object_group().to_owned(),
                );
                Self::prepare_listener(conn, lease, *listener)
            }
            BidiChannelOwnerActionBody::ReleaseObjectGroup(group) => {
                let result = Self::prepare_dispatch(conn, owner.clone(), group.clone(), None);
                match result {
                    Ok(prepared) => Some(prepared),
                    Err(error) => {
                        tracing::debug!(
                            ?error,
                            group,
                            "failed to prepare BiDi channel object group release"
                        );
                        conn.unregister_runtime_remote_object_group_for_owner(
                            owner.command_owner(),
                            &group,
                        );
                        None
                    }
                }
            }
        }
    }

    pub(crate) fn prepare_listener(
        conn: &mut CdpConnection,
        mut lease: BidiChannelObjectGroupLease,
        listener: PendingBidiChannelListener,
    ) -> Option<Self> {
        assert_eq!(
            lease.name(),
            listener.channel_object_group(),
            "the listener takes ownership of its proxy group"
        );
        if !lease.owner.can_start_listener(conn) {
            lease.release(conn);
            return None;
        }
        let result = Self::prepare_dispatch(
            conn,
            lease.owner.clone(),
            lease.group.clone(),
            Some(listener),
        );
        match result {
            Ok(prepared) => {
                lease.access.take();
                Some(prepared)
            }
            Err(error) => {
                tracing::debug!(?error, "failed to prepare BiDi channel listener");
                lease.release(conn);
                None
            }
        }
    }

    fn prepare_dispatch(
        conn: &mut CdpConnection,
        owner: BidiChannelPageOwner,
        group: String,
        listener: Option<PendingBidiChannelListener>,
    ) -> Result<Self> {
        anyhow::ensure!(owner.is_current(conn), NoDocumentLoaded);
        let command_id = conn.next_internal_runtime_command_id();
        let (raw, action, kind) = if let Some(listener) = listener.as_ref() {
            anyhow::ensure!(
                conn.runtime_inspector_response_ready_sender().is_some(),
                BidiChannelResponseHookUnavailable
            );
            (
                bidi_channel_listener_call_function_json(command_id, listener),
                "callFunctionOn",
                BidiChannelDispatchKind::Listener,
            )
        } else {
            let raw = json!({
                "id": command_id,
                "method": "Runtime.releaseObjectGroup",
                "params": { "objectGroup": group },
            })
            .to_string();
            (
                raw,
                "releaseObjectGroup",
                BidiChannelDispatchKind::ReleaseObjectGroup,
            )
        };
        let descriptor = RendererCommandDescriptor::for_document_helper(raw)?;
        let command_owner = owner.command_owner();
        let pending = conn.start_runtime_protocol_message_with_context_resolution_for_owner_with_deferred_response(
            command_owner, action, descriptor, command_id,
        )?;
        let correlation = conn
            .target_devtools_session_state_for_owner(command_owner)
            .and_then(|session| session.renderer_call_for_frontend(command_id))
            .expect("finite BiDi admission registered its exact renderer call");
        if let Some(listener) = listener {
            conn.register_pending_bidi_channel_listener_for_owner(
                command_id,
                command_owner,
                BidiChannelListenerResidence::new(owner.clone(), listener),
            );
        }
        Ok(Self {
            pending: Some(pending),
            cancellation: BidiChannelDispatchCancellation {
                identity: BidiChannelDispatchIdentity {
                    owner,
                    correlation,
                    group,
                    kind,
                    settled: Rc::new(Cell::new(false)),
                },
                access: conn.protocol_state_access(),
            },
        })
    }

    pub(crate) async fn run(
        mut self,
        driver: &mut ProtocolWorkflowDriver<'_>,
    ) -> BidiChannelOutput {
        let mut events = BidiChannelOutput {
            target_id: self
                .cancellation
                .identity
                .owner
                .target_id()
                .map(str::to_owned),
            ..BidiChannelOutput::default()
        };
        if let Err(error) = self.run_inner(driver, &mut events).await {
            tracing::debug!(?error, "BiDi channel owner action failed");
        }
        let identity = self.cancellation.identity.clone();
        if driver
            .with_state(move |state| state.cancel_bidi_channel_dispatch(identity))
            .await
            .is_ok()
        {
            self.cancellation.access.take();
        }
        events
    }

    async fn run_inner(
        &mut self,
        driver: &mut ProtocolWorkflowDriver<'_>,
        events: &mut BidiChannelOutput,
    ) -> Result<()> {
        let completed = self
            .pending
            .take()
            .expect("one BiDi action owns one native dispatch")
            .wait()
            .await?;
        if let Some(predecessor) = completed.renderer_output_predecessor() {
            predecessor.merge_into_same_stream_tail(&mut events.predecessor);
        }
        let identity = self.cancellation.identity.clone();
        let output = driver
            .with_state(move |state| state.finish_bidi_channel_dispatch(identity, completed))
            .await??;
        events.append(output.output);
        if let Some(receiver) = output.receiver {
            let response = receiver
                .await
                .context("BiDi object-group release response cancelled")?;
            if let Some(predecessor) = response.renderer_output_predecessor() {
                predecessor.merge_into_same_stream_tail(&mut events.predecessor);
            }
            let identity = self.cancellation.identity.clone();
            events.append(
                driver
                    .with_state(move |state| {
                        state.finish_bidi_channel_dispatch_response(identity, response)
                    })
                    .await??,
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
