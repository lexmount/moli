use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy)]
pub(crate) enum SharedWorkerRuntimeOwnerWake {
    ServiceLane,
}

pub(crate) type SharedWorkerRuntimeOwnerWakeSender =
    mpsc::UnboundedSender<SharedWorkerRuntimeOwnerWake>;

pub(crate) fn shared_worker_owner_wake_channel() -> (
    SharedWorkerRuntimeOwnerWakeSender,
    mpsc::UnboundedReceiver<SharedWorkerRuntimeOwnerWake>,
) {
    mpsc::unbounded_channel()
}
