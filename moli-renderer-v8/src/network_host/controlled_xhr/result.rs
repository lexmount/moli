//! Buffered XHR loading has one native result and one network terminal fact.
//! Protocol output and Page/Worker diagnostics project that same fact.

use moli_fetch::{FetchCancelHandle, Request, Response};

use crate::{
    network::loads::ResourceLoadLease,
    types::{PendingSubresourceFetchInfo, SubresourceNetworkRecord, SubresourceResponseBody},
};

/// The JS boundary needs a materialized Response, while Network can retain
/// the original raw-byte capture. Keeping both here avoids copying the whole
/// body back out of the Response or creating another disk-pool capture.
pub(super) struct BufferedXhrResponse {
    pub(super) response: Response,
    pub(super) body: SubresourceResponseBody,
}

impl BufferedXhrResponse {
    fn from_response(response: Response) -> Self {
        let body = SubresourceResponseBody::from_fetch_response(&response);
        Self { response, body }
    }
}

#[derive(Debug)]
pub(crate) struct XhrLoadResult {
    pub(crate) result: Result<Response, String>,
    pub(crate) info: PendingSubresourceFetchInfo,
    pub(crate) network_record: SubresourceNetworkRecord,
}

impl XhrLoadResult {
    pub(super) fn new(
        info: PendingSubresourceFetchInfo,
        result: Result<BufferedXhrResponse, String>,
        network_headers: Option<Vec<(String, String)>>,
    ) -> Self {
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
            info,
            result: result.map(|delivered| delivered.response),
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
                XhrLoadResult::new(info, result.map(BufferedXhrResponse::from_response), None)
            }
        };
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
