use std::any::Any;

use crate::context::AudioContextRegistration;
use crate::node::AudioNode;

use crate::events::{EventHandler, EventPayload, EventType};

/// One of the two ports of a message channel
///
/// Allowing messages to be sent from one port and listening out for them arriving at the other.
pub struct MessagePort<'a>(&'a AudioContextRegistration);

impl std::fmt::Debug for MessagePort<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessagePort").finish_non_exhaustive()
    }
}

impl<'a> MessagePort<'a> {
    pub(crate) fn from_node(node: &'a dyn AudioNode) -> Self {
        Self(node.registration())
    }

    /// Send a message from the port.
    pub fn post_message<M: Any + Send + 'static>(&self, msg: M) {
        self.0.post_message(msg);
    }

    /// Register callback to run when a message arrives on the channel.
    ///
    /// Only a single event handler is active at any time. Calling this method multiple times will
    /// override the previous event handler.
    pub fn set_onmessage<F: FnMut(Box<dyn Any + Send + 'static>) + Send + 'static>(
        &self,
        mut callback: F,
    ) {
        let callback = move |v| match v {
            EventPayload::Message(v) => callback(v),
            _ => unreachable!(),
        };

        self.0.context().set_event_handler(
            EventType::Message(self.0.id()),
            EventHandler::Multiple(Box::new(callback)),
        );
    }

    /// Unset the callback to run when a message arrives on the channel.
    pub fn clear_onmessage(&self) {
        self.0
            .context()
            .clear_event_handler(EventType::Message(self.0.id()));
    }
}
