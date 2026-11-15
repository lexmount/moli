use super::*;
use crate::{RendererPageCommand as C, RendererSyntheticResponseBody};
impl WorkerTestHandle {
    pub(crate) async fn native_response_body(
        &self,
        id: u64,
    ) -> crate::types::SubresourceResponseBody {
        let body = self
            .native_pauses
            .lock()
            .get(&id)
            .unwrap()
            .response_body()
            .unwrap()
            .clone();
        body.capture(64 * 1024 * 1024).await.unwrap()
    }

    fn decide(&self, id: u64, command: C) {
        self.native_pauses
            .lock()
            .get(&id)
            .expect("a test decision requires its exact native pause")
            .dispatch(command)
            .expect("native test decision");
    }
    pub(crate) fn continue_pending_fetch(&self, r: crate::worker::WorkerPendingFetchContinue) {
        let id = r.fetch_id;
        let command = match r.auth {
            Some(auth) => C::ContinuePendingSubresourceAuth {
                internal_id: id,
                auth,
            },
            None => C::ContinuePendingSubresourceFetch {
                internal_id: id,
                url: Some(r.url),
                method: Some(r.method),
                body: Some(r.body),
                headers: Some(r.headers.into()),
                intercept_response: r.intercept_response,
                handle_auth_requests: r.handle_auth_requests,
            },
        };
        self.decide(id, command);
    }
    pub(crate) fn continue_pending_xhr(&self, r: crate::worker::WorkerPendingXhrContinue) {
        let id = r.xhr_id;
        let command = match r.auth {
            Some(auth) => C::ContinuePendingSubresourceAuth {
                internal_id: id,
                auth,
            },
            None => C::ContinuePendingSubresourceFetch {
                internal_id: id,
                url: Some(r.url),
                method: Some(r.method),
                body: Some(r.body),
                headers: Some(r.headers.into()),
                intercept_response: r.intercept_response,
                handle_auth_requests: r.handle_auth_requests,
            },
        };
        self.decide(id, command);
    }
    pub(crate) fn fulfill_pending_fetch(
        &self,
        r: crate::worker::WorkerPendingFetchContinue,
        status: u16,
        headers: Vec<(String, Vec<u8>)>,
        body: RendererSyntheticResponseBody,
    ) {
        self.fulfill(r.fetch_id, status, headers, body);
    }
    pub(crate) fn fulfill_pending_xhr(
        &self,
        r: crate::worker::WorkerPendingXhrContinue,
        status: u16,
        headers: Vec<(String, Vec<u8>)>,
        body: RendererSyntheticResponseBody,
    ) {
        self.fulfill(r.xhr_id, status, headers, body);
    }
    fn fulfill(
        &self,
        id: u64,
        status: u16,
        headers: Vec<(String, Vec<u8>)>,
        body: RendererSyntheticResponseBody,
    ) {
        self.decide(
            id,
            C::FulfillPendingSubresourceFetch {
                internal_id: id,
                request_overrides: None,
                response_code: status,
                response_headers: headers,
                response_body: body,
            },
        );
    }

    pub(crate) fn fail_pending_xhr(
        &self,
        r: crate::worker::WorkerPendingXhrContinue,
        error_text: String,
    ) {
        self.decide(
            r.xhr_id,
            C::FailPendingSubresourceFetch {
                internal_id: r.xhr_id,
                error_text,
            },
        );
    }
    pub(crate) fn fail_pending_fetch_auth(
        &self,
        r: crate::worker::WorkerPendingFetchContinue,
        error_text: String,
    ) {
        self.decide(
            r.fetch_id,
            C::FailPendingSubresourceAuth {
                internal_id: r.fetch_id,
                error_text,
            },
        );
    }
    pub(crate) fn fail_pending_xhr_auth(
        &self,
        r: crate::worker::WorkerPendingXhrContinue,
        error_text: String,
    ) {
        self.decide(
            r.xhr_id,
            C::FailPendingSubresourceAuth {
                internal_id: r.xhr_id,
                error_text,
            },
        );
    }
    pub(crate) fn continue_pending_fetch_response(
        &self,
        r: crate::worker::WorkerPendingFetchContinue,
        status: Option<u16>,
        headers: Option<Vec<(String, Vec<u8>)>>,
    ) {
        self.decide(
            r.fetch_id,
            C::ContinuePendingSubresourceResponse {
                internal_id: r.fetch_id,
                response_code: status,
                response_headers: headers,
            },
        );
    }
    pub(crate) fn continue_pending_xhr_response(
        &self,
        r: crate::worker::WorkerPendingXhrContinue,
        status: Option<u16>,
        headers: Option<Vec<(String, Vec<u8>)>>,
    ) {
        self.decide(
            r.xhr_id,
            C::ContinuePendingSubresourceResponse {
                internal_id: r.xhr_id,
                response_code: status,
                response_headers: headers,
            },
        );
    }
}
