mod sessions;

pub(in crate::script_vm) use sessions::PageInspectorSessionTarget;
pub(super) use sessions::{
    RendererFrontendInspectorSessions, RendererInspectorSessionConnection, inspector_session_key,
};
