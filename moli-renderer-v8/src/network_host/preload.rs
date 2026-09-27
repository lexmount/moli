use crate::{
    network::{
        RendererResourceTaskRunner, ResourceRequestClient,
        preload::{PreloadResponseProvenance, PreloadedResource},
    },
    page_task_queue::RendererResourceCompletionSender,
    types::{AsyncSubresourceFetchCompletion, AsyncSubresourceFetchResult},
};

pub(crate) fn try_spawn_preloaded_subresource_fetch(
    runner: RendererResourceTaskRunner,
    completion_tx: RendererResourceCompletionSender,
    loader: &ResourceRequestClient,
    request: &moli_fetch::Request,
    internal_id: u64,
) -> bool {
    let Some(preload) = loader.consume_preload(request) else {
        return false;
    };
    spawn_preloaded_subresource_fetch(runner, completion_tx, preload, request.clone(), internal_id);
    true
}

pub(crate) fn spawn_preloaded_subresource_fetch(
    runner: RendererResourceTaskRunner,
    completion_tx: RendererResourceCompletionSender,
    preload: PreloadedResource,
    request: moli_fetch::Request,
    internal_id: u64,
) {
    runner.spawn(async move {
        let terminal = preload.wait().await;
        let (result, response_filter, skip_fetch_security_validation, response_status_text) =
            match terminal.consumer_response() {
                Ok(response) => (
                    Ok(response.response.clone()),
                    response.filter,
                    matches!(
                        response.provenance,
                        PreloadResponseProvenance::ServiceWorker { .. }
                    ),
                    match &response.provenance {
                        PreloadResponseProvenance::ServiceWorker { status_text, .. } => {
                            status_text.clone()
                        }
                        PreloadResponseProvenance::Network => None,
                    },
                ),
                Err(message) => (Err(message), None, false, None),
            };
        let _ = completion_tx.send_async_subresource(AsyncSubresourceFetchCompletion {
            internal_id,
            request_url: request.url,
            request_method: request.method,
            request_headers: request.request_headers,
            request_body: None,
            response_status_text,
            skip_fetch_security_validation,
            response_filter,
            network_error_text: None,
            result: AsyncSubresourceFetchResult::Preloaded(result),
        });
    });
}
