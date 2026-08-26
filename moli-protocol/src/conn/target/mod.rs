mod activation;
mod auto_attach_owner;
mod control;
mod graph;
mod host;
mod observer;
mod projection;
mod registry;
mod route;
mod session;
mod session_binding;
mod transaction;
mod worker_auto_attach;
mod worker_session;

pub(crate) use activation::TargetActivationTransition;
pub(crate) use control::TargetControlPlane;
pub(crate) use observer::{TargetHandlerStore, target_destroyed_automation_events};
pub(crate) use registry::{TargetClosurePlan, TargetHostDelta, TargetRegistry};
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
