use super::*;

/// Where a finished WebCrypto blocking task routes its completion.
///
/// `Page` settles through the renderer owner queue (window/iframe runtimes);
/// `Worker` settles through the dedicated/shared worker event loop. Both lanes
/// snapshot inputs at the V8 boundary, run the primitive on the blocking pool,
/// and reuse the same `WebCryptoTaskResult` / rejection mapping, so the
/// renderer-visible promise outcome is identical regardless of lane.
pub(crate) enum WebCryptoCompletionSink {
    Page(crate::page_task_queue::RendererPageWebCryptoTaskProducer),
    Worker {
        task_id: u64,
        tx: tokio::sync::mpsc::UnboundedSender<crate::worker::WorkerWebCryptoCompletion>,
    },
}

impl WebCryptoCompletionSink {
    /// Deliver exactly one result through the owner captured at registration.
    pub(crate) fn send(self, result: Result<WebCryptoTaskResult, WebCryptoRejection>) {
        match self {
            WebCryptoCompletionSink::Page(producer) => {
                let _ = producer.send(result);
            }
            WebCryptoCompletionSink::Worker { task_id, tx } => {
                let _ = tx.send(crate::worker::WorkerWebCryptoCompletion { task_id, result });
            }
        }
    }
}

pub(crate) fn register_webcrypto_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    promise: &PendingCryptoPromise<'s>,
) -> Option<(tokio::runtime::Handle, WebCryptoCompletionSink)> {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return None;
    };
    // Page runtimes expose a JsContextHost on the global bridge and settle
    // through the renderer owner queue. Worker globals do not, so fall back to
    // the worker-owned completion lane routed through the worker event loop.
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        // SAFETY: the global bridge stores the current realm's live
        // JsContextHost. Registration runs synchronously inside the V8
        // callback before control returns to the owner. The returned producer
        // captures the exact PageVm, Window realm, and never-reused task id.
        let producer =
            unsafe { &mut *host_ptr }.register_pending_webcrypto_task(scope, promise.resolver())?;
        return Some((handle, WebCryptoCompletionSink::Page(producer)));
    }
    let (task_id, completion_tx) =
        crate::worker::register_worker_webcrypto_task(scope, promise.resolver())?;
    Some((
        handle,
        WebCryptoCompletionSink::Worker {
            task_id,
            tx: completion_tx,
        },
    ))
}

pub(crate) fn spawn_webcrypto_bytes_task<F>(
    handle: tokio::runtime::Handle,
    completion_tx: WebCryptoCompletionSink,
    operation: F,
) where
    F: FnOnce() -> Result<Vec<u8>, WebCryptoError> + Send + 'static,
{
    // Callers must finish all JS-observable normalization before this point;
    // the blocking closure may only touch snapshotted bytes and enum params.
    handle.spawn_blocking(move || {
        let result = operation()
            .map(WebCryptoTaskResult::Bytes)
            .map_err(WebCryptoRejection::from);
        completion_tx.send(result);
    });
}

pub(crate) fn spawn_webcrypto_key_task<F>(
    handle: tokio::runtime::Handle,
    completion_tx: WebCryptoCompletionSink,
    operation: F,
) where
    F: FnOnce() -> Result<CryptoKeyClonePayload, WebCryptoRejection> + Send + 'static,
{
    // The task receives a clone payload rather than a V8 handle so all
    // primitive work and key-material import stay off the renderer callback.
    handle.spawn_blocking(move || {
        let result = operation().map(|payload| WebCryptoTaskResult::CryptoKey(Box::new(payload)));
        completion_tx.send(result);
    });
}

pub(crate) fn spawn_webcrypto_key_pair_task<F>(
    handle: tokio::runtime::Handle,
    completion_tx: WebCryptoCompletionSink,
    operation: F,
) where
    F: FnOnce() -> Result<(CryptoKeyClonePayload, CryptoKeyClonePayload), WebCryptoError>
        + Send
        + 'static,
{
    handle.spawn_blocking(move || {
        let result = operation()
            .map(
                |(private_key, public_key)| WebCryptoTaskResult::CryptoKeyPair {
                    private_key: Box::new(private_key),
                    public_key: Box::new(public_key),
                },
            )
            .map_err(WebCryptoRejection::from);
        completion_tx.send(result);
    });
}

pub(crate) fn spawn_webcrypto_bool_task<F>(
    handle: tokio::runtime::Handle,
    completion_tx: WebCryptoCompletionSink,
    operation: F,
) where
    F: FnOnce() -> Result<bool, WebCryptoError> + Send + 'static,
{
    handle.spawn_blocking(move || {
        let result = operation()
            .map(WebCryptoTaskResult::Bool)
            .map_err(WebCryptoRejection::from);
        completion_tx.send(result);
    });
}

pub(crate) fn spawn_webcrypto_result_task<F>(
    handle: tokio::runtime::Handle,
    completion_tx: WebCryptoCompletionSink,
    operation: F,
) where
    F: FnOnce() -> Result<WebCryptoTaskResult, WebCryptoRejection> + Send + 'static,
{
    handle.spawn_blocking(move || {
        let result = operation();
        completion_tx.send(result);
    });
}
