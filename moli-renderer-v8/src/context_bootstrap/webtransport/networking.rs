//! Native results cross threads through a single-use mailbox. V8 handles stay
//! with the exact Window realm or worker that registered the operation.

use tokio::sync::oneshot;

use crate::{
    native_bridge::WindowExecutionContextIdentity,
    page_task_queue::{
        RendererPageWebTransportSender, RendererPageWebTransportTaskId,
        RendererPageWebTransportTaskKind,
    },
};

pub(crate) enum Completion {
    Connected(Result<moli_webtransport::Session, String>),
    Closed(moli_webtransport::CloseReason),
    Stream(super::streams::StreamCompletion),
    StreamClosed {
        id: u64,
        end: super::streams::StreamEnd,
    },
    Datagram(Result<(), String>),
}

pub(crate) struct Pending {
    payload: v8::Global<v8::Object>,
    result: oneshot::Receiver<Completion>,
}

impl Pending {
    pub(crate) fn new<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        payload: v8::Local<'s, v8::Object>,
    ) -> (Self, oneshot::Sender<Completion>) {
        let (sender, result) = oneshot::channel();
        (
            Self {
                payload: v8::Global::new(scope, payload),
                result,
            },
            sender,
        )
    }

    pub(crate) fn apply(mut self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        let Ok(result) = self.result.try_recv() else {
            return false;
        };
        let payload = v8::Local::new(scope, self.payload);
        super::backend::apply(scope, payload, result);
        true
    }

    pub(crate) fn context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Option<v8::Local<'s, v8::Context>> {
        v8::Local::new(scope, &self.payload).get_creation_context(scope)
    }
}

enum Route {
    Window {
        sender: RendererPageWebTransportSender,
        target: WindowExecutionContextIdentity,
        id: RendererPageWebTransportTaskId,
    },
    Worker {
        sender: tokio::sync::mpsc::UnboundedSender<crate::worker::WorkerMessage>,
        id: u64,
    },
}

pub(crate) struct Producer {
    result: oneshot::Sender<Completion>,
    route: Route,
}

impl Producer {
    pub(crate) fn window(
        result: oneshot::Sender<Completion>,
        sender: RendererPageWebTransportSender,
        target: WindowExecutionContextIdentity,
        id: RendererPageWebTransportTaskId,
    ) -> Self {
        Self {
            result,
            route: Route::Window { sender, target, id },
        }
    }

    pub(crate) fn worker(
        result: oneshot::Sender<Completion>,
        sender: tokio::sync::mpsc::UnboundedSender<crate::worker::WorkerMessage>,
        id: u64,
    ) -> Self {
        Self {
            result,
            route: Route::Worker { sender, id },
        }
    }

    pub(crate) fn send(self, result: Completion) {
        // Teardown drops the receiver before retiring the V8 payload. A late
        // result then releases its native resources without waking the owner.
        if self.result.send(result).is_err() {
            return;
        }
        match self.route {
            Route::Window { sender, target, id } => {
                sender.send(target, id, RendererPageWebTransportTaskKind::Complete);
            }
            Route::Worker { sender, id } => {
                let _ = sender.send(crate::worker::WorkerMessage::RunWebTransportTask(id));
            }
        }
    }
}

pub(super) fn register<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    payload: v8::Local<'s, v8::Object>,
) -> Option<Producer> {
    if let Some(host) = crate::util::context_host_ptr_from_global_bridge(scope) {
        // SAFETY: the current realm's bridge owns this isolate-local host.
        unsafe { &mut *host }.register_webtransport_task(scope, owner, payload)
    } else {
        crate::worker::webtransport_tasks::register(scope, payload)
    }
}
