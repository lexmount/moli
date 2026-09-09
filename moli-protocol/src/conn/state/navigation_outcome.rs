#[cfg(test)]
use moli_core::browser::DownloadBody;
use moli_core::browser::WebContentsHandle;
use serde::Serialize;
use serde_json::Value;
use url::Url;

use crate::conn::CommandOwnerScope;
use crate::devtools_runtime::DevToolsProtocol;
#[cfg(test)]
use crate::domains::network::CompletedDownloadProgressTransfer;

use super::browser_context::BrowserContext;

pub(crate) const NETWORK_ERROR_PAGE_URL: &str = "chrome-error://chromewebdata/";

#[derive(Debug)]
#[cfg(test)]
pub(crate) struct CompletedDownloadBodyArtifact {
    body: DownloadBody,
    response_headers: Vec<(String, String)>,
}

#[cfg(test)]
impl CompletedDownloadBodyArtifact {
    #[cfg(test)]
    pub(crate) fn from_body(body: DownloadBody, response_headers: Vec<(String, String)>) -> Self {
        Self {
            body,
            response_headers,
        }
    }

    pub(crate) fn into_parts(self) -> (DownloadBody, Vec<(String, String)>) {
        (self.body, self.response_headers)
    }
}

#[derive(Debug)]
#[cfg(test)]
pub struct DownloadNavigation {
    pub final_url: Url,
    pub(crate) progress_transfer: CompletedDownloadProgressTransfer,
}

#[derive(Debug)]
pub enum NavigationLoadOutcome {
    #[cfg(test)]
    Download(Box<DownloadNavigation>),
    NetworkFailure(String),
}

impl NavigationLoadOutcome {
    #[cfg(test)]
    pub(crate) fn download(navigation: DownloadNavigation) -> Self {
        Self::Download(Box::new(navigation))
    }
}

#[derive(Debug, Clone)]
pub(crate) enum NavigationResultProjection {
    Cdp(Value),
    WebDriverClassic(Value),
    WebDriverBidi(Value),
}

impl NavigationResultProjection {
    pub(crate) fn new(protocol: DevToolsProtocol, payload: Value) -> Self {
        match protocol {
            DevToolsProtocol::Cdp => Self::Cdp(payload),
            DevToolsProtocol::WebDriverClassic => Self::WebDriverClassic(payload),
            DevToolsProtocol::WebDriverBidi => Self::WebDriverBidi(payload),
        }
    }

    pub(crate) fn protocol(&self) -> DevToolsProtocol {
        match self {
            Self::Cdp(_) => DevToolsProtocol::Cdp,
            Self::WebDriverClassic(_) => DevToolsProtocol::WebDriverClassic,
            Self::WebDriverBidi(_) => DevToolsProtocol::WebDriverBidi,
        }
    }

    pub(crate) fn payload(&self) -> &Value {
        match self {
            Self::Cdp(payload) | Self::WebDriverClassic(payload) | Self::WebDriverBidi(payload) => {
                payload
            }
        }
    }

    pub(crate) fn payload_mut(&mut self) -> &mut Value {
        match self {
            Self::Cdp(payload) | Self::WebDriverClassic(payload) | Self::WebDriverBidi(payload) => {
                payload
            }
        }
    }

    pub(crate) fn into_payload(self) -> Value {
        match self {
            Self::Cdp(payload) | Self::WebDriverClassic(payload) | Self::WebDriverBidi(payload) => {
                payload
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct NavigationDispatchState {
    pub navigate_id: Option<u64>,
    pub(crate) owner: CommandOwnerScope,
    pub(crate) web_contents: WebContentsHandle,
    pub(crate) result_projection: NavigationResultProjection,
    pub frame_id: String,
    pub session_id: Option<String>,
    pub request_id: Option<String>,
    pub loader_id: String,
    pub request_announced: bool,
    pub requested_url: Url,
    pub request_method: String,
    /// Text projection used by CDP request events and Fetch interception.
    pub request_body: Option<String>,
    /// Authoritative bytes used for transport. This differs from the text
    /// projection for multipart form data containing binary file payloads.
    pub request_body_bytes: Option<Vec<u8>>,
    pub request_headers: Vec<(String, String)>,
    pub request_load_policy: NavigationRequestLoadPolicy,
    pub timestamp: f64,
}

impl NavigationDispatchState {
    pub(crate) fn native_result_payload(
        &self,
        url: &Url,
        error: Option<&str>,
        download: bool,
    ) -> Result<Value, String> {
        if let Some(error) = error
            && self.result_projection.protocol() != DevToolsProtocol::Cdp
        {
            return Err(error.to_owned());
        }
        let mut result = self.result_projection.payload().clone();
        if let Some(payload) = result.as_object_mut() {
            if payload.contains_key("url") {
                payload.insert("url".into(), Value::String(url.to_string()));
            }
            if let Some(error) = error {
                payload.insert("errorText".into(), Value::String(error.to_owned()));
                payload.insert("isDownload".into(), Value::Bool(false));
            }
            if download {
                payload.remove("loaderId");
                payload.insert(
                    "errorText".into(),
                    Value::String(moli_fetch::NET_ERR_ABORTED_ERROR_TEXT.into()),
                );
                payload.insert("isDownload".into(), Value::Bool(true));
            }
        }
        Ok(result)
    }

    #[cfg(test)]
    pub(crate) fn detached_web_contents_for_test() -> WebContentsHandle {
        WebContentsHandle::new(
            moli_core::browser::BrowserContextId::allocate(),
            moli_core::browser::WebContentsId::allocate(),
        )
    }

    pub(crate) fn clone_request_body_bytes(&self) -> Option<Vec<u8>> {
        self.request_body_bytes
            .clone()
            .or_else(|| self.request_body.clone().map(String::into_bytes))
    }

    pub(crate) fn set_request_body_text(&mut self, body: String) {
        self.request_body_bytes = Some(body.as_bytes().to_vec());
        self.request_body = Some(body);
    }
}

pub use moli_core::browser::NavigationRequestLoadPolicy;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetInfo<'a> {
    pub target_id: &'a str,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub title: String,
    pub url: &'a str,
    pub attached: bool,
    pub can_access_opener: bool,
    pub browser_context_id: &'a str,
}

impl<'a> TargetInfo<'a> {
    pub fn from_bc(bc: &'a BrowserContext, target_id: &'a str, attached: bool) -> Self {
        Self {
            target_id,
            kind: "page",
            title: bc
                .active_page_target()
                .owner_state
                .committed_document_title()
                .map(str::to_owned)
                .or_else(|| bc.target_document_title(bc.active_target_id()?))
                .unwrap_or_default(),
            url: bc.target_url(),
            attached,
            can_access_opener: false,
            browser_context_id: &bc.id,
        }
    }
}
