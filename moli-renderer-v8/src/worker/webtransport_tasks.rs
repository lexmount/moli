//! Worker-local V8 payloads and one-shot networking completions.

use super::WorkerMessage;
use crate::context_bootstrap::webtransport::{
    backend::ConnectionContext,
    networking::{Pending, Producer},
};
use crate::network::loads::{ResourceLoadDisposition, ResourceLoadKind};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use tokio::sync::mpsc;

#[derive(Default)]
struct Tasks {
    next_id: u64,
    pending: HashMap<u64, Pending>,
}

#[derive(Clone)]
pub(super) struct WorkerWebTransportTasks {
    tasks: Rc<RefCell<Tasks>>,
    wake: mpsc::UnboundedSender<WorkerMessage>,
}

impl WorkerWebTransportTasks {
    pub(super) fn new(wake: mpsc::UnboundedSender<WorkerMessage>) -> Self {
        Self {
            tasks: Default::default(),
            wake,
        }
    }
    pub(super) fn is_empty(&self) -> bool {
        self.tasks.borrow().pending.is_empty()
    }
    pub(super) fn clear(&self) {
        self.tasks.borrow_mut().pending.clear();
    }
    pub(super) fn settle(&self, scope: &mut v8::PinScope<'_, '_>, id: u64) {
        let pending = self.tasks.borrow_mut().pending.remove(&id);
        if let Some(pending) = pending {
            pending.apply(scope);
        }
    }
}

pub(crate) fn register<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    payload: v8::Local<'s, v8::Object>,
) -> Option<Producer> {
    let queue = scope.get_slot::<WorkerWebTransportTasks>()?.clone();
    let (pending, result) = Pending::new(scope, payload);
    let id = {
        let mut tasks = queue.tasks.borrow_mut();
        tasks.next_id = tasks
            .next_id
            .checked_add(1)
            .expect("worker WebTransport task id exhausted");
        let id = tasks.next_id;
        assert!(tasks.pending.insert(id, pending).is_none());
        id
    };
    Some(Producer::worker(result, queue.wake, id))
}

pub(crate) fn connection_context(
    scope: &mut v8::PinScope<'_, '_>,
    url: &url::Url,
) -> Option<ConnectionContext> {
    let state = super::get_worker_state(scope)?;
    let state = state.borrow();
    if !state.callback_task_delivery_allowed() {
        return None;
    }
    let lease = state.loader.register_load(
        ResourceLoadKind::WebTransport,
        ResourceLoadDisposition::Ordinary,
        None,
    )?;
    let blocked = state.network_offline
        || state
            .blocked_url_patterns
            .iter()
            .any(|pattern| moli_fetch::url_pattern_matches(pattern, url.as_str()));
    Some(ConnectionContext {
        runner: state.loader.task_runner(),
        connector: state.loader.request_client().webtransport_connector(),
        lease,
        origin: super::global_scope::worker_global_origin(scope)?
            .ascii_serialization()
            .to_owned(),
        blocked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_bootstrap::webtransport::networking::Completion;

    #[test]
    fn webtransport_retired_worker_drops_late_completion_without_a_wake() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let (wake, mut messages) = mpsc::unbounded_channel();
        let tasks = WorkerWebTransportTasks::new(wake);
        scope.set_slot(tasks.clone());
        let payload = v8::Object::new(scope);
        let producer = register(scope, payload).unwrap();
        assert!(!tasks.is_empty());
        tasks.clear();
        scope.remove_slot::<WorkerWebTransportTasks>();
        producer.send(Completion::Connected(Err("late TLS failure".into())));
        assert!(tasks.is_empty());
        assert!(matches!(
            messages.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn webtransport_stale_worker_wake_cannot_take_a_new_operation() {
        crate::ensure_v8_for_test();
        let mut isolate = v8::Isolate::new(Default::default());
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let (wake, mut messages) = mpsc::unbounded_channel();
        let tasks = WorkerWebTransportTasks::new(wake);
        scope.set_slot(tasks.clone());
        let payload = v8::Object::new(scope);
        register(scope, payload)
            .unwrap()
            .send(Completion::Connected(Err("first failure".into())));
        let WorkerMessage::RunWebTransportTask(first) = messages.try_recv().unwrap() else {
            panic!("wrong task");
        };
        tasks.clear();
        let payload = v8::Object::new(scope);
        let producer = register(scope, payload).unwrap();
        tasks.settle(scope, first);
        assert!(!tasks.is_empty());
        producer.send(Completion::Connected(Err("second failure".into())));
        let WorkerMessage::RunWebTransportTask(second) = messages.try_recv().unwrap() else {
            panic!("wrong task");
        };
        assert_ne!(first, second);
        tasks.settle(scope, second);
        assert!(tasks.is_empty());
        assert!(matches!(
            messages.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        scope.remove_slot::<WorkerWebTransportTasks>();
    }
}
