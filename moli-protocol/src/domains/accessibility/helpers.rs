use crate::conn::CdpConnection;
pub(super) use chromiumoxide_cdp::cdp::browser_protocol::accessibility::{
    GetChildAxNodesParams as ChildAxNodesParams, GetFullAxTreeParams,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FrameScopedParams {
    #[serde(default)]
    pub(super) frame_id: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct NodeReferenceParams {
    #[serde(default)]
    pub(super) node_id: Option<u32>,
    #[serde(default)]
    pub(super) backend_node_id: Option<u32>,
    #[serde(default)]
    pub(super) object_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AncestorsParams {
    #[serde(flatten)]
    pub(super) reference: NodeReferenceParams,
    #[serde(default)]
    pub(super) frame_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct QueryAxTreeParams {
    #[serde(flatten)]
    pub(super) reference: NodeReferenceParams,
    #[serde(default)]
    pub(super) frame_id: Option<String>,
    #[serde(default)]
    pub(super) accessible_name: Option<String>,
    #[serde(default)]
    pub(super) role: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PartialAxTreeParams {
    #[serde(flatten)]
    pub(super) reference: NodeReferenceParams,
    #[serde(default)]
    pub(super) frame_id: Option<String>,
    #[serde(default)]
    pub(super) fetch_relatives: Option<bool>,
}

pub(super) fn top_frame_id_for_session(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> Option<String> {
    conn.target_session_owner_frame_tree_identity(session_id)
        .map(|(frame_id, _, _, _)| frame_id)
}

pub(super) fn parse_ax_backend_node_id(id: &str) -> Option<u32> {
    let backend_id = id.strip_prefix("AX-")?.parse::<u32>().ok()?;
    if backend_id == 0 {
        return None;
    }
    Some(backend_id)
}
