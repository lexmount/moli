use super::*;
use crate::RendererSyntheticResponseBody;
use crate::network::interception_control::{InterceptionPhase, ResourceInterceptionDecision};

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

    fn decide(&self, id: u64, phase: InterceptionPhase, decision: ResourceInterceptionDecision) {
        self.native_pauses
            .lock()
            .get(&id)
            .expect("a test decision requires its exact native pause")
            .send_test_decision(id, phase, decision);
    }

    pub(crate) fn continue_pending_fetch(&self, r: crate::worker::WorkerPendingFetchContinue) {
        let (phase, decision) = match r.auth {
            Some(auth) => (
                InterceptionPhase::Auth,
                ResourceInterceptionDecision::Auth(auth),
            ),
            None => (
                InterceptionPhase::Request,
                ResourceInterceptionDecision::ContinueRequest {
                    url: Some(r.url),
                    method: Some(r.method),
                    body: Some(r.body),
                    headers: Some(r.headers.into()),
                    intercept_response: r.intercept_response,
                    handle_auth_requests: r.handle_auth_requests,
                },
            ),
        };
        self.decide(r.fetch_id, phase, decision);
    }

    pub(crate) fn continue_pending_xhr(&self, r: crate::worker::WorkerPendingXhrContinue) {
        let (phase, decision) = match r.auth {
            Some(auth) => (
                InterceptionPhase::Auth,
                ResourceInterceptionDecision::Auth(auth),
            ),
            None => (
                InterceptionPhase::Request,
                ResourceInterceptionDecision::ContinueRequest {
                    url: Some(r.url),
                    method: Some(r.method),
                    body: Some(r.body),
                    headers: Some(r.headers.into()),
                    intercept_response: r.intercept_response,
                    handle_auth_requests: r.handle_auth_requests,
                },
            ),
        };
        self.decide(r.xhr_id, phase, decision);
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
            InterceptionPhase::Request,
            ResourceInterceptionDecision::Fulfill {
                request_overrides: None,
                status,
                headers,
                body,
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
            InterceptionPhase::Request,
            ResourceInterceptionDecision::Fail(error_text),
        );
    }
    pub(crate) fn fail_pending_fetch_auth(
        &self,
        r: crate::worker::WorkerPendingFetchContinue,
        error_text: String,
    ) {
        self.decide(
            r.fetch_id,
            InterceptionPhase::Auth,
            ResourceInterceptionDecision::Fail(error_text),
        );
    }
    pub(crate) fn fail_pending_xhr_auth(
        &self,
        r: crate::worker::WorkerPendingXhrContinue,
        error_text: String,
    ) {
        self.decide(
            r.xhr_id,
            InterceptionPhase::Auth,
            ResourceInterceptionDecision::Fail(error_text),
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
            InterceptionPhase::Response,
            ResourceInterceptionDecision::ContinueResponse { status, headers },
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
            InterceptionPhase::Response,
            ResourceInterceptionDecision::ContinueResponse { status, headers },
        );
    }
}
