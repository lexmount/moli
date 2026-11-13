use crate::{
    document_runtime::DocumentConnectPolicySnapshot,
    network::{interception_output::ResourceInterceptionContext, loads::ResourceLoadLease},
    types::PendingSubresourceFetchInfo,
};

/// Chosen once, before dispatch. Compatibility is the explicit asynchronous
/// adapter for embeddings without a native protocol owner (and SharedWorkers).
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
        request: moli_fetch::Request,
        info: PendingSubresourceFetchInfo,
        load: ResourceLoadLease,
        cancel: moli_fetch::FetchCancelHandle,
        preflight_headers: Vec<(String, String)>,
    ) -> super::ControlledXhrLoad {
        super::ControlledXhrLoad {
            request,
            info,
            load,
            cancel,
            source_document: self.interception.source_document,
            output: self.interception.output,
            connect_policy: self.connect_policy,
            preflight_headers,
            initial_decision: None,
        }
    }
}
