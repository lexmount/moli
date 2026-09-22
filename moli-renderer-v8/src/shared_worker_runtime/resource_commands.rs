use crate::{
    RendererSyntheticResponseBody,
    worker::{WorkerMessage, WorkerPendingFetchContinue, WorkerPendingXhrContinue},
};

use super::host::RendererSharedWorkerHost;

impl RendererSharedWorkerHost {
    pub(super) fn continue_pending_fetch(&self, request: WorkerPendingFetchContinue) -> bool {
        self.send_worker_message(WorkerMessage::ContinuePendingFetch(request))
    }

    pub(super) fn continue_pending_xhr(&self, request: WorkerPendingXhrContinue) -> bool {
        self.send_worker_message(WorkerMessage::ContinuePendingXhr(request))
    }

    pub(super) fn continue_pending_csp_report(&self, request: WorkerPendingFetchContinue) -> bool {
        self.send_worker_message(WorkerMessage::ContinuePendingCspReport(request))
    }

    pub(super) fn continue_pending_fetch_response(
        &self,
        request: WorkerPendingFetchContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
    ) -> bool {
        self.send_worker_message(WorkerMessage::ContinuePendingFetchResponse {
            request,
            response_code,
            response_headers,
        })
    }

    pub(super) fn continue_pending_xhr_response(
        &self,
        request: WorkerPendingXhrContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
    ) -> bool {
        self.send_worker_message(WorkerMessage::ContinuePendingXhrResponse {
            request,
            response_code,
            response_headers,
        })
    }

    pub(super) fn fail_pending_fetch(
        &self,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.send_worker_message(WorkerMessage::FailPendingFetch {
            request,
            error_text,
        })
    }

    pub(super) fn fail_pending_xhr(
        &self,
        request: WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        self.send_worker_message(WorkerMessage::FailPendingXhr {
            request,
            error_text,
        })
    }

    pub(super) fn fail_pending_csp_report(
        &self,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.send_worker_message(WorkerMessage::FailPendingCspReport {
            request,
            error_text,
        })
    }

    pub(super) fn fail_pending_fetch_auth(
        &self,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.send_worker_message(WorkerMessage::FailPendingFetchAuth {
            request,
            error_text,
        })
    }

    pub(super) fn fail_pending_xhr_auth(
        &self,
        request: WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        self.send_worker_message(WorkerMessage::FailPendingXhrAuth {
            request,
            error_text,
        })
    }

    pub(super) fn fail_pending_fetch_response(
        &self,
        request: WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        self.send_worker_message(WorkerMessage::FailPendingFetchResponse {
            request,
            error_text,
        })
    }

    pub(super) fn fail_pending_xhr_response(
        &self,
        request: WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        self.send_worker_message(WorkerMessage::FailPendingXhrResponse {
            request,
            error_text,
        })
    }

    pub(super) fn fulfill_pending_fetch(
        &self,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.send_worker_message(WorkerMessage::FulfillPendingFetch {
            request,
            response_code,
            response_headers,
            response_body,
        })
    }

    pub(super) fn fulfill_pending_xhr(
        &self,
        request: WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.send_worker_message(WorkerMessage::FulfillPendingXhr {
            request,
            response_code,
            response_headers,
            response_body,
        })
    }

    pub(super) fn fulfill_pending_csp_report(
        &self,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.send_worker_message(WorkerMessage::FulfillPendingCspReport {
            request,
            response_code,
            response_headers,
            response_body,
        })
    }

    pub(super) fn fulfill_pending_fetch_response(
        &self,
        request: WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.send_worker_message(WorkerMessage::FulfillPendingFetchResponse {
            request,
            response_code,
            response_headers,
            response_body,
        })
    }

    pub(super) fn fulfill_pending_xhr_response(
        &self,
        request: WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        self.send_worker_message(WorkerMessage::FulfillPendingXhrResponse {
            request,
            response_code,
            response_headers,
            response_body,
        })
    }
}
