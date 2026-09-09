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
            RendererWorkerLifecycle::Service(lifecycle) => {
                use crate::browser::{ServiceWorkerExecution, ServiceWorkerSnapshot};
                use crate::page::RendererServiceWorkerLifecycle;
                match lifecycle {
                    RendererServiceWorkerLifecycle::Created { info, active_run } => {
                        if context.service_workers.contains_key(&info.version_id) {
                            return;
                        }
                        let worker = ServiceWorkerSnapshot {
                            info: info.clone(),
                            execution: active_run.clone().map_or(
                                ServiceWorkerExecution::Stopped,
                                ServiceWorkerExecution::Starting,
                            ),
                        };
                        context
                            .service_workers
                            .insert(info.version_id, worker.clone());
                        BrowserEvent::WorkerCreated(WorkerSnapshot::Service {
                            context: id,
                            worker,
                        })
                    }
                    RendererServiceWorkerLifecycle::Destroyed {
                        version_id,
                        active_run,
                    } => {
                        let Some(worker) = context.service_workers.get(version_id) else {
                            return;
                        };
                        if worker.execution.active_run() != active_run.as_ref() {
                            return;
                        }
                        context.service_workers.shift_remove(version_id);
                        BrowserEvent::WorkerDestroyed(WorkerHandle::Service {
                            context: id,
                            version: *version_id,
                        })
                    }
                    RendererServiceWorkerLifecycle::Starting { version_id, .. }
                    | RendererServiceWorkerLifecycle::Started { version_id, .. }
                    | RendererServiceWorkerLifecycle::Stopped { version_id, .. }
                    | RendererServiceWorkerLifecycle::VersionUpdated { version_id, .. } => {
                        let Some(worker) = context.service_workers.get_mut(version_id) else {
                            return;
                        };
                        match lifecycle {
                            RendererServiceWorkerLifecycle::Starting { run, .. } => {
                                if worker.execution != ServiceWorkerExecution::Stopped {
                                    return;
                                }
                                worker.execution = ServiceWorkerExecution::Starting(run.clone());
                            }
                            RendererServiceWorkerLifecycle::Started { run, .. } => {
                                if worker.execution != ServiceWorkerExecution::Starting(run.clone())
                                {
                                    return;
                                }
                                worker.execution = ServiceWorkerExecution::Running(run.clone());
                            }
                            RendererServiceWorkerLifecycle::Stopped { run, .. } => {
                                if worker.execution.active_run() != Some(run) {
                                    return;
                                }
                                worker.execution = ServiceWorkerExecution::Stopped;
                            }
                            RendererServiceWorkerLifecycle::VersionUpdated { status, .. } => {
                                worker.info.status = *status;
                            }
                            _ => unreachable!("creation and retirement handled above"),
                        }
                        BrowserEvent::WorkerUpdated(WorkerSnapshot::Service {
                            context: id,
                            worker: worker.clone(),
                        })
                    }
                }
            }
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
