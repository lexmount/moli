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
            RendererWorkerLifecycle::DedicatedCreated(info) => {
                if context.dedicated_workers.contains_key(&info.instance_id) {
                    return;
                }
                let worker = crate::browser::DedicatedWorkerSnapshot {
                    info: info.clone(),
                    main_script: None,
                };
                context
                    .dedicated_workers
                    .insert(info.instance_id, worker.clone());
                BrowserEvent::WorkerCreated(WorkerSnapshot::Dedicated {
                    context: id,
                    worker,
                })
            }
            RendererWorkerLifecycle::DedicatedScriptCompleted {
                instance_id,
                script,
            } => {
                let Some(worker) = context.dedicated_workers.get_mut(instance_id) else {
                    return;
                };
                if worker.main_script.is_some() {
                    return;
                }
                worker.main_script = Some(script.clone());
                BrowserEvent::WorkerUpdated(WorkerSnapshot::Dedicated {
                    context: id,
                    worker: worker.clone(),
                })
            }
            RendererWorkerLifecycle::DedicatedDestroyed(instance) => {
                if context.dedicated_workers.shift_remove(instance).is_none() {
                    return;
                }
                BrowserEvent::WorkerDestroyed(WorkerHandle::Dedicated {
                    context: id,
                    instance: *instance,
                })
            }
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
