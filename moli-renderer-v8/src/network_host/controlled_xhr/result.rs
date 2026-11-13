//! Buffered XHR loading has one native result and one network terminal fact.
//! Protocol output and Page/Worker diagnostics project that same fact.

use moli_fetch::{FetchCancelHandle, Request, Response};

use crate::{
    network::loads::ResourceLoadLease,
    types::{PendingSubresourceFetchInfo, SubresourceNetworkRecord, SubresourceResponseBody},
};

#[derive(Debug)]
pub(crate) struct XhrLoadResult {
    pub(crate) result: Result<Response, String>,
    pub(crate) info: PendingSubresourceFetchInfo,
    pub(crate) network_record: SubresourceNetworkRecord,
}

impl XhrLoadResult {
    pub(super) fn new(
        info: PendingSubresourceFetchInfo,
        result: Result<Response, String>,
        network_headers: Option<Vec<(String, String)>>,
    ) -> Self {
        let record = match &result {
            Ok(response) => {
                let mut record = SubresourceNetworkRecord::success_with_body(
                    info.frame_id.clone(),
                    info.document_url.clone(),
                    info.url.clone(),
                    info.method.clone(),
                    info.request_headers.clone(),
                    info.request_body.clone(),
                    info.resource_type,
                    response
                        .request_cookie_report
                        .clone()
                        .or_else(|| info.request_cookie_report.clone()),
                    response
                        .redirect_chain
                        .clone()
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    response.final_url.clone(),
                    response.status,
                    response.headers.clone(),
                    SubresourceResponseBody::from_fetch_response(response),
                    response.cookie_set_reports.clone(),
                )
                .with_from_cache(response.from_cache)
                .with_negotiated_http_version(response.negotiated_http_version)
                .with_network_request_headers(network_headers);
                if let Some(status_text) = &response.status_text {
                    record = record.with_response_status_text(status_text);
                }
                record
            }
            Err(error) => SubresourceNetworkRecord::failure(
                info.frame_id.clone(),
                info.document_url.clone(),
                info.url.clone(),
                info.method.clone(),
                info.request_headers.clone(),
                info.request_body.clone(),
                info.resource_type,
                if crate::network_host::is_cors_policy_failure_message(error) {
                    crate::network_host::FAILED_ERROR_TEXT.to_owned()
                } else {
                    error.clone()
                },
            ),
        }
        .with_request_body_bytes(info.request_body_bytes.clone());
        let network_record = match info.network_request_handle {
            Some(handle) => record.with_request_handle(handle),
            None => record,
        };
        Self {
            info,
            result,
            network_record,
        }
    }
}

/// Starts a buffered consumer on the resource executor. Window and Worker keep
/// their own cancellation/deadline waiters and apply the result on their owner.
pub(crate) fn start_buffered_xhr(
    request: Request,
    mut info: PendingSubresourceFetchInfo,
    load: ResourceLoadLease,
    cancel: FetchCancelHandle,
    preflight_headers: Vec<(String, String)>,
    control: Option<super::ControlledXhrContext>,
) -> crossbeam_channel::Receiver<Result<XhrLoadResult, String>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    load.task_runner().spawn(async move {
        let completed = match control {
            Some(control) => {
                control
                    .load(request, info, load, cancel, preflight_headers)
                    .run()
                    .await
            }
            None => {
                // Ordinary synchronous loads have no native protocol request.
                info.network_request_handle = None;
                let origin = request
                    .request_origin()
                    .cloned()
                    .expect("prepared XHR carries its requesting origin");
                let credentials = request.credentials_mode;
                let result = crate::network_host::fetch_browser_subresource_with_preflight_headers(
                    load.request_client(),
                    request,
                    Some(cancel),
                    preflight_headers,
                )
                .await
                .and_then(|response| {
                    crate::network_host::validate_cors_response_chain(
                        &origin,
                        &response.head(),
                        credentials,
                    )?;
                    Ok(response)
                });
                XhrLoadResult::new(info, result, None)
            }
        };
        let _ = tx.send(Ok(completed));
    });
    rx
}
