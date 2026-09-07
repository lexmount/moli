use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy)]
pub(crate) enum ServiceWorkerRuntimeOwnerWake {
    ServiceLane,
}

pub(crate) type ServiceWorkerRuntimeOwnerWakeSender =
    mpsc::UnboundedSender<ServiceWorkerRuntimeOwnerWake>;

pub(crate) fn service_worker_owner_wake_channel() -> (
    ServiceWorkerRuntimeOwnerWakeSender,
    mpsc::UnboundedReceiver<ServiceWorkerRuntimeOwnerWake>,
) {
    mpsc::unbounded_channel()
}
