//! Completion ownership, independent of the backend and its execution lane.
//!
//! An internal operation returns a value to its adapter. A frontend call owns
//! a terminal publication and returns only its publication receipt to a waiter.
//! Neither receipt is an execution lock or an acknowledgement from the socket.

use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::oneshot;

/// The two destinations are exclusive. An internal query cannot claim a
/// frontend terminal, and a published frontend reply cannot also be returned
/// through an adapter value channel.
enum Destination<Completion, Settlement> {
    Adapter(oneshot::Sender<Completion>),
    Session(Option<oneshot::Sender<Settlement>>),
}

struct State<Completion, Settlement> {
    next_lease_id: u64,
    active_lease_id: Option<u64>,
    // Taking this destination is the linearization point for completion or
    // cancellation. Revocation only clears the active lease: replacement may
    // acquire a new lease for the same still-open frontend call.
    destination: Option<Destination<Completion, Settlement>>,
}

pub(crate) struct RendererCommandResponseAuthority<Completion, Settlement> {
    state: Arc<Mutex<State<Completion, Settlement>>>,
}

impl<C, S> Clone for RendererCommandResponseAuthority<C, S> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
}

impl<C, S> std::fmt::Debug for RendererCommandResponseAuthority<C, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock();
        f.debug_struct("RendererCommandResponseAuthority")
            .field("active_lease_id", &state.active_lease_id)
            .field("open", &state.destination.is_some())
            .finish()
    }
}

impl<C, S> PartialEq for RendererCommandResponseAuthority<C, S> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

impl<C, S> Eq for RendererCommandResponseAuthority<C, S> {}

impl<C, S> RendererCommandResponseAuthority<C, S> {
    fn new(destination: Destination<C, S>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                next_lease_id: 1,
                active_lease_id: None,
                destination: Some(destination),
            })),
        }
    }

    pub(crate) fn adapter(tx: oneshot::Sender<C>) -> Self {
        Self::new(Destination::Adapter(tx))
    }

    pub(crate) fn session() -> Self {
        Self::new(Destination::Session(None))
    }

    pub(crate) fn activate(&self) -> Option<RendererCommandResponseLease<C, S>> {
        let lease_id = {
            let mut state = self.state.lock();
            state.destination.as_ref()?;
            let id = state.next_lease_id;
            state.next_lease_id = id
                .checked_add(1)
                .expect("renderer response lease id exhausted");
            state.active_lease_id = Some(id);
            id
        };
        Some(RendererCommandResponseLease {
            lifetime: Arc::new(LeaseLifetime {
                lease_id,
                authority: self.clone(),
            }),
        })
    }

    pub(crate) fn try_revoke_active_lease(&self) -> bool {
        self.state.lock().active_lease_id.take().is_some()
    }

    pub(crate) fn cancel(&self) {
        let destination = {
            let mut state = self.state.lock();
            state.active_lease_id = None;
            state.destination.take()
        };
        drop(destination);
    }

    fn cancel_lease(&self, lease_id: u64) {
        let destination = {
            let mut state = self.state.lock();
            if state.active_lease_id != Some(lease_id) {
                return;
            }
            state.active_lease_id = None;
            state.destination.take()
        };
        drop(destination);
    }
}

struct LeaseLifetime<C, S> {
    lease_id: u64,
    authority: RendererCommandResponseAuthority<C, S>,
}

impl<C, S> Drop for LeaseLifetime<C, S> {
    fn drop(&mut self) {
        // Only the last copy of the active lease abandons this call. Dropping
        // a retired renderer's lease must not cancel its replacement.
        self.authority.cancel_lease(self.lease_id);
    }
}

pub(crate) struct RendererCommandResponseLease<C, S> {
    lifetime: Arc<LeaseLifetime<C, S>>,
}

impl<C, S> Clone for RendererCommandResponseLease<C, S> {
    fn clone(&self) -> Self {
        Self {
            lifetime: self.lifetime.clone(),
        }
    }
}

impl<C, S> std::fmt::Debug for RendererCommandResponseLease<C, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RendererCommandResponseLease")
            .field("lease_id", &self.lifetime.lease_id)
            .field("authority", &self.lifetime.authority)
            .finish()
    }
}

impl<C, S> RendererCommandResponseLease<C, S> {
    /// A native response can arrive after its adapter has cancelled. Check
    /// before deferring publication: the parent queue must not hide a closed
    /// receiver from the helper's native terminal cleanup.
    pub(crate) fn is_open(&self) -> bool {
        let state = self.lifetime.authority.state.lock();
        state.active_lease_id == Some(self.lifetime.lease_id)
            && match state.destination.as_ref() {
                Some(Destination::Adapter(tx)) => !tx.is_closed(),
                Some(Destination::Session(_)) => true,
                None => false,
            }
    }

    pub(crate) fn send_adapter_reply(self, completion: C) -> Result<(), C> {
        let tx = {
            let mut state = self.lifetime.authority.state.lock();
            if state.active_lease_id != Some(self.lifetime.lease_id)
                || !matches!(state.destination, Some(Destination::Adapter(_)))
            {
                return Err(completion);
            }
            state.active_lease_id = None;
            let Some(Destination::Adapter(tx)) = state.destination.take() else {
                unreachable!("destination was checked under the same lock")
            };
            tx
        };
        tx.send(completion)
    }

    pub(crate) fn take_session_response_settlement_receiver(&self) -> Option<oneshot::Receiver<S>> {
        let mut state = self.lifetime.authority.state.lock();
        if state.active_lease_id != Some(self.lifetime.lease_id) {
            return None;
        }
        let Some(Destination::Session(tx @ None)) = &mut state.destination else {
            return None;
        };
        let (sender, receiver) = oneshot::channel();
        *tx = Some(sender);
        Some(receiver)
    }

    pub(crate) fn claim_session(&self) -> Result<Option<oneshot::Sender<S>>, ()> {
        let mut state = self.lifetime.authority.state.lock();
        if state.active_lease_id != Some(self.lifetime.lease_id)
            || !matches!(state.destination, Some(Destination::Session(_)))
        {
            return Err(());
        }
        state.active_lease_id = None;
        let Some(Destination::Session(tx)) = state.destination.take() else {
            unreachable!("destination was checked under the same lock")
        };
        Ok(tx)
    }
}
