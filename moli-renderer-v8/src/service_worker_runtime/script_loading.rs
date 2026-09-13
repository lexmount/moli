use std::time::Instant;

use moli_crypto::sha256_hex;
use moli_fetch::{
    FetchCancelHandle, Request, RequestCacheMode, RequestCredentialsMode, ResponseHead,
    ScriptFetchRequestMetadata,
};
use url::Url;

use crate::content_security_policy::ContentSecurityPolicyReportingEndpoints;
use crate::network::ResourceRequestClient;
use crate::worker::WorkerScriptResourceKind;

use super::{
    jobs::ServiceWorkerLaunchParams,
    path_restriction::{
        service_worker_allowed_header_value, verify_service_worker_script_path_restriction,
    },
};

#[derive(Clone)]
pub(super) struct ServiceWorkerScriptLoadParams {
    pub(super) script_url: Url,
    pub(super) scope_url: Url,
    pub(super) document_url: Url,
    pub(super) request_client: ResourceRequestClient,
    pub(super) cache_mode: RequestCacheMode,
}

impl ServiceWorkerScriptLoadParams {
    pub(super) fn from_launch_params(params: &ServiceWorkerLaunchParams) -> Self {
        Self {
            script_url: params.script_url.clone(),
            scope_url: params.scope_url.clone(),
            document_url: params.document_url.clone(),
            request_client: params.request_client.clone(),
            cache_mode: RequestCacheMode::Default,
        }
    }
}

#[derive(Clone)]
pub(super) struct ServiceWorkerScriptUpdateCheckParams {
    pub(super) owner: super::run_owner::ServiceWorkerRunOwner,
    pub(super) main_script: ServiceWorkerScriptLoadParams,
    pub(super) newest_main_body_sha256: String,
    pub(super) imported_scripts: Vec<ServiceWorkerScriptResource>,
    pub(super) imported_script_cache_mode: RequestCacheMode,
    pub(super) skip_script_comparison: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ServiceWorkerScriptResource {
    pub(super) request_url: Url,
    pub(super) final_url: Url,
    pub(super) kind: WorkerScriptResourceKind,
    pub(super) status: u16,
    pub(super) headers: Vec<(String, Vec<u8>)>,
    pub(super) body_len: usize,
    pub(super) body_sha256: String,
    pub(super) response_time_ms: u64,
    pub(super) mime_type: Option<String>,
}

impl ServiceWorkerScriptResource {
    fn from_response_parts(
        request_url: Url,
        head: &ResponseHead,
        body_bytes: &[u8],
        response_time_ms: u64,
    ) -> Self {
        let body_sha256 = sha256_hex(body_bytes);
        let mime_type = moli_web_mime::response_header_value(&head.headers, "content-type");
        Self {
            request_url,
            final_url: head.final_url.clone(),
            kind: WorkerScriptResourceKind::JavaScript,
            status: head.status,
            headers: head.headers.clone(),
            body_len: body_bytes.len(),
            body_sha256,
            response_time_ms,
            mime_type,
        }
    }

    pub(super) fn from_worker_script_resource(
        resource: crate::worker::WorkerScriptResource,
    ) -> Self {
        Self {
            request_url: resource.request_url,
            final_url: resource.final_url,
            kind: resource.kind,
            status: resource.status,
            headers: resource.headers,
            body_len: resource.body_len,
            body_sha256: resource.body_sha256,
            response_time_ms: resource.response_time_ms,
            mime_type: resource.mime_type,
        }
    }
}

pub(super) struct LoadedServiceWorkerScript {
    pub(super) resource: ServiceWorkerScriptResource,
    pub(super) source: String,
    pub(super) response_referrer_policy: Option<String>,
    pub(super) response_content_security_policies: Vec<String>,
    pub(super) response_content_security_report_only_policies: Vec<String>,
    pub(super) response_content_security_reporting_endpoints:
        ContentSecurityPolicyReportingEndpoints,
}

pub(super) struct ServiceWorkerScriptUpdateCheckResult {
    pub(super) main_script: LoadedServiceWorkerScript,
    pub(super) change: ServiceWorkerScriptUpdateCheckChange,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ServiceWorkerScriptUpdateCheckChange {
    Identical,
    ScriptComparisonSkipped,
    MainScriptDifferent,
    ImportedScriptDifferent { script_url: Url },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ServiceWorkerScriptUpdateCheckFailureStatus {
    ScriptLoadFailed,
    Stale,
}

impl ServiceWorkerScriptUpdateCheckFailureStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ScriptLoadFailed => "script-load-failed",
            Self::Stale => "stale",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ServiceWorkerScriptUpdateCheckFailure {
    pub(super) status: ServiceWorkerScriptUpdateCheckFailureStatus,
    pub(super) message: String,
}

impl ServiceWorkerScriptUpdateCheckFailure {
    pub(super) fn script_load(message: String) -> Self {
        Self {
            status: ServiceWorkerScriptUpdateCheckFailureStatus::ScriptLoadFailed,
            message,
        }
    }

    pub(super) fn stale(message: String) -> Self {
        Self {
            status: ServiceWorkerScriptUpdateCheckFailureStatus::Stale,
            message,
        }
    }
}

pub(super) type ServiceWorkerScriptUpdateCheckCompletion =
    Result<ServiceWorkerScriptUpdateCheckResult, ServiceWorkerScriptUpdateCheckFailure>;

pub(super) struct ServiceWorkerScriptLoader {
    source: crate::runtime::RendererWorkerNetworkReporter,
    observer: crate::worker::WorkerNetworkObserver,
    cancel: FetchCancelHandle,
}

impl ServiceWorkerScriptLoader {
    pub(super) fn new(
        source: crate::runtime::RendererWorkerNetworkReporter,
        observer: crate::worker::WorkerNetworkObserver,
        cancel: FetchCancelHandle,
    ) -> Self {
        Self {
            source,
            observer,
            cancel,
        }
    }

    async fn fetch_script(
        &self,
        client: &ResourceRequestClient,
        url: &Url,
        initiator: &Url,
        cache_mode: RequestCacheMode,
        network: &crate::worker::WorkerResourceTransfer,
    ) -> Result<(moli_fetch::Response, u64), String> {
        let started = Instant::now();
        let request = Request::new("GET", url.as_str(), None, vec![])
            .map_err(|error| {
                let message = error.to_string();
                network.failed(&crate::network::ResourceResponseFailure::Request(
                    message.clone(),
                ));
                message
            })?
            .with_credentials_mode(RequestCredentialsMode::SameOrigin)
            .with_cache_mode(cache_mode)
            .with_page_network_policy()
            .with_initiator_url(initiator)
            .with_request_origin(moli_url::WebOrigin::from_url(initiator))
            .with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
        let response = client
            .fetch_observed_script_text_with_cancel(request, self.cancel.clone(), network)
            .await
            .map_err(|error| {
                network.failed(&error);
                format!("Failed to load service worker script `{url}`: {error}")
            })?;
        Ok((
            response,
            started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        ))
    }

    pub(super) async fn load_main_script(
        &self,
        params: &ServiceWorkerScriptLoadParams,
    ) -> Result<LoadedServiceWorkerScript, String> {
        let mut request_url = params.script_url.clone();
        request_url.set_fragment(None);
        let network = crate::worker::WorkerResourceTransfer::start_main_script(
            &self.source,
            self.observer.clone(),
            &request_url,
            &params.document_url,
        )
        .ok_or_else(|| "ServiceWorker script loading stopped".to_owned())?;
        let (response, response_time_ms) = self
            .fetch_script(
                &params.request_client,
                &request_url,
                &params.document_url,
                params.cache_mode,
                &network,
            )
            .await?;
        network.materialize_script_response(response, |response| {
            crate::worker::ensure_worker_script_redirect_chain_same_origin(
                &params.document_url,
                &response.redirect_chain,
                &response.final_url,
            )
            .map_err(|message| {
                format!("Failed to load service worker script `{request_url}`: {message}")
            })?;
            moli_fetch::ensure_http_status_success(
                response.final_url.as_str(),
                response.status,
                false,
            )
            .map_err(|error| error.to_string())?;
            crate::worker::ensure_worker_script_mime_acceptable(
                &response.final_url,
                &response.headers,
                response.body_bytes(),
            )?;
            let service_worker_allowed_header =
                service_worker_allowed_header_value(&response.headers);
            verify_service_worker_script_path_restriction(
                &params.scope_url,
                &response.final_url,
                service_worker_allowed_header.as_deref(),
            )?;
            let response_referrer_policy =
                crate::referrer_policy::response_referrer_policy_from_headers(&response.headers);
            let response_content_security_policies =
                crate::content_security_policy::content_security_policy_headers(&response.headers);
            let response_content_security_report_only_policies =
                crate::content_security_policy::content_security_policy_report_only_headers(
                    &response.headers,
                );
            let response_content_security_reporting_endpoints =
        crate::content_security_policy::content_security_policy_reporting_endpoints_from_headers(
            &response.headers,
            &response.final_url,
        );
            let (head, body, body_bytes) = response.into_parts();
            let mut resource = ServiceWorkerScriptResource::from_response_parts(
                request_url,
                &head,
                &body_bytes,
                response_time_ms,
            );
            let mut final_url = head.final_url;
            final_url.set_fragment(params.script_url.fragment());
            resource.final_url = final_url;
            Ok(LoadedServiceWorkerScript {
                resource,
                source: body,
                response_referrer_policy,
                response_content_security_policies,
                response_content_security_report_only_policies,
                response_content_security_reporting_endpoints,
            })
        })
    }

    pub(super) async fn check_update(
        &self,
        params: &ServiceWorkerScriptUpdateCheckParams,
    ) -> ServiceWorkerScriptUpdateCheckCompletion {
        let main_script = self
            .load_main_script(&params.main_script)
            .await
            .map_err(ServiceWorkerScriptUpdateCheckFailure::script_load)?;
        if params.skip_script_comparison {
            return Ok(ServiceWorkerScriptUpdateCheckResult {
                main_script,
                change: ServiceWorkerScriptUpdateCheckChange::ScriptComparisonSkipped,
            });
        }
        if main_script.resource.body_sha256 != params.newest_main_body_sha256 {
            return Ok(ServiceWorkerScriptUpdateCheckResult {
                main_script,
                change: ServiceWorkerScriptUpdateCheckChange::MainScriptDifferent,
            });
        }
        let request_client = &params.main_script.request_client;
        for imported_script in &params.imported_scripts {
            let result = self
                .load_imported_script(
                    request_client,
                    &imported_script.request_url,
                    &main_script.resource.final_url,
                    params.imported_script_cache_mode,
                    imported_script.kind,
                )
                .await;
            match result {
                Ok(updated_resource)
                    if updated_resource.body_sha256 != imported_script.body_sha256 =>
                {
                    return Ok(ServiceWorkerScriptUpdateCheckResult {
                        main_script,
                        change: ServiceWorkerScriptUpdateCheckChange::ImportedScriptDifferent {
                            script_url: imported_script.request_url.clone(),
                        },
                    });
                }
                Ok(_) => {}
                Err(message) if self.cancel.is_cancelled() => {
                    return Err(ServiceWorkerScriptUpdateCheckFailure::script_load(message));
                }
                Err(message) => {
                    tracing::debug!(
                        script_url = %imported_script.request_url,
                        error = %message,
                        "ignored service worker imported script update check failure"
                    );
                }
            }
        }
        Ok(ServiceWorkerScriptUpdateCheckResult {
            main_script,
            change: ServiceWorkerScriptUpdateCheckChange::Identical,
        })
    }

    async fn load_imported_script(
        &self,
        request_client: &ResourceRequestClient,
        request_url: &Url,
        initiator_url: &Url,
        cache_mode: RequestCacheMode,
        kind: WorkerScriptResourceKind,
    ) -> Result<ServiceWorkerScriptResource, String> {
        let mut request_url_without_fragment = request_url.clone();
        request_url_without_fragment.set_fragment(None);
        let network = crate::worker::WorkerResourceTransfer::start_script(
            &self.source,
            self.observer.clone(),
            &request_url_without_fragment,
            initiator_url,
        )
        .ok_or_else(|| "ServiceWorker script loading stopped".to_owned())?;
        let (response, response_time_ms) = self
            .fetch_script(
                request_client,
                &request_url_without_fragment,
                initiator_url,
                cache_mode,
                &network,
            )
            .await?;
        network.materialize_script_response(response, |response| {
    crate::worker::ensure_worker_script_redirect_chain_same_origin(
        initiator_url,
        &response.redirect_chain,
        &response.final_url,
    )
    .map_err(|message| {
        format!(
            "Failed to load service worker imported script `{request_url_without_fragment}`: {message}"
        )
    })?;
    moli_fetch::ensure_http_status_success(response.final_url.as_str(), response.status, false)
        .map_err(|error| error.to_string())?;
    ensure_imported_script_resource_mime(kind, &response)?;
    let (head, body_bytes) = response.into_byte_parts();
    let mut resource = ServiceWorkerScriptResource::from_response_parts(
        request_url_without_fragment,
        &head,
        &body_bytes,
        response_time_ms,
    );
    resource.kind = kind;
    let mut final_url = head.final_url;
    final_url.set_fragment(request_url.fragment());
    resource.final_url = final_url;
    Ok(resource)
        })
    }
}

fn ensure_imported_script_resource_mime(
    kind: WorkerScriptResourceKind,
    response: &moli_fetch::Response,
) -> Result<(), String> {
    match kind {
        WorkerScriptResourceKind::JavaScript => {
            crate::worker::ensure_worker_script_mime_acceptable(
                &response.final_url,
                &response.headers,
                response.body_bytes(),
            )
        }
        WorkerScriptResourceKind::CssModule => {
            crate::worker::ensure_worker_css_module_mime(response)
        }
        WorkerScriptResourceKind::JsonModule => {
            crate::worker::ensure_worker_json_module_mime(response)
        }
        WorkerScriptResourceKind::TextModule => {
            crate::worker::ensure_worker_text_module_mime(response)
        }
        WorkerScriptResourceKind::WebAssemblyModule => {
            crate::worker::ensure_worker_wasm_module_mime(response)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use moli_crypto::sha256_hex;
    use moli_fetch::{FetchConfig, RequestCacheMode, ResponseHead};
    use url::Url;

    use crate::network::ResourceRequestClient;
    use crate::worker::WorkerScriptResourceKind;

    use super::{
        ServiceWorkerScriptLoadParams, ServiceWorkerScriptLoader, ServiceWorkerScriptResource,
        ServiceWorkerScriptUpdateCheckChange, ServiceWorkerScriptUpdateCheckParams,
    };

    #[test]
    fn script_resource_records_response_metadata_and_body_hash() {
        let request_url = Url::parse("https://example.test/app/sw.js").unwrap();
        let head = ResponseHead {
            final_url: Url::parse("https://example.test/app/sw.js?final").unwrap(),
            status: 200,
            headers: vec![("Content-Type".to_owned(), b"text/javascript".to_vec())],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        };

        let resource =
            ServiceWorkerScriptResource::from_response_parts(request_url.clone(), &head, b"abc", 7);

        assert_eq!(resource.request_url, request_url);
        assert_eq!(resource.final_url, head.final_url);
        assert_eq!(resource.status, 200);
        assert_eq!(resource.body_len, 3);
        assert_eq!(resource.kind, WorkerScriptResourceKind::JavaScript);
        assert_eq!(
            resource.body_sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(resource.response_time_ms, 7);
        assert_eq!(resource.mime_type.as_deref(), Some("text/javascript"));
    }

    #[tokio::test]
    async fn update_check_reports_changed_imported_script_resource() {
        let main_body = "importScripts('./dep.js');";
        let (base_url, server) = spawn_script_response_server(vec![
            (
                "/app/sw.js",
                "HTTP/1.1 200 OK",
                "application/javascript",
                main_body.to_owned(),
            ),
            (
                "/app/dep.js",
                "HTTP/1.1 200 OK",
                "application/javascript",
                "globalThis.dep = 2;".to_owned(),
            ),
        ]);
        let request_client_owner =
            ResourceRequestClient::new(&FetchConfig::default()).expect("test request client");
        let request_client = request_client_owner.handle();
        let script_url = Url::parse(&format!("{base_url}/app/sw.js")).unwrap();
        let dep_url = Url::parse(&format!("{base_url}/app/dep.js")).unwrap();

        let (loader, owner) = test_loader();
        let result = loader
            .check_update(&ServiceWorkerScriptUpdateCheckParams {
                owner,
                main_script: ServiceWorkerScriptLoadParams {
                    script_url: script_url.clone(),
                    scope_url: Url::parse(&format!("{base_url}/app/")).unwrap(),
                    document_url: Url::parse(&format!("{base_url}/app/page.html")).unwrap(),
                    request_client,
                    cache_mode: RequestCacheMode::Validate,
                },
                newest_main_body_sha256: hash_body(main_body.as_bytes()),
                imported_scripts: vec![script_resource_with_body(&dep_url, b"globalThis.dep = 1;")],
                imported_script_cache_mode: RequestCacheMode::Default,
                skip_script_comparison: false,
            })
            .await
            .expect("update check should load main script");

        assert_eq!(result.main_script.resource.request_url, script_url);
        assert_eq!(
            result.change,
            ServiceWorkerScriptUpdateCheckChange::ImportedScriptDifferent {
                script_url: dep_url
            }
        );
        server.join().expect("script response server should finish");
    }

    #[tokio::test]
    async fn update_check_can_skip_script_comparison_after_main_script_load() {
        let main_body = "importScripts('./dep.js');";
        let (base_url, server) = spawn_script_response_server(vec![(
            "/app/sw.js",
            "HTTP/1.1 200 OK",
            "application/javascript",
            main_body.to_owned(),
        )]);
        let request_client_owner =
            ResourceRequestClient::new(&FetchConfig::default()).expect("test request client");
        let request_client = request_client_owner.handle();
        let script_url = Url::parse(&format!("{base_url}/app/sw.js")).unwrap();
        let dep_url = Url::parse(&format!("{base_url}/app/dep.js")).unwrap();

        let (loader, owner) = test_loader();
        let result = loader
            .check_update(&ServiceWorkerScriptUpdateCheckParams {
                owner,
                main_script: ServiceWorkerScriptLoadParams {
                    script_url: script_url.clone(),
                    scope_url: Url::parse(&format!("{base_url}/app/")).unwrap(),
                    document_url: Url::parse(&format!("{base_url}/app/page.html")).unwrap(),
                    request_client,
                    cache_mode: RequestCacheMode::Validate,
                },
                newest_main_body_sha256: hash_body(main_body.as_bytes()),
                imported_scripts: vec![script_resource_with_body(&dep_url, b"globalThis.dep = 1;")],
                imported_script_cache_mode: RequestCacheMode::Default,
                skip_script_comparison: true,
            })
            .await
            .expect("update check should load main script");

        assert_eq!(result.main_script.resource.request_url, script_url);
        assert_eq!(
            result.change,
            ServiceWorkerScriptUpdateCheckChange::ScriptComparisonSkipped
        );
        server.join().expect("script response server should finish");
    }

    #[tokio::test]
    async fn update_check_reports_changed_json_module_resource() {
        let main_body = r#"import data from "./dep.json" with { type: "json" };"#;
        let (base_url, server) = spawn_script_response_server(vec![
            (
                "/app/sw.js",
                "HTTP/1.1 200 OK",
                "application/javascript",
                main_body.to_owned(),
            ),
            (
                "/app/dep.json",
                "HTTP/1.1 200 OK",
                "application/manifest+json; charset=utf-8",
                r#"{"dep":2}"#.to_owned(),
            ),
        ]);
        let request_client_owner =
            ResourceRequestClient::new(&FetchConfig::default()).expect("test request client");
        let request_client = request_client_owner.handle();
        let script_url = Url::parse(&format!("{base_url}/app/sw.js")).unwrap();
        let dep_url = Url::parse(&format!("{base_url}/app/dep.json")).unwrap();

        let (loader, owner) = test_loader();
        let result = loader
            .check_update(&ServiceWorkerScriptUpdateCheckParams {
                owner,
                main_script: ServiceWorkerScriptLoadParams {
                    script_url,
                    scope_url: Url::parse(&format!("{base_url}/app/")).unwrap(),
                    document_url: Url::parse(&format!("{base_url}/app/page.html")).unwrap(),
                    request_client,
                    cache_mode: RequestCacheMode::Validate,
                },
                newest_main_body_sha256: hash_body(main_body.as_bytes()),
                imported_scripts: vec![script_resource_with_body_mime_and_kind(
                    &dep_url,
                    b"{\"dep\":1}",
                    "application/json",
                    WorkerScriptResourceKind::JsonModule,
                )],
                imported_script_cache_mode: RequestCacheMode::Default,
                skip_script_comparison: false,
            })
            .await
            .expect("update check should accept JSON module MIME");

        assert_eq!(
            result.change,
            ServiceWorkerScriptUpdateCheckChange::ImportedScriptDifferent {
                script_url: dep_url
            }
        );
        server.join().expect("script response server should finish");
    }

    #[tokio::test]
    async fn update_check_reports_changed_css_module_resource() {
        let main_body = r#"import sheet from "./dep.css" with { type: "css" };"#;
        let (base_url, server) = spawn_script_response_server(vec![
            (
                "/app/sw.js",
                "HTTP/1.1 200 OK",
                "application/javascript",
                main_body.to_owned(),
            ),
            (
                "/app/dep.css",
                "HTTP/1.1 200 OK",
                "text/css; charset=utf-8",
                ".answer { color: blue; }".to_owned(),
            ),
        ]);
        let request_client_owner =
            ResourceRequestClient::new(&FetchConfig::default()).expect("test request client");
        let request_client = request_client_owner.handle();
        let script_url = Url::parse(&format!("{base_url}/app/sw.js")).unwrap();
        let dep_url = Url::parse(&format!("{base_url}/app/dep.css")).unwrap();

        let (loader, owner) = test_loader();
        let result = loader
            .check_update(&ServiceWorkerScriptUpdateCheckParams {
                owner,
                main_script: ServiceWorkerScriptLoadParams {
                    script_url,
                    scope_url: Url::parse(&format!("{base_url}/app/")).unwrap(),
                    document_url: Url::parse(&format!("{base_url}/app/page.html")).unwrap(),
                    request_client,
                    cache_mode: RequestCacheMode::Validate,
                },
                newest_main_body_sha256: hash_body(main_body.as_bytes()),
                imported_scripts: vec![script_resource_with_body_mime_and_kind(
                    &dep_url,
                    b".answer { color: green; }",
                    "text/css",
                    WorkerScriptResourceKind::CssModule,
                )],
                imported_script_cache_mode: RequestCacheMode::Default,
                skip_script_comparison: false,
            })
            .await
            .expect("update check should accept CSS module MIME");

        assert_eq!(
            result.change,
            ServiceWorkerScriptUpdateCheckChange::ImportedScriptDifferent {
                script_url: dep_url
            }
        );
        server.join().expect("script response server should finish");
    }

    #[tokio::test]
    async fn update_check_reports_changed_text_module_resource() {
        let main_body = r#"import text from "./dep.txt" with { type: "text" };"#;
        let (base_url, server) = spawn_script_response_server(vec![
            (
                "/app/sw.js",
                "HTTP/1.1 200 OK",
                "application/javascript",
                main_body.to_owned(),
            ),
            (
                "/app/dep.txt",
                "HTTP/1.1 200 OK",
                "text/plain; charset=utf-8",
                "updated text".to_owned(),
            ),
        ]);
        let request_client_owner =
            ResourceRequestClient::new(&FetchConfig::default()).expect("test request client");
        let request_client = request_client_owner.handle();
        let script_url = Url::parse(&format!("{base_url}/app/sw.js")).unwrap();
        let dep_url = Url::parse(&format!("{base_url}/app/dep.txt")).unwrap();

        let (loader, owner) = test_loader();
        let result = loader
            .check_update(&ServiceWorkerScriptUpdateCheckParams {
                owner,
                main_script: ServiceWorkerScriptLoadParams {
                    script_url,
                    scope_url: Url::parse(&format!("{base_url}/app/")).unwrap(),
                    document_url: Url::parse(&format!("{base_url}/app/page.html")).unwrap(),
                    request_client,
                    cache_mode: RequestCacheMode::Validate,
                },
                newest_main_body_sha256: hash_body(main_body.as_bytes()),
                imported_scripts: vec![script_resource_with_body_mime_and_kind(
                    &dep_url,
                    b"old text",
                    "text/plain",
                    WorkerScriptResourceKind::TextModule,
                )],
                imported_script_cache_mode: RequestCacheMode::Default,
                skip_script_comparison: false,
            })
            .await
            .expect("update check should accept text module MIME");

        assert_eq!(
            result.change,
            ServiceWorkerScriptUpdateCheckChange::ImportedScriptDifferent {
                script_url: dep_url
            }
        );
        server.join().expect("script response server should finish");
    }

    #[tokio::test]
    async fn update_check_reports_changed_wasm_module_resource() {
        let main_body = r#"import source wasmSource from "./dep.wasm";"#;
        let (base_url, server) = spawn_script_response_server(vec![
            (
                "/app/sw.js",
                "HTTP/1.1 200 OK",
                "application/javascript",
                main_body.to_owned(),
            ),
            (
                "/app/dep.wasm",
                "HTTP/1.1 200 OK",
                "application/wasm",
                "wasm-v2".to_owned(),
            ),
        ]);
        let request_client_owner =
            ResourceRequestClient::new(&FetchConfig::default()).expect("test request client");
        let request_client = request_client_owner.handle();
        let script_url = Url::parse(&format!("{base_url}/app/sw.js")).unwrap();
        let dep_url = Url::parse(&format!("{base_url}/app/dep.wasm")).unwrap();

        let (loader, owner) = test_loader();
        let result = loader
            .check_update(&ServiceWorkerScriptUpdateCheckParams {
                owner,
                main_script: ServiceWorkerScriptLoadParams {
                    script_url,
                    scope_url: Url::parse(&format!("{base_url}/app/")).unwrap(),
                    document_url: Url::parse(&format!("{base_url}/app/page.html")).unwrap(),
                    request_client,
                    cache_mode: RequestCacheMode::Validate,
                },
                newest_main_body_sha256: hash_body(main_body.as_bytes()),
                imported_scripts: vec![script_resource_with_body_mime_and_kind(
                    &dep_url,
                    b"wasm-v1",
                    "application/wasm",
                    WorkerScriptResourceKind::WebAssemblyModule,
                )],
                imported_script_cache_mode: RequestCacheMode::Default,
                skip_script_comparison: false,
            })
            .await
            .expect("update check should accept Wasm module MIME");

        assert_eq!(
            result.change,
            ServiceWorkerScriptUpdateCheckChange::ImportedScriptDifferent {
                script_url: dep_url
            }
        );
        server.join().expect("script response server should finish");
    }

    #[tokio::test]
    async fn update_check_ignores_imported_script_load_failure() {
        let main_body = "importScripts('./dep.js');";
        let (base_url, server) = spawn_script_response_server(vec![
            (
                "/app/sw.js",
                "HTTP/1.1 200 OK",
                "application/javascript",
                main_body.to_owned(),
            ),
            (
                "/app/dep.js",
                "HTTP/1.1 404 Not Found",
                "text/plain",
                "missing".to_owned(),
            ),
        ]);
        let request_client_owner =
            ResourceRequestClient::new(&FetchConfig::default()).expect("test request client");
        let request_client = request_client_owner.handle();
        let dep_url = Url::parse(&format!("{base_url}/app/dep.js")).unwrap();

        let (loader, owner) = test_loader();
        let result = loader
            .check_update(&ServiceWorkerScriptUpdateCheckParams {
                owner,
                main_script: ServiceWorkerScriptLoadParams {
                    script_url: Url::parse(&format!("{base_url}/app/sw.js")).unwrap(),
                    scope_url: Url::parse(&format!("{base_url}/app/")).unwrap(),
                    document_url: Url::parse(&format!("{base_url}/app/page.html")).unwrap(),
                    request_client,
                    cache_mode: RequestCacheMode::Validate,
                },
                newest_main_body_sha256: hash_body(main_body.as_bytes()),
                imported_scripts: vec![script_resource_with_body(&dep_url, b"globalThis.dep = 1;")],
                imported_script_cache_mode: RequestCacheMode::Default,
                skip_script_comparison: false,
            })
            .await
            .expect("imported script failure should not fail the update check");

        assert_eq!(
            result.change,
            ServiceWorkerScriptUpdateCheckChange::Identical
        );
        server.join().expect("script response server should finish");
    }

    #[tokio::test]
    async fn update_check_cancellation_retains_partial_import_without_reporting_identical() {
        use crate::runtime::RendererNetworkOutputItem;
        use crate::worker::{WorkerNetworkObserver, WorkerToParentMessage};
        use moli_page_types::{ScriptNetworkOutputItem, SubresourceBodyFinishedResult};

        let main_body = "importScripts('./dep.js');";
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (release, held) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            for (path, body) in [("/app/sw.js", main_body), ("/app/dep.js", "//partial")] {
                let (mut stream, _) = listener.accept().unwrap();
                assert!(read_http_request_head(&mut stream).starts_with(&format!("GET {path} ")));
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                if path.ends_with("dep.js") {
                    stream.write_all(&body.as_bytes()[..2]).unwrap();
                    held.recv().unwrap();
                } else {
                    stream.write_all(body.as_bytes()).unwrap();
                }
            }
        });
        let client_owner = ResourceRequestClient::new(&FetchConfig::default()).unwrap();
        let (mut loader, owner) = test_loader();
        let cancel = loader.cancel.clone();
        let (sender, mut output) = tokio::sync::mpsc::unbounded_channel();
        loader.observer = WorkerNetworkObserver::Channel(sender.downgrade());
        let params = ServiceWorkerScriptUpdateCheckParams {
            owner,
            main_script: ServiceWorkerScriptLoadParams {
                script_url: Url::parse(&format!("{origin}/app/sw.js")).unwrap(),
                scope_url: Url::parse(&format!("{origin}/app/")).unwrap(),
                document_url: Url::parse(&format!("{origin}/app/page.html")).unwrap(),
                request_client: client_owner.handle(),
                cache_mode: RequestCacheMode::Validate,
            },
            newest_main_body_sha256: hash_body(main_body.as_bytes()),
            imported_scripts: vec![script_resource_with_body(
                &Url::parse(&format!("{origin}/app/dep.js")).unwrap(),
                b"//old",
            )],
            imported_script_cache_mode: RequestCacheMode::Default,
            skip_script_comparison: false,
        };
        let task = tokio::spawn(async move { loader.check_update(&params).await });
        let dependency = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut dependency = None;
            loop {
                let WorkerToParentMessage::Network(observation) = output.recv().await.unwrap()
                else {
                    unreachable!()
                };
                let RendererNetworkOutputItem::Resource(item) = observation.item() else {
                    unreachable!()
                };
                match item.as_ref() {
                    ScriptNetworkOutputItem::SubresourceRequestStarted(start)
                        if start.url().path().ends_with("dep.js") =>
                    {
                        assert!(!start.is_worker_main_script());
                        dependency = Some(start.handle());
                    }
                    ScriptNetworkOutputItem::SubresourceDataReceived(data)
                        if Some(data.handle()) == dependency =>
                    {
                        return data.handle();
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("the import check must stream before its tail is released");
        cancel.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .expect("cancellation settles while the server tail remains held")
            .unwrap();
        let Err(failure) = result else {
            panic!("cancellation cannot be ignored as an identical update");
        };
        assert_eq!(
            failure.status,
            super::ServiceWorkerScriptUpdateCheckFailureStatus::ScriptLoadFailed
        );
        let mut terminals = 0;
        while let Ok(WorkerToParentMessage::Network(observation)) = output.try_recv() {
            if let RendererNetworkOutputItem::Resource(item) = observation.item()
                && let ScriptNetworkOutputItem::SubresourceBodyFinished(body) = item.as_ref()
                && body.handle() == dependency
            {
                let SubresourceBodyFinishedResult::FailedWithPartialBody { partial_body, .. } =
                    body.result()
                else {
                    panic!("retain the real failed import response")
                };
                assert_eq!(partial_body.clone_body_bytes(), b"//");
                terminals += 1;
            }
        }
        assert_eq!(terminals, 1);
        release.send(()).unwrap();
        server.join().unwrap();
    }

    fn test_loader() -> (
        ServiceWorkerScriptLoader,
        super::super::run_owner::ServiceWorkerRunOwner,
    ) {
        use crate::runtime::{
            RendererBrowserContextRuntimeId, RendererNetworkReporter, RendererWorkerIdentity,
            RendererWorkerNetworkReporter,
        };
        let owner = super::super::run_owner::ServiceWorkerRunOwner::fresh(
            super::super::ids::ServiceWorkerVersionId(1),
        );
        let source = RendererWorkerNetworkReporter::new(
            RendererNetworkReporter::new(RendererBrowserContextRuntimeId::new_for_testing(0)),
            RendererWorkerIdentity::Service {
                version: owner.version_id().as_u64(),
                run: owner.cloned_run_identity(),
            },
        );
        let (sender, _) = tokio::sync::mpsc::unbounded_channel();
        (
            ServiceWorkerScriptLoader::new(
                source,
                crate::worker::WorkerNetworkObserver::Channel(sender.downgrade()),
                moli_fetch::FetchCancelHandle::new(),
            ),
            owner,
        )
    }

    fn script_resource_with_body(script_url: &Url, body: &[u8]) -> ServiceWorkerScriptResource {
        script_resource_with_body_mime_and_kind(
            script_url,
            body,
            "application/javascript",
            WorkerScriptResourceKind::JavaScript,
        )
    }

    fn script_resource_with_body_mime_and_kind(
        script_url: &Url,
        body: &[u8],
        mime_type: &str,
        kind: WorkerScriptResourceKind,
    ) -> ServiceWorkerScriptResource {
        let head = ResponseHead {
            final_url: script_url.clone(),
            status: 200,
            headers: vec![("Content-Type".to_owned(), mime_type.as_bytes().to_vec())],
            request_cookie_report: None,
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        };
        let mut resource =
            ServiceWorkerScriptResource::from_response_parts(script_url.clone(), &head, body, 0);
        resource.kind = kind;
        resource
    }

    fn hash_body(body: &[u8]) -> String {
        sha256_hex(body)
    }

    fn spawn_script_response_server(
        responses: Vec<(&'static str, &'static str, &'static str, String)>,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind script response server");
        let addr = listener.local_addr().expect("script response server addr");
        let server = thread::spawn(move || {
            let mut responses = responses
                .into_iter()
                .map(|(path, status_line, content_type, body)| {
                    (
                        path.to_owned(),
                        (status_line.to_owned(), content_type.to_owned(), body),
                    )
                })
                .collect::<HashMap<_, _>>();
            while !responses.is_empty() {
                let (mut stream, _) = listener.accept().expect("accept script request");
                let request = read_http_request_head(&mut stream);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .expect("script request path");
                let (status_line, content_type, body) = responses
                    .remove(path)
                    .unwrap_or_else(|| panic!("unexpected script path: {path}"));
                let response = format!(
                    "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write script response");
            }
        });
        (format!("http://{addr}"), server)
    }

    fn read_http_request_head(stream: &mut std::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1];
        while !bytes.ends_with(b"\r\n\r\n") {
            let read = stream.read(&mut buffer).expect("read script request");
            assert!(read != 0, "connection closed before request headers");
            bytes.extend_from_slice(&buffer[..read]);
        }
        String::from_utf8(bytes).expect("request should be UTF-8")
    }
}
