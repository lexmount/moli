use anyhow::{Result, bail, ensure};

use super::{
    InterceptionPhase, ResourceInterceptionDecision, ResourceInterceptionPause,
    ResourceInterceptionReply,
};
use crate::{RendererDevToolsPageCommand, RendererPageCommand};

impl ResourceInterceptionPause {
    /// Releasing an interceptor changes native eligibility immediately. It
    /// must not enqueue a Page policy turn behind another session's pause.
    pub fn set_target_interception(&self, classes: crate::SubresourceInterceptionClasses) {
        self.target_policy.set_subresource_interception(classes);
    }

    /// Adapter for the existing Fetch command surface. Only the exhaustive
    /// native network command subset is accepted; other Page work stays queued.
    pub fn dispatch(&self, command: RendererPageCommand) -> Result<ResourceInterceptionReply> {
        use RendererDevToolsPageCommand as C;
        use ResourceInterceptionDecision as D;
        use ResourceInterceptionReply as R;
        let RendererPageCommand::DevTools(command) = command else {
            bail!("command cannot run through a native network pause");
        };
        let (id, phase, decision, reply) = match command {
            C::ContinuePendingSubresourceFetch {
                internal_id,
                url,
                method,
                body,
                headers,
                intercept_response,
                handle_auth_requests,
            } => (
                internal_id,
                InterceptionPhase::Request,
                D::ContinueRequest {
                    url,
                    method,
                    body,
                    headers,
                    intercept_response,
                    handle_auth_requests,
                },
                R::Started,
            ),
            C::FailPendingSubresourceFetch {
                internal_id,
                error_text,
            } => (
                internal_id,
                InterceptionPhase::Request,
                D::Fail(error_text),
                R::Unit,
            ),
            C::FulfillPendingSubresourceFetch {
                internal_id,
                request_overrides,
                response_code,
                response_headers,
                response_body,
            } => (
                internal_id,
                InterceptionPhase::Request,
                D::Fulfill {
                    request_overrides,
                    status: response_code,
                    headers: response_headers,
                    body: response_body,
                },
                R::Unit,
            ),
            C::ContinuePendingSubresourceResponse {
                internal_id,
                response_code,
                response_headers,
            } => (
                internal_id,
                InterceptionPhase::Response,
                D::ContinueResponse {
                    status: response_code,
                    headers: response_headers,
                },
                R::Unit,
            ),
            C::FailPendingSubresourceResponse {
                internal_id,
                error_text,
            } => (
                internal_id,
                InterceptionPhase::Response,
                D::Fail(error_text),
                R::Unit,
            ),
            C::FulfillPendingSubresourceResponse {
                internal_id,
                response_code,
                response_headers,
                response_body,
            } => (
                internal_id,
                InterceptionPhase::Response,
                D::Fulfill {
                    request_overrides: None,
                    status: response_code,
                    headers: response_headers,
                    body: response_body,
                },
                R::Unit,
            ),
            C::ContinuePendingSubresourceAuth { internal_id, auth } => (
                internal_id,
                InterceptionPhase::Auth,
                D::Auth(auth),
                R::Started,
            ),
            C::CancelPendingSubresourceAuth { internal_id } => {
                (internal_id, InterceptionPhase::Auth, D::CancelAuth, R::Unit)
            }
            C::FailPendingSubresourceAuth {
                internal_id,
                error_text,
            } => (
                internal_id,
                InterceptionPhase::Auth,
                D::Fail(error_text),
                R::Unit,
            ),
            _ => bail!("command cannot run through a native network pause"),
        };
        ensure!(
            id == self.internal_id && phase == self.phase,
            "network pause identity or phase does not match"
        );
        self.send_decision(decision)?;
        Ok(reply)
    }
}
