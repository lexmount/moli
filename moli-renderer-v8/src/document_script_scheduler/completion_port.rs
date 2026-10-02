use std::sync::Arc;

use parking_lot::Mutex;

use crate::{dom::NodeId, planning::PreparedScriptSourceLoadOutcome};

type ParseTimeAsyncCompletionSender =
    dyn Fn(NodeId, PreparedScriptSourceLoadOutcome) -> bool + Send + Sync + 'static;

#[derive(Clone)]
pub(super) struct ParseTimeAsyncCompletionPort {
    sender: Arc<Mutex<Option<Arc<ParseTimeAsyncCompletionSender>>>>,
}

impl ParseTimeAsyncCompletionPort {
    pub(super) fn new(
        sender: impl Fn(NodeId, PreparedScriptSourceLoadOutcome) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            sender: Arc::new(Mutex::new(Some(Arc::new(sender)))),
        }
    }

    pub(super) fn send(&self, node_id: NodeId, outcome: PreparedScriptSourceLoadOutcome) -> bool {
        let sender = self.sender.lock();
        sender
            .as_ref()
            .is_some_and(|sender| sender(node_id, outcome))
    }

    pub(super) fn retire(&self) {
        *self.sender.lock() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn outcome() -> PreparedScriptSourceLoadOutcome {
        PreparedScriptSourceLoadOutcome {
            source_result: Ok("ready".to_owned()),
            source_bytes: None,
            network_result: None,
            muted_errors: false,
        }
    }

    #[test]
    fn retired_parse_time_completion_port_rejects_late_terminal() {
        let sends = Arc::new(AtomicUsize::new(0));
        let sends_for_port = sends.clone();
        let port = ParseTimeAsyncCompletionPort::new(move |_, _| {
            sends_for_port.fetch_add(1, Ordering::Relaxed);
            true
        });

        assert!(port.send(NodeId::new(1), outcome()));
        port.retire();
        assert!(!port.send(NodeId::new(2), outcome()));
        assert_eq!(sends.load(Ordering::Relaxed), 1);
    }
}
