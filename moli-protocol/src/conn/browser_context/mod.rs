use crate::conn::{BrowserContext, CdpConnection, ParkedPageSessionState, TargetPageSessionState};
use crate::domains::audits::SessionOwnerAuditsEnableResult;
use crate::domains::log::{SessionOwnerLogControlResult, SessionOwnerLogEnableResult};
use serde_json::Value;

mod emulation_owner;
mod fetch_owner;
mod lifecycle;
mod lookup;
mod network_owner;
mod page_owner;
mod runtime_owner;
mod session_owner;
mod target_session_owner;

pub(crate) use emulation_owner::TargetEmulationSessionStateMut;
pub(crate) use page_owner::PageLifecycleEventsEnableResult;
pub(crate) use runtime_owner::{
    SessionOwnerInspectorEnableResult, SessionOwnerRuntimeFrontendEnableResult,
};
pub(crate) use session_owner::{CdpSessionRoute, TargetHandlerAccessMode};
pub(crate) use target_session_owner::TargetNavigationLoadInputs;
