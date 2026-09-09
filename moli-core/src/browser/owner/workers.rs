use crate::browser::{BrowserContextId, BrowserEvent, WorkerHandle, WorkerSnapshot};
use crate::page::{RendererWorkerLifecycle, RendererWorkerLifecycleInput};

use super::Browser;

impl Browser {
    pub(super) fn commit_worker_lifecycle(
        &mut self,
        id: BrowserContextId,
        input: RendererWorkerLifecycleInput,
    ) {
        let Ok(context) = self.context_mut(id) else {
            return;
        };
        if !context.routes_renderer_browser_context_runtime(input.runtime) {
            return;
        }
        let event = match input.lifecycle.as_ref() {
            RendererWorkerLifecycle::SharedCreated(info) => {
                if context.shared_workers.contains_key(&info.instance_id) {
                    return;
                }
                context
                    .shared_workers
                    .insert(info.instance_id, info.clone());
                BrowserEvent::WorkerCreated(WorkerSnapshot::Shared {
                    context: id,
                    info: info.clone(),
                })
            }
            RendererWorkerLifecycle::SharedDestroyed(instance) => {
                if context.shared_workers.shift_remove(instance).is_none() {
                    return;
                }
                BrowserEvent::WorkerDestroyed(WorkerHandle::Shared {
                    context: id,
                    instance: *instance,
                })
            }
        };
        // Mutation, native occurrence and FIFO acknowledgement form one owner
        // turn. Protocol cannot invent or commit this lifecycle by draining output.
        let record = self.events.publish(event);
        input.commit(record.sequence.get());
    }

    pub(super) fn publish_retired_workers(&mut self, context: &super::BrowserContext) {
        for worker in context.worker_snapshots() {
            self.events
                .publish(BrowserEvent::WorkerDestroyed(worker.handle()));
        }
    }
}
