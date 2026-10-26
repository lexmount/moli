//! Temporary actor-owned state access for workflows whose legacy neighbours
//! still borrow the connection across awaits. The actor executes each request
//! synchronously. Remove this mailbox when those entry points have migrated.

use tokio::sync::{mpsc, oneshot};

use super::CdpConnection;

type StateOperation = Box<dyn FnOnce(&mut CdpConnection)>;

pub struct ProtocolStateRequest(StateOperation);

impl ProtocolStateRequest {
    pub fn apply(self, connection: &mut CdpConnection) {
        (self.0)(connection);
    }
}

pub type ProtocolStateRequestReceiver = mpsc::UnboundedReceiver<ProtocolStateRequest>;

/// Revocable access to finite protocol operations. It neither owns a
/// connection nor lends a connection, reference, guard or future to callers.
#[derive(Clone)]
pub(crate) struct ProtocolStateAccess {
    sender: mpsc::WeakUnboundedSender<ProtocolStateRequest>,
}

pub(crate) struct ProtocolWorkflowState<'a> {
    connection: &'a mut CdpConnection,
}

pub(super) struct ProtocolStateMailbox {
    sender: mpsc::UnboundedSender<ProtocolStateRequest>,
    receiver: Option<ProtocolStateRequestReceiver>,
}

impl Default for ProtocolStateMailbox {
    fn default() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            sender,
            receiver: Some(receiver),
        }
    }
}

impl ProtocolStateAccess {
    pub(crate) async fn with_state<T: 'static>(
        &self,
        operation: impl for<'a> FnOnce(ProtocolWorkflowState<'a>) -> T + 'static,
    ) -> anyhow::Result<T> {
        let sender = self
            .sender
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("protocol owner closed"))?;
        let (reply, receive) = oneshot::channel();
        let owner = sender.downgrade();
        sender
            .send(ProtocolStateRequest(Box::new(move |connection| {
                // An aborted workflow must not register work merely because
                // its already-queued request is drained later.
                if !reply.is_closed()
                    && owner
                        .upgrade()
                        .is_some_and(|owner| owner.same_channel(&connection.state_mailbox.sender))
                {
                    let result = operation(ProtocolWorkflowState { connection });
                    let _ = reply.send(result);
                }
            })))
            .map_err(|_| anyhow::anyhow!("protocol owner closed"))?;
        drop(sender);
        receive
            .await
            .map_err(|_| anyhow::anyhow!("protocol owner closed"))
    }
}

impl CdpConnection {
    pub fn take_protocol_state_requests(&mut self) -> Option<ProtocolStateRequestReceiver> {
        self.state_mailbox.receiver.take()
    }

    pub(crate) fn protocol_state_access(&self) -> Option<ProtocolStateAccess> {
        // The inline compatibility driver must retain its old behaviour until
        // its caller takes responsibility for pumping state requests.
        self.state_mailbox
            .receiver
            .is_none()
            .then(|| ProtocolStateAccess {
                sender: self.state_mailbox.sender.downgrade(),
            })
    }
}

mod navigation_commit;
pub use navigation_commit::BackgroundPreparedNavigationCommit;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn queued_state_operation_is_skipped_after_its_workflow_is_cancelled() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let mut conn = CdpConnection::new();
                let mut requests = conn.take_protocol_state_requests().unwrap();
                let access = conn.protocol_state_access().unwrap();
                let ran = std::rc::Rc::new(std::cell::Cell::new(false));
                let operation_ran = ran.clone();
                let task = tokio::task::spawn_local(async move {
                    access.with_state(move |_| operation_ran.set(true)).await
                });
                let request = requests.recv().await.unwrap();
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                request.apply(&mut conn);
                assert!(!ran.get());
            })
            .await;
    }

    #[tokio::test]
    async fn workflow_access_does_not_keep_the_protocol_owner_alive() {
        let mut conn = CdpConnection::new();
        let _requests = conn.take_protocol_state_requests().unwrap();
        let access = conn.protocol_state_access().unwrap();
        drop(conn);
        assert_eq!(
            access.with_state(|_| ()).await.unwrap_err().to_string(),
            "protocol owner closed"
        );
    }

    #[tokio::test]
    async fn state_request_cannot_be_applied_to_another_connection() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let mut first = CdpConnection::new();
                let mut requests = first.take_protocol_state_requests().unwrap();
                let access = first.protocol_state_access().unwrap();
                let ran = std::rc::Rc::new(std::cell::Cell::new(false));
                let operation_ran = ran.clone();
                let task = tokio::task::spawn_local(async move {
                    access.with_state(move |_| operation_ran.set(true)).await
                });
                let request = requests.recv().await.unwrap();
                request.apply(&mut CdpConnection::new());
                assert!(task.await.unwrap().is_err());
                assert!(!ran.get());
            })
            .await;
    }

    #[tokio::test]
    async fn dropping_state_request_receiver_settles_a_waiting_workflow() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let mut conn = CdpConnection::new();
                let mut requests = conn.take_protocol_state_requests().unwrap();
                let access = conn.protocol_state_access().unwrap();
                let task = tokio::task::spawn_local(async move { access.with_state(|_| 42).await });
                let request = requests.recv().await.unwrap();
                drop(request);
                drop(requests);
                assert_eq!(
                    task.await.unwrap().unwrap_err().to_string(),
                    "protocol owner closed"
                );
            })
            .await;
    }
}
