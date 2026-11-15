use crate::{
    document_runtime::DocumentConnectPolicySnapshot,
    network::{interception_output::ResourceInterceptionContext, loads::ResourceLoadLease},
    types::PendingSubresourceFetchInfo,
};

/// Chosen once, before dispatch. Compatibility is the explicit asynchronous
/// adapter for embeddings without a native protocol owner.
/// An incomplete native owner is an error, never an ordinary network route.
pub(crate) enum XhrInterceptionRoute {
    Ordinary,
    Native(ControlledXhrContext),
    Compatibility,
}

impl XhrInterceptionRoute {
    pub(crate) fn synchronous(self) -> Result<Option<ControlledXhrContext>, &'static str> {
        match self {
            Self::Ordinary => Ok(None),
            Self::Native(context) => Ok(Some(context)),
            Self::Compatibility => {
                Err("Synchronous XMLHttpRequest interception requires a protocol output owner")
            }
        }
    }
}

pub(crate) struct ControlledXhrContext {
    pub(crate) interception: ResourceInterceptionContext,
    pub(crate) connect_policy: DocumentConnectPolicySnapshot,
}

impl ControlledXhrContext {
    pub(crate) fn load(
        self,
        mut request: moli_fetch::Request,
        mut info: PendingSubresourceFetchInfo,
        load: ResourceLoadLease,
        cancel: moli_fetch::FetchCancelHandle,
        preflight_headers: Vec<(String, String)>,
    ) -> super::ControlledResourceLoad {
        request = load.request_client().bind_browser_site_context(request);
        info.request_headers = request.interception_request_headers_with_web_security(
            load.request_client().web_security_disabled(),
        );
        super::ControlledResourceLoad {
            response_policy: super::ResourceResponsePolicy::Xhr,
            service_worker: None,
            response_filter: None,
            request,
            info,
            load,
            cancel,
            interception: self.interception,
            connect_policy: Some(self.connect_policy),
            csp_violations: Vec::new(),
            preflight_headers,
            initial_decision: None,
            request_initiator_type: crate::types::SubresourceRequestInitiatorType::Script,
        }
    }
}

impl ControlledXhrContext {
    pub(crate) fn fetch_load(
        self,
        request: moli_fetch::Request,
        info: PendingSubresourceFetchInfo,
        load: ResourceLoadLease,
        cancel: moli_fetch::FetchCancelHandle,
        policy: crate::types::SubresourcePolicyContext,
    ) -> super::ControlledResourceLoad {
        let headers = request.request_headers.to_byte_strings();
        let mut controlled = self.load(request, info, load, cancel, headers);
        controlled.response_policy = super::ResourceResponsePolicy::Fetch(policy);
        controlled
    }
}
