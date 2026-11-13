//! Buffered XHR loading has one native result and one network terminal fact.
//! Protocol output and Page/Worker diagnostics project that same fact.

use moli_fetch::{FetchCancelHandle, Request, Response, ResponseHead};

use crate::{
    network::loads::ResourceLoadLease,
    types::{PendingSubresourceFetchInfo, SubresourceNetworkRecord, SubresourceResponseBody},
};

/// The JS boundary needs a materialized Response, while Network can retain
/// the original raw-byte capture. Keeping both here avoids copying the whole
/// body back out of the Response or creating another disk-pool capture.
pub(super) struct BufferedXhrResponse {
    pub(super) response_filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
    pub(super) body_is_null: Option<bool>,
    pub(super) skip_fetch_security_validation: bool,
    pub(super) response: Response,
    pub(super) body: SubresourceResponseBody,
}

impl BufferedXhrResponse {
    fn from_response(response: Response) -> Self {
        let body = SubresourceResponseBody::from_fetch_response(&response);
        Self {
            response,
            body,
            response_filter: None,
            body_is_null: None,
            skip_fetch_security_validation: false,
        }
    }
}

/// Headers selected after redirect and security checks survive a body-read
/// failure. Resource Timing still reports that real response and native clock.
pub(super) struct SelectedXhrResponseHead {
    pub(super) head: ResponseHead,
    pub(super) response_filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
    pub(super) skip_fetch_security_validation: bool,
}

#[derive(Debug)]
pub(crate) struct XhrLoadResult {
    pub(crate) response_head: Option<ResponseHead>,
    pub(crate) response_filter: Option<crate::types::AsyncSubresourceFetchResponseFilter>,
    pub(crate) body_is_null: Option<bool>,
    pub(crate) skip_fetch_security_validation: bool,

    pub(crate) result: Result<Response, String>,
    pub(crate) info: PendingSubresourceFetchInfo,
    pub(crate) network_record: SubresourceNetworkRecord,
    pub(crate) csp_violations:
        Vec<crate::content_security_policy::ContentSecurityPolicyUrlViolation>,
}

impl XhrLoadResult {
    pub(super) fn new(
        info: PendingSubresourceFetchInfo,
        result: Result<BufferedXhrResponse, String>,
        network_headers: Option<Vec<(String, String)>>,
    ) -> Self {
        let response_filter = result
            .as_ref()
            .ok()
            .and_then(|response| response.response_filter.clone());
        let body_is_null = result
            .as_ref()
            .ok()
            .and_then(|response| response.body_is_null);
        let skip_fetch_security_validation = result
            .as_ref()
            .is_ok_and(|response| response.skip_fetch_security_validation);
        let record = match &result {
            Ok(delivered) => {
                let response = &delivered.response;
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
                    delivered.body.clone(),
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
            response_head: result
                .as_ref()
                .ok()
                .map(|delivered| delivered.response.head()),
            info,
            result: result.map(|delivered| delivered.response),
            response_filter,
            body_is_null,
            skip_fetch_security_validation,
            network_record,
            csp_violations: Vec::new(),
        }
    }
}

pub(crate) struct BufferedXhrContext {
    pub(crate) control: Option<super::ControlledXhrContext>,
    pub(crate) service_worker: Option<super::XhrServiceWorkerContext>,
}

/// Starts a buffered consumer on the resource executor. Window and Worker keep
/// their own cancellation/deadline waiters and apply the result on their owner.
pub(crate) fn start_buffered_xhr(
    request: Request,
    mut info: PendingSubresourceFetchInfo,
    load: ResourceLoadLease,
    cancel: FetchCancelHandle,
    preflight_headers: Vec<(String, String)>,
    context: BufferedXhrContext,
    on_completed: impl FnOnce(&XhrLoadResult) + Send + 'static,
) -> crossbeam_channel::Receiver<Result<XhrLoadResult, String>> {
    let BufferedXhrContext {
        control,
        service_worker,
    } = context;
    let (tx, rx) = crossbeam_channel::bounded(1);
    load.task_runner().spawn(async move {
        let completed = match control {
            Some(control) => {
                control
                    .load(request, info, load, cancel, preflight_headers)
                    .with_service_worker(service_worker)
                    .run()
                    .await
            }
            None if service_worker.is_some() => {
                // An ordinary request has no protocol pause/output capability.
                info.network_request_handle = None;
                let worker = service_worker.expect("matched the controlled ordinary route");
                super::ControlledResourceLoad {
                    response_policy: super::ResourceResponsePolicy::Xhr,
                    redirect_owner: None,
                    blob_url_entry: super::super::CapturedBlobUrl::capture(&request.url),
                    worker_service_worker: None,
                    response_filter: None,
                    selected_response_head: None,
                    race_response: None,
                    csp_source_location: Default::default(),
                    csp_violations: Vec::new(),
                    request,
                    info,
                    load,
                    cancel,
                    preflight_headers,
                    connect_policy: Some(worker.connect_policy.clone()),
                    interception: None,
                    service_worker: Some(worker),
                    request_initiator_type: crate::types::SubresourceRequestInitiatorType::Script,
                    initial_decision: None,
                }
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
                        load.request_client().web_security_disabled(),
                    )?;
                    Ok(response)
                });
                XhrLoadResult::new(info, result.map(BufferedXhrResponse::from_response), None)
            }
        };
        on_completed(&completed);
        let _ = tx.send(Ok(completed));
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        network::ResourceInterceptionBody,
        types::{
            SubresourceNetworkOutcome, SubresourceResourceType, SubresourceResponseBodyWriter,
        },
    };

    #[tokio::test]
    async fn buffered_xhr_network_record_reuses_original_raw_capture() {
        for disk_backed in [false, true] {
            let pool = moli_disk_pool::DiskPool::new(None).unwrap();
            let payload = [0, 0xff, 0xc3, 0x28, 0x80, b'x', b'y', b'z'].repeat(if disk_backed {
                256 * 1024
            } else {
                512
            });
            let mut writer = SubresourceResponseBodyWriter::with_memory_limit_and_disk_pool(
                if disk_backed {
                    64 * 1024
                } else {
                    payload.len() + 1
                },
                Some(pool.clone()),
            );
            writer.append(&payload);
            let captured = writer.finish();
            let retained_bytes = captured.retained_size_bytes();
            let address = (!disk_backed).then(|| captured.try_bytes().unwrap().as_ptr() as usize);
            let before = pool.diagnostics();
            let live_disk = before.disk_footprint_bytes - before.free_bytes as u64;
            assert_eq!(live_disk > 0, disk_backed);

            let url = url::Url::parse("https://xhr.test/body").unwrap();
            let info = PendingSubresourceFetchInfo {
                internal_id: 1,
                network_request_handle: None,
                frame_id: None,
                document_url: url.clone(),
                url: url.clone(),
                websocket_socket_id: None,
                method: "GET".into(),
                request_headers: Default::default(),
                request_body: None,
                request_body_bytes: None,
                resource_type: SubresourceResourceType::Xhr,
                request_cookie_report: None,
            };
            let loaded = super::super::LoadedResponse {
                response_filter: None,
                body_is_null: None,
                source: moli_fetch::RedirectSource::Network,
                head: moli_fetch::ResponseHead {
                    final_url: url,
                    status: 200,
                    status_text: Some("OK".into()),
                    headers: vec![("content-type".into(), b"application/octet-stream".to_vec())],
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    cache_state: Default::default(),
                    negotiated_http_version: None,
                    request_include_credentials: false,
                    preload_state: Default::default(),
                },
                body: ResourceInterceptionBody::from_complete(captured),
            };
            let delivered = loaded.deliver(&info, None, None).await.unwrap().unwrap();
            let completed = XhrLoadResult::new(info, Ok(delivered), None);
            drop(loaded);

            assert_eq!(completed.result.as_ref().unwrap().body_bytes(), payload);
            let SubresourceNetworkOutcome::Success { response_body, .. } =
                completed.network_record.outcome()
            else {
                panic!("buffered delivery must construct a success record")
            };
            assert_eq!(response_body.retained_size_bytes(), retained_bytes);
            assert_eq!(response_body.materialize_bytes().unwrap(), payload);
            if let Some(address) = address {
                assert_eq!(
                    response_body.try_bytes().unwrap().as_ptr() as usize,
                    address,
                    "the Network record must share the original allocation"
                );
            }
            let retained = pool.diagnostics();
            assert_eq!(
                retained.disk_footprint_bytes, before.disk_footprint_bytes,
                "Network delivery must not allocate a second disk capture"
            );
            assert_eq!(
                retained.disk_footprint_bytes - retained.free_bytes as u64,
                live_disk,
                "the record retains the original disk capture after its producer drops"
            );
            drop(completed.network_record);
            let released = pool.diagnostics();
            assert_eq!(released.disk_footprint_bytes, released.free_bytes as u64);
            assert_eq!(completed.result.unwrap().body_bytes(), payload);
        }
    }
}
