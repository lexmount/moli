mod activation;
mod auto_attach_owner;
mod control;
mod default_target;
mod graph;
mod observer;
mod projection;
mod route;
mod session;
mod session_binding;
mod transaction;
mod worker_auto_attach;
mod worker_session;

pub(crate) use activation::TargetActivationTransition;
pub(crate) use control::TargetControlPlane;
pub(crate) use default_target::{DEFAULT_BROWSER_CONTEXT_ID, DefaultTargetLifecycle};
pub(crate) use graph::{TargetClosurePlan, TargetHostDelta};
pub(crate) use observer::{TargetHandlerStore, target_destroyed_automation_events};
pub(crate) use route::{CdpSessionRoute, TargetHandlerAccessMode};
pub(crate) use session::{
    CommittedAttachSession, DetachedTargetSession, PreparedAttachSession, TargetSessionRegistry,
};
pub(crate) use transaction::{
    PreparedTargetAttach, PreparedTargetHostClosure, PreparedTargetHostDelta,
    TargetAttachRollbackPlan, TargetAttachSessionCommit, TargetAutoAttachedSessionDetachPlan,
    TargetBindingCleanupAction, TargetBindingCleanupPlan, TargetClosureCleanupPlan,
    TargetEventPlan, TargetSessionDetachCleanupPlan,
};
