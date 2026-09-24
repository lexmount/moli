use crate::conn::{BrowserContext, CdpConnection, CdpSessionRoute, TargetPageSessionState};
use crate::domains::audits::SessionOwnerAuditsEnableResult;
use crate::domains::log::{SessionOwnerLogControlResult, SessionOwnerLogEnableResult};
use serde_json::Value;

mod dialog_owner;
mod document_commit;
mod emulation_owner;
mod fetch_owner;
mod lifecycle;
mod lookup;
mod navigation_decisions;
mod navigation_events;
mod network_events;
mod network_owner;
mod page_owner;
mod runtime_owner;
mod session_owner;
mod target_session_owner;

pub(crate) use lifecycle::PageCloseNotifications;
pub(crate) use page_owner::PageLifecycleEventsEnableResult;
pub(crate) use runtime_owner::{
    SessionOwnerInspectorEnableResult, SessionOwnerRuntimeFrontendEnableResult,
};
#[cfg(test)]
pub(crate) use target_session_owner::TargetNavigationLoadInputs;
