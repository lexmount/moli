use moli_shared_worker::SharedWorkerInstanceId;

use crate::{
    RendererSyntheticResponseBody,
    worker::{WorkerPendingFetchContinue, WorkerPendingXhrContinue},
};

use super::{host::RendererSharedWorkerHost, service::SharedWorkerRuntimeService};

enum SharedWorkerResourceCommand {
    ContinueFetch {
        request: WorkerPendingFetchContinue,
    },
    ContinueXhr {
        request: WorkerPendingXhrContinue,
    },
    ContinueCspReport {
        request: WorkerPendingFetchContinue,
    },
    ContinueFetchResponse {
        request: WorkerPendingFetchContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
    },
    ContinueXhrResponse {
        request: WorkerPendingXhrContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
    },
    FailFetch {
        request: WorkerPendingFetchContinue,
        error_text: String,
    },
    FailXhr {
        request: WorkerPendingXhrContinue,
        error_text: String,
    },
    FailCspReport {
        request: WorkerPendingFetchContinue,
        error_text: String,
    },
    FailFetchAuth {
        request: WorkerPendingFetchContinue,
        error_text: String,
    },
    FailXhrAuth {
        request: WorkerPendingXhrContinue,
        error_text: String,
    },
    FailFetchResponse {
        request: WorkerPendingFetchContinue,
        error_text: String,
    },
    FailXhrResponse {
        request: WorkerPendingXhrContinue,
        error_text: String,
    },
    FulfillFetch {
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    },
    FulfillXhr {
        request: WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    },
    FulfillCspReport {
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    },
    FulfillFetchResponse {
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    },
    FulfillXhrResponse {
        request: WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    },
}

impl SharedWorkerResourceCommand {
    fn dispatch(self, host: &RendererSharedWorkerHost) -> bool {
        match self {
            Self::ContinueFetch { request } => host.continue_pending_fetch(request),
            Self::ContinueXhr { request } => host.continue_pending_xhr(request),
            Self::ContinueCspReport { request } => host.continue_pending_csp_report(request),
            Self::ContinueFetchResponse {
                request,
                response_code,
                response_headers,
            } => host.continue_pending_fetch_response(request, response_code, response_headers),
            Self::ContinueXhrResponse {
                request,
                response_code,
                response_headers,
            } => host.continue_pending_xhr_response(request, response_code, response_headers),
            Self::FailFetch {
                request,
                error_text,
            } => host.fail_pending_fetch(request, error_text),
            Self::FailXhr {
                request,
                error_text,
            } => host.fail_pending_xhr(request, error_text),
            Self::FailCspReport {
                request,
                error_text,
            } => host.fail_pending_csp_report(request, error_text),
            Self::FailFetchAuth {
                request,
                error_text,
            } => host.fail_pending_fetch_auth(request, error_text),
            Self::FailXhrAuth {
                request,
                error_text,
            } => host.fail_pending_xhr_auth(request, error_text),
            Self::FailFetchResponse {
                request,
                error_text,
            } => host.fail_pending_fetch_response(request, error_text),
            Self::FailXhrResponse {
                request,
                error_text,
            } => host.fail_pending_xhr_response(request, error_text),
            Self::FulfillFetch {
                request,
                response_code,
                response_headers,
                response_body,
            } => {
                host.fulfill_pending_fetch(request, response_code, response_headers, response_body)
            }
            Self::FulfillXhr {
                request,
                response_code,
                response_headers,
                response_body,
            } => host.fulfill_pending_xhr(request, response_code, response_headers, response_body),
            Self::FulfillCspReport {
                request,
                response_code,
                response_headers,
                response_body,
            } => host.fulfill_pending_csp_report(
                request,
                response_code,
                response_headers,
                response_body,
            ),
            Self::FulfillFetchResponse {
                request,
                response_code,
                response_headers,
                response_body,
            } => host.fulfill_pending_fetch_response(
                request,
                response_code,
                response_headers,
                response_body,
            ),
            Self::FulfillXhrResponse {
                request,
                response_code,
                response_headers,
                response_body,
            } => host.fulfill_pending_xhr_response(
                request,
                response_code,
                response_headers,
                response_body,
            ),
        }
    }
}

impl SharedWorkerRuntimeService {
    fn dispatch_resource_command(
        &self,
        instance_id: SharedWorkerInstanceId,
        command: SharedWorkerResourceCommand,
    ) -> bool {
        self.route_running_host(instance_id, |host| command.dispatch(host))
    }

    pub(crate) fn continue_pending_fetch(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::ContinueFetch { request },
        )
    }

    pub(crate) fn continue_pending_xhr(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingXhrContinue,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::ContinueXhr { request },
        )
    }

    pub(crate) fn continue_pending_csp_report(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::ContinueCspReport { request },
        )
    }

    pub(crate) fn continue_pending_fetch_response(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::ContinueFetchResponse {
                request,
                response_code,
                response_headers,
            },
        )
    }

    pub(crate) fn continue_pending_xhr_response(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingXhrContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::ContinueXhrResponse {
                request,
                response_code,
                response_headers,
            },
        )
    }

    pub(crate) fn fail_pending_fetch(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::FailFetch {
                request,
                error_text,
            },
        )
    }

    pub(crate) fn fail_pending_xhr(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::FailXhr {
                request,
                error_text,
            },
        )
    }

    pub(crate) fn fail_pending_csp_report(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::FailCspReport {
                request,
                error_text,
            },
        )
    }

    pub(crate) fn fail_pending_fetch_auth(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::FailFetchAuth {
                request,
                error_text,
            },
        )
    }

    pub(crate) fn fail_pending_xhr_auth(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::FailXhrAuth {
                request,
                error_text,
            },
        )
    }

    pub(crate) fn fail_pending_fetch_response(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::FailFetchResponse {
                request,
                error_text,
            },
        )
    }

    pub(crate) fn fail_pending_xhr_response(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::FailXhrResponse {
                request,
                error_text,
            },
        )
    }

    pub(crate) fn fulfill_pending_fetch(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::FulfillFetch {
                request,
                response_code,
                response_headers,
                response_body,
            },
        )
    }

    pub(crate) fn fulfill_pending_xhr(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::FulfillXhr {
                request,
                response_code,
                response_headers,
                response_body,
            },
        )
    }

    pub(crate) fn fulfill_pending_csp_report(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::FulfillCspReport {
                request,
                response_code,
                response_headers,
                response_body,
            },
        )
    }

    pub(crate) fn fulfill_pending_fetch_response(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::FulfillFetchResponse {
                request,
                response_code,
                response_headers,
                response_body,
            },
        )
    }

    pub(crate) fn fulfill_pending_xhr_response(
        &self,
        instance_id: SharedWorkerInstanceId,
        request: WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.dispatch_resource_command(
            instance_id,
            SharedWorkerResourceCommand::FulfillXhrResponse {
                request,
                response_code,
                response_headers,
                response_body,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use moli_shared_worker::{SharedWorkerConnectAction, SharedWorkerDescriptor};
    use tokio::sync::mpsc;
    use url::Url;

    use crate::worker::WorkerMessage;

    use super::{
        super::{host::RendererSharedWorkerHostState, test_support},
        *,
    };

    fn pending_fetch(fetch_id: u32) -> WorkerPendingFetchContinue {
        WorkerPendingFetchContinue {
            redirect_headers: None,
            fetch_id,
            internal_id: u64::from(fetch_id),
            network_request_handle: None,
            url: Url::parse("https://example.test/shared-worker-fetch").unwrap(),
            method: "GET".to_owned(),
            body: None,
            headers: Vec::new().into(),
            intercept_response: false,
            handle_auth_requests: false,
            auth: None,
        }
    }

    #[test]
    fn shared_worker_resource_commands_route_only_to_running_instance() {
        let service = test_support::runtime_service();
        let key = test_support::shared_worker_key();
        let action = test_support::connect_matching(
            &service,
            key.clone(),
            SharedWorkerDescriptor::default(),
        );
        let SharedWorkerConnectAction::StartLoading { instance_id, .. } = action else {
            panic!("first shared worker connection should start a loading instance");
        };

        assert!(
            !service.continue_pending_fetch(instance_id, pending_fetch(1)),
            "resource commands must not route to loading SharedWorker hosts"
        );

        let host = test_support::loading_host_with_runtime_service(instance_id, &key, &service);
        let (tx, mut rx) = mpsc::unbounded_channel();
        *host.state.lock() = RendererSharedWorkerHostState::Running {
            tx,
            handle: None,
            parent_rx: None,
        };
        let ready =
            test_support::finish_loading_matching(&service, &key, instance_id, host.clone());
        assert!(
            matches!(
                ready,
                moli_shared_worker::SharedWorkerLoadReady::Running { .. }
            ),
            "shared worker matching store should publish the running host"
        );

        assert!(service.continue_pending_fetch(instance_id, pending_fetch(7)));
        match rx
            .try_recv()
            .expect("running host should receive fetch command")
        {
            WorkerMessage::ContinuePendingFetch(request) => {
                assert_eq!(request.fetch_id, 7);
                assert_eq!(request.internal_id, 7);
            }
            other => panic!("expected ContinuePendingFetch, got {other:?}"),
        }

        assert!(
            !service.continue_pending_fetch(
                SharedWorkerInstanceId::from_u64(instance_id.as_u64() + 1),
                pending_fetch(8),
            ),
            "resource commands must fail closed for stale or missing SharedWorker instances"
        );
        assert!(
            rx.try_recv().is_err(),
            "stale SharedWorker resource command must not be delivered to the live host"
        );
    }
}
