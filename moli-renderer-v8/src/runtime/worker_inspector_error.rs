/// Native Worker Inspector lifecycle failures. Protocol adapters may recover
/// subscription commands from an unavailable worker without inspecting text.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum WorkerInspectorError {
    #[error("SharedWorkerRuntimeUnavailable")]
    SharedUnavailable,
    #[error("DedicatedWorkerRuntimeUnavailable")]
    DedicatedUnavailable,
    #[error("ServiceWorkerRuntimeUnavailable")]
    ServiceUnavailable,
    #[error("Worker Inspector session has been detached")]
    Detached,
    #[error("Worker Inspector task runner is disposed")]
    Disposed,
    #[error("Worker environment owner is unavailable")]
    EnvironmentUnavailable,
    #[error("Worker Inspector owner is unavailable")]
    OwnerUnavailable,
    #[error("Worker closed before Inspector task dispatch")]
    Closed,
    #[error("Worker terminated before Inspector task dispatch")]
    Terminated,
    #[error("Worker global installation failed")]
    InstallationFailed,
    #[error("Worker exited before Inspector task dispatch")]
    Exited,
    #[error("WorkerRuntimeInspectorResponseCanceled")]
    ResponseCanceled,
}
