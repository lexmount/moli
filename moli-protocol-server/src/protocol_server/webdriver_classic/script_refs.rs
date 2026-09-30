use std::collections::BTreeMap;

use moli_protocol::automation::DevToolsDomNodeReference;
use serde_json::{Map, Value};

use super::{
    CLASSIC_SCRIPT_WEB_REFERENCE_BACKEND_NODE_ID, CLASSIC_SCRIPT_WEB_REFERENCE_ELEMENT,
    CLASSIC_SCRIPT_WEB_REFERENCE_FRAME, CLASSIC_SCRIPT_WEB_REFERENCE_MARKER,
    CLASSIC_SCRIPT_WEB_REFERENCE_NODE_ID, CLASSIC_SCRIPT_WEB_REFERENCE_POPUP_ID,
    CLASSIC_SCRIPT_WEB_REFERENCE_POPUP_WINDOW, CLASSIC_SCRIPT_WEB_REFERENCE_SHADOW_ROOT,
    CLASSIC_SCRIPT_WEB_REFERENCE_WINDOW,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClassicScriptCanonicalNodeReference {
    pub(super) node_id: u32,
    pub(super) reference: DevToolsDomNodeReference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct ClassicScriptFrameOwnerReferenceKey {
    node_id: u32,
    backend_node_id: Option<u32>,
}

impl ClassicScriptFrameOwnerReferenceKey {
    fn new(node_id: u32, backend_node_id: Option<u32>) -> Self {
        Self {
            node_id,
            backend_node_id,
        }
    }

    pub(super) fn node_id(self) -> u32 {
        self.node_id
    }

    pub(super) fn has_backend_node_id(self) -> bool {
        self.backend_node_id.is_some()
    }

    pub(super) fn dom_reference(self) -> DevToolsDomNodeReference {
        self.backend_node_id
            .map(DevToolsDomNodeReference::BackendNodeId)
            .unwrap_or(DevToolsDomNodeReference::FrontendNodeId(self.node_id))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClassicScriptResultReference {
    Element {
        node_id: u32,
        backend_node_id: Option<u32>,
    },
    ShadowRoot {
        node_id: u32,
        backend_node_id: Option<u32>,
    },
    Frame {
        owner: ClassicScriptFrameOwnerReferenceKey,
    },
    Window,
    PopupWindow(u64),
}

pub(super) fn collect_classic_script_dom_reference_node_ids(value: &Value, out: &mut Vec<u32>) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_classic_script_dom_reference_node_ids(value, out);
            }
        }
        Value::Object(map) => {
            match classic_script_result_reference(map) {
                Some(ClassicScriptResultReference::Element {
                    node_id,
                    backend_node_id,
                })
                | Some(ClassicScriptResultReference::ShadowRoot {
                    node_id,
                    backend_node_id,
                }) => {
                    if backend_node_id.is_none() {
                        out.push(node_id);
                    }
                    return;
                }
                Some(ClassicScriptResultReference::Frame { .. })
                | Some(ClassicScriptResultReference::Window)
                | Some(ClassicScriptResultReference::PopupWindow(_)) => return,
                None => {}
            }
            for value in map.values() {
                collect_classic_script_dom_reference_node_ids(value, out);
            }
        }
        _ => {}
    }
}

pub(super) fn classic_script_canonical_dom_reference(
    node_id: u32,
    backend_node_id: Option<u32>,
    dom_references_by_node_id: &BTreeMap<u32, ClassicScriptCanonicalNodeReference>,
) -> ClassicScriptCanonicalNodeReference {
    if let Some(reference) = dom_references_by_node_id.get(&node_id) {
        return reference.clone();
    }
    if let Some(backend_node_id) = backend_node_id {
        return ClassicScriptCanonicalNodeReference {
            node_id: backend_node_id,
            reference: DevToolsDomNodeReference::BackendNodeId(backend_node_id),
        };
    }
    ClassicScriptCanonicalNodeReference {
        node_id,
        reference: DevToolsDomNodeReference::FrontendNodeId(node_id),
    }
}

pub(super) fn classic_script_canonical_dom_reference_from_described_node(
    node: &Value,
    fallback_node_id: u32,
) -> ClassicScriptCanonicalNodeReference {
    if let Some(backend_node_id) = backend_node_id_from_described_node(node) {
        return ClassicScriptCanonicalNodeReference {
            node_id: backend_node_id,
            reference: DevToolsDomNodeReference::BackendNodeId(backend_node_id),
        };
    }
    ClassicScriptCanonicalNodeReference {
        node_id: fallback_node_id,
        reference: DevToolsDomNodeReference::FrontendNodeId(fallback_node_id),
    }
}

pub(super) fn classic_script_frame_owner_dom_reference_from_described_node(
    owner: ClassicScriptFrameOwnerReferenceKey,
    node: &Value,
) -> DevToolsDomNodeReference {
    if owner.has_backend_node_id() {
        return owner.dom_reference();
    }
    backend_node_id_from_described_node(node)
        .map(DevToolsDomNodeReference::BackendNodeId)
        .unwrap_or(DevToolsDomNodeReference::FrontendNodeId(owner.node_id()))
}

pub(super) fn collect_classic_script_frame_reference_owner_keys(
    value: &Value,
    out: &mut Vec<ClassicScriptFrameOwnerReferenceKey>,
) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_classic_script_frame_reference_owner_keys(value, out);
            }
        }
        Value::Object(map) => {
            if let Some(ClassicScriptResultReference::Frame { owner }) =
                classic_script_result_reference(map)
            {
                out.push(owner);
                return;
            }
            for value in map.values() {
                collect_classic_script_frame_reference_owner_keys(value, out);
            }
        }
        _ => {}
    }
}

pub(super) fn classic_script_result_reference(
    map: &Map<String, Value>,
) -> Option<ClassicScriptResultReference> {
    let kind = map
        .get(CLASSIC_SCRIPT_WEB_REFERENCE_MARKER)
        .and_then(Value::as_str)?;
    if kind == CLASSIC_SCRIPT_WEB_REFERENCE_WINDOW {
        return Some(ClassicScriptResultReference::Window);
    }
    if kind == CLASSIC_SCRIPT_WEB_REFERENCE_POPUP_WINDOW {
        return map
            .get(CLASSIC_SCRIPT_WEB_REFERENCE_POPUP_ID)
            .and_then(Value::as_str)
            .and_then(|popup_id| popup_id.parse::<u64>().ok())
            .filter(|popup_id| *popup_id > 0)
            .map(ClassicScriptResultReference::PopupWindow);
    }
    let backend_node_id = map
        .get(CLASSIC_SCRIPT_WEB_REFERENCE_BACKEND_NODE_ID)
        .and_then(Value::as_u64)
        .and_then(|node_id| u32::try_from(node_id).ok())
        .filter(|node_id| *node_id > 0);
    let node_id = map
        .get(CLASSIC_SCRIPT_WEB_REFERENCE_NODE_ID)
        .and_then(Value::as_u64)
        .and_then(|node_id| u32::try_from(node_id).ok())
        .or(backend_node_id)?;
    match kind {
        CLASSIC_SCRIPT_WEB_REFERENCE_ELEMENT => Some(ClassicScriptResultReference::Element {
            node_id,
            backend_node_id,
        }),
        CLASSIC_SCRIPT_WEB_REFERENCE_SHADOW_ROOT => {
            Some(ClassicScriptResultReference::ShadowRoot {
                node_id,
                backend_node_id,
            })
        }
        CLASSIC_SCRIPT_WEB_REFERENCE_FRAME => Some(ClassicScriptResultReference::Frame {
            owner: ClassicScriptFrameOwnerReferenceKey::new(node_id, backend_node_id),
        }),
        _ => None,
    }
}

pub(super) fn classic_script_result_contains_popup_window_reference(value: &Value) -> bool {
    match value {
        Value::Array(values) => values
            .iter()
            .any(classic_script_result_contains_popup_window_reference),
        Value::Object(map) => {
            matches!(
                classic_script_result_reference(map),
                Some(ClassicScriptResultReference::PopupWindow(_))
            ) || map
                .values()
                .any(classic_script_result_contains_popup_window_reference)
        }
        _ => false,
    }
}

fn backend_node_id_from_described_node(node: &Value) -> Option<u32> {
    node.get("backendNodeId")
        .and_then(Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .filter(|id| *id > 0)
}

#[cfg(test)]
mod tests {
    use moli_protocol::automation::DevToolsDomNodeReference;
    use serde_json::json;

    use super::*;

    #[test]
    fn frame_result_reference_preserves_backend_node_id() {
        let mut value = Map::new();
        value.insert(
            CLASSIC_SCRIPT_WEB_REFERENCE_MARKER.to_owned(),
            json!(CLASSIC_SCRIPT_WEB_REFERENCE_FRAME),
        );
        value.insert(CLASSIC_SCRIPT_WEB_REFERENCE_NODE_ID.to_owned(), json!(7));
        value.insert(
            CLASSIC_SCRIPT_WEB_REFERENCE_BACKEND_NODE_ID.to_owned(),
            json!(2_000_000_007_u32),
        );

        assert_eq!(
            classic_script_result_reference(&value),
            Some(ClassicScriptResultReference::Frame {
                owner: ClassicScriptFrameOwnerReferenceKey::new(7, Some(2_000_000_007)),
            })
        );
    }

    #[test]
    fn frame_result_reference_accepts_backend_node_id_without_frontend_node_id() {
        let mut value = Map::new();
        value.insert(
            CLASSIC_SCRIPT_WEB_REFERENCE_MARKER.to_owned(),
            json!(CLASSIC_SCRIPT_WEB_REFERENCE_FRAME),
        );
        value.insert(
            CLASSIC_SCRIPT_WEB_REFERENCE_BACKEND_NODE_ID.to_owned(),
            json!(2_000_000_007_u32),
        );

        assert_eq!(
            classic_script_result_reference(&value),
            Some(ClassicScriptResultReference::Frame {
                owner: ClassicScriptFrameOwnerReferenceKey::new(2_000_000_007, Some(2_000_000_007)),
            })
        );
    }

    #[test]
    fn frame_owner_key_prefers_backend_node_id() {
        assert_eq!(
            ClassicScriptFrameOwnerReferenceKey::new(7, Some(2_000_000_007)).dom_reference(),
            DevToolsDomNodeReference::BackendNodeId(2_000_000_007)
        );
        assert_eq!(
            ClassicScriptFrameOwnerReferenceKey::new(7, None).dom_reference(),
            DevToolsDomNodeReference::FrontendNodeId(7)
        );
    }

    #[test]
    fn described_frame_owner_reference_prefers_backend_node_id() {
        let owner = ClassicScriptFrameOwnerReferenceKey::new(7, None);
        assert_eq!(
            classic_script_frame_owner_dom_reference_from_described_node(
                owner,
                &json!({ "nodeId": 7, "backendNodeId": 2_000_000_007_u32 })
            ),
            DevToolsDomNodeReference::BackendNodeId(2_000_000_007)
        );
    }

    #[test]
    fn parsed_frame_owner_backend_node_id_wins_over_described_node() {
        let owner = ClassicScriptFrameOwnerReferenceKey::new(7, Some(2_000_000_007));
        assert_eq!(
            classic_script_frame_owner_dom_reference_from_described_node(
                owner,
                &json!({ "nodeId": 7, "backendNodeId": 2_000_000_008_u32 })
            ),
            DevToolsDomNodeReference::BackendNodeId(2_000_000_007)
        );
    }

    #[test]
    fn canonical_dom_reference_does_not_guess_backend_identity_from_frontend_id_range() {
        let high_frontend_node_id = 2_000_000_007;
        let references = BTreeMap::new();

        assert_eq!(
            classic_script_canonical_dom_reference(high_frontend_node_id, None, &references),
            ClassicScriptCanonicalNodeReference {
                node_id: high_frontend_node_id,
                reference: DevToolsDomNodeReference::FrontendNodeId(high_frontend_node_id),
            }
        );
        assert_eq!(
            classic_script_canonical_dom_reference_from_described_node(
                &json!({ "nodeId": high_frontend_node_id }),
                high_frontend_node_id
            ),
            ClassicScriptCanonicalNodeReference {
                node_id: high_frontend_node_id,
                reference: DevToolsDomNodeReference::FrontendNodeId(high_frontend_node_id),
            }
        );
    }
}
