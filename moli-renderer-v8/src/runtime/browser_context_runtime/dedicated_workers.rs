use std::{
    collections::HashMap,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use super::RendererBrowserContextRuntime;
use crate::runtime::{
    PendingRendererOutputRecord, RendererDedicatedWorkerMainScript, RendererDedicatedWorkerOwner,
    RendererDedicatedWorkerTargetInfo, RendererNetworkReporter, RendererOutputFence,
    RendererOutputTransportSender, RendererOutputTransportSenderSlot, RendererProtocolObservation,
    RendererRuntimeInspectorResponsePublication, RendererTurnOutputJournal, RendererWorkerIdentity,
    RendererWorkerInspectionEndpoint, RendererWorkerLifecycle, RendererWorkerLifecycleReporter,
    RendererWorkerNetworkReporter, RendererWorkerOutputStreams,
};
use crate::worker::WorkerDevToolsHandle;
use parking_lot::Mutex;

/// Context-owned directory and transport binding. Nested producers retain
/// neither BrowserContext services nor the thread owner through this capability.
#[derive(Debug)]
pub(crate) struct RendererDedicatedWorkerRegistry {
    pub(super) network: RendererNetworkReporter,
    pub(super) outputs: RendererWorkerOutputStreams,
    pub(super) transport: RendererOutputTransportSenderSlot,
    next_instance: AtomicU64,
    hosts: Mutex<HashMap<u64, Weak<RendererDedicatedWorkerHostInner>>>,
    pause_on_start: AtomicBool,
}

impl RendererDedicatedWorkerRegistry {
    pub(super) fn new(network: RendererNetworkReporter) -> Self {
        let transport = RendererOutputTransportSenderSlot::default();
        Self {
            outputs: RendererWorkerOutputStreams::new(
                RendererWorkerLifecycleReporter::new(network.runtime()),
                transport.clone(),
            ),
            transport,
            network,
            next_instance: AtomicU64::new(1),
            hosts: Mutex::new(HashMap::new()),
            pause_on_start: AtomicBool::new(false),
        }
    }

    pub(crate) fn create(
        self: &Arc<Self>,
        owner: RendererDedicatedWorkerOwner,
        request_url: String,
        document_url: String,
        name: String,
    ) -> RendererDedicatedWorkerHost {
        let instance_id = self
            .next_instance
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("Dedicated Worker identity exhausted");
        let identity = RendererWorkerIdentity::Dedicated(instance_id);
        let host = RendererDedicatedWorkerHost(Arc::new(RendererDedicatedWorkerHostInner {
            info: RendererDedicatedWorkerTargetInfo {
                owner,
                instance_id,
                request_url,
                document_url,
                name,
            },
            network: RendererWorkerNetworkReporter::new(self.network.clone(), identity.clone()),
            output: self.outputs.open(identity),
            registry: self.clone(),
            execution: Mutex::new(DedicatedWorkerExecution::Loading),
            publication: Mutex::new(()),
        }));
        self.hosts
            .lock()
            .insert(instance_id, Arc::downgrade(&host.0));
        host.lifecycle(RendererWorkerLifecycle::DedicatedCreated(
            host.0.info.clone(),
        ));
        host
    }

    fn host(&self, instance: u64) -> Option<RendererDedicatedWorkerHost> {
        self.hosts
            .lock()
            .get(&instance)?
            .upgrade()
            .map(RendererDedicatedWorkerHost)
    }

    pub(super) fn bind_transport(&self, sender: RendererOutputTransportSender) {
        self.outputs.bind_transport(sender);
    }

    pub(super) fn shutdown(&self) {
        let hosts = self
            .hosts
            .lock()
            .values()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for host in hosts {
            let worker = match &*host.execution.lock() {
                DedicatedWorkerExecution::Running(worker) => Some(worker.clone()),
                DedicatedWorkerExecution::Loading | DedicatedWorkerExecution::Retired(_) => None,
            };
            if let Some(worker) = worker {
                let _ = worker.terminate_for_devtools();
            } else {
                host.retire();
            }
        }
    }
}

#[derive(Debug)]
enum DedicatedWorkerExecution {
    Loading,
    Running(WorkerDevToolsHandle),
    Retired(Option<RendererOutputFence>),
}

#[derive(Debug)]
struct RendererDedicatedWorkerHostInner {
    info: RendererDedicatedWorkerTargetInfo,
    registry: Arc<RendererDedicatedWorkerRegistry>,
    network: RendererWorkerNetworkReporter,
    output: RendererTurnOutputJournal,
    execution: Mutex<DedicatedWorkerExecution>,
    // Serialize native facts and their source records without holding the
    // inspection-state lock across native observer callbacks.
    publication: Mutex<()>,
}

impl RendererDedicatedWorkerHostInner {
    fn publish(&self, observation: RendererProtocolObservation) {
        self.output.publish_record(
            PendingRendererOutputRecord::observation(None, observation)
                .resolve()
                .expect("Worker output has a concrete source"),
        );
    }

    fn retire(&self) {
        let _publication = self.publication.lock();
        {
            let mut execution = self.execution.lock();
            if matches!(*execution, DedicatedWorkerExecution::Retired(_)) {
                return;
            }
            *execution = DedicatedWorkerExecution::Retired(None);
        }
        self.network.close_source();
        self.publish(RendererProtocolObservation::WorkerLifecycle(
            self.registry.outputs.worker_lifecycle.report(
                RendererWorkerLifecycle::DedicatedDestroyed(self.info.instance_id),
            ),
        ));
        let terminal = self
            .output
            .last_published_cursor()
            .map(|cursor| self.output.declare_fence(cursor));
        *self.execution.lock() = DedicatedWorkerExecution::Retired(terminal);
        self.registry
            .outputs
            .retire(&RendererWorkerIdentity::Dedicated(self.info.instance_id));
        self.registry.hosts.lock().remove(&self.info.instance_id);
    }
}

impl Drop for RendererDedicatedWorkerHostInner {
    fn drop(&mut self) {
        self.retire();
    }
}

/// Exact physical producer, minted before script execution. Its JS parent
/// does not own native facts or the Inspector output sequence.
#[derive(Clone, Debug)]
pub(crate) struct RendererDedicatedWorkerHost(Arc<RendererDedicatedWorkerHostInner>);

#[derive(Clone, Debug)]
pub(crate) struct RendererDedicatedWorkerNetworkObserver(Weak<RendererDedicatedWorkerHostInner>);

impl RendererDedicatedWorkerNetworkObserver {
    pub(crate) fn publish(&self, observation: crate::runtime::RendererNetworkObservation) {
        if let Some(host) = self.0.upgrade() {
            RendererDedicatedWorkerHost(host)
                .publish(RendererProtocolObservation::Network(observation));
        }
    }
}

impl PartialEq for RendererDedicatedWorkerHost {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for RendererDedicatedWorkerHost {}

impl RendererDedicatedWorkerHost {
    pub(crate) fn network_observer(&self) -> RendererDedicatedWorkerNetworkObserver {
        RendererDedicatedWorkerNetworkObserver(Arc::downgrade(&self.0))
    }

    pub(crate) fn instance_id(&self) -> u64 {
        self.0.info.instance_id
    }
    pub(crate) fn name(&self) -> &str {
        &self.0.info.name
    }
    pub(crate) fn network(&self) -> &RendererWorkerNetworkReporter {
        &self.0.network
    }
    pub(crate) fn pause_on_start(&self) -> bool {
        self.0.registry.pause_on_start.load(Ordering::Acquire)
    }

    pub(crate) fn bind_execution(&self, worker: WorkerDevToolsHandle) {
        let mut execution = self.0.execution.lock();
        match &*execution {
            DedicatedWorkerExecution::Loading => {
                *execution = DedicatedWorkerExecution::Running(worker)
            }
            DedicatedWorkerExecution::Retired(_) => {
                let _ = worker.terminate_for_devtools();
            }
            DedicatedWorkerExecution::Running(_) => panic!("one physical Worker binds once"),
        }
    }

    pub(crate) fn lifecycle(&self, lifecycle: RendererWorkerLifecycle) {
        let _publication = self.0.publication.lock();
        let retired = matches!(
            *self.0.execution.lock(),
            DedicatedWorkerExecution::Retired(_)
        );
        if !retired {
            self.0.publish(RendererProtocolObservation::WorkerLifecycle(
                self.0.registry.outputs.worker_lifecycle.report(lifecycle),
            ));
        }
    }

    pub(crate) fn script_completed(&self, script: RendererDedicatedWorkerMainScript) {
        self.lifecycle(RendererWorkerLifecycle::DedicatedScriptCompleted {
            instance_id: self.instance_id(),
            script: Arc::new(script),
        });
    }

    pub(crate) fn publish(&self, observation: RendererProtocolObservation) {
        let _publication = self.0.publication.lock();
        let retired = matches!(
            *self.0.execution.lock(),
            DedicatedWorkerExecution::Retired(_)
        );
        if !retired {
            self.0.publish(observation);
        }
    }

    pub(crate) fn publish_response(&self, response: RendererRuntimeInspectorResponsePublication) {
        let _publication = self.0.publication.lock();
        let execution = self.0.execution.lock();
        let predecessor = match &*execution {
            DedicatedWorkerExecution::Loading | DedicatedWorkerExecution::Running(_) => self
                .0
                .output
                .last_published_cursor()
                .map(|cursor| self.0.output.declare_fence(cursor)),
            DedicatedWorkerExecution::Retired(terminal) => terminal.clone(),
        };
        let _ = response.commit(predecessor);
    }

    pub(crate) fn retire(&self) {
        self.0.retire();
    }

    fn inspection_endpoint(&self) -> Option<RendererWorkerInspectionEndpoint> {
        let execution = self.0.execution.lock();
        let DedicatedWorkerExecution::Running(worker) = &*execution else {
            return None;
        };
        Some(RendererWorkerInspectionEndpoint::new(
            worker.clone(),
            Some(self.0.output.clone()),
            "DedicatedWorkerRuntimeUnavailable",
        ))
    }
}

impl RendererBrowserContextRuntime {
    pub fn set_dedicated_worker_pause_on_start_for_devtools(&self, pause: bool) {
        self.inner
            .dedicated_workers
            .pause_on_start
            .store(pause, Ordering::Release);
    }

    pub(super) fn dedicated_worker_inspection_endpoint(
        &self,
        instance: u64,
    ) -> Option<RendererWorkerInspectionEndpoint> {
        self.inner
            .dedicated_workers
            .host(instance)?
            .inspection_endpoint()
    }

    pub fn run_dedicated_worker_if_waiting_for_debugger_for_devtools(&self, instance: u64) -> bool {
        self.inner.dedicated_workers.host(instance).is_some_and(|host| {
            let execution = host.0.execution.lock();
            matches!(&*execution, DedicatedWorkerExecution::Running(worker) if worker.run_if_waiting_for_debugger())
        })
    }

    pub fn close_dedicated_worker_for_devtools(&self, instance: u64) -> bool {
        self.inner.dedicated_workers.host(instance).is_some_and(|host| {
            let execution = host.0.execution.lock();
            matches!(&*execution, DedicatedWorkerExecution::Running(worker) if worker.terminate_for_devtools())
        })
    }
}
