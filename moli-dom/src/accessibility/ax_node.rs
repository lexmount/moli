use std::fmt;

use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::Value;

/// Keep references numeric until a protocol or dump actually serializes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccessibilityNodeId {
    Dom(u32),
    ListMarker(u32),
}

impl fmt::Display for AccessibilityNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dom(id) => write!(f, "AX-{id}"),
            Self::ListMarker(id) => write!(f, "AX-LM-{id}"),
        }
    }
}

impl Serialize for AccessibilityNodeId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[derive(Debug, Serialize)]
pub struct AccessibilityValue<T> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub value: T,
}

/// The canonical AX payload, without a JSON object and allocated field names
/// for every node. Serialization retains the protocol's field insertion order.
#[derive(Debug)]
pub struct AccessibilityNode {
    pub node_id: AccessibilityNodeId,
    pub backend_node_id: Option<u32>,
    pub ignored: bool,
    pub ignored_reasons: Option<Value>,
    pub role: AccessibilityValue<&'static str>,
    pub name: Option<AccessibilityValue<String>>,
    pub value: Option<Value>,
    pub properties: Option<Vec<Value>>,
    pub parent_id: Option<AccessibilityNodeId>,
    pub child_ids: Option<Vec<AccessibilityNodeId>>,
}

impl Serialize for AccessibilityNode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut payload = serializer.serialize_map(None)?;
        payload.serialize_entry("nodeId", &self.node_id)?;
        if let Some(id) = self.backend_node_id {
            payload.serialize_entry("backendDOMNodeId", &id)?;
        }
        payload.serialize_entry("ignored", &self.ignored)?;
        // DOM nodes without an AX object have a smaller synthetic payload,
        // whose role historically precedes its ignored reason.
        if self.properties.is_none() {
            payload.serialize_entry("role", &self.role)?;
        }
        if let Some(reasons) = &self.ignored_reasons {
            payload.serialize_entry("ignoredReasons", reasons)?;
        }
        if self.properties.is_some() {
            payload.serialize_entry("role", &self.role)?;
        }
        if let Some(name) = &self.name {
            payload.serialize_entry("name", name)?;
        }
        if let Some(value) = &self.value {
            payload.serialize_entry("value", value)?;
        }
        if let Some(properties) = &self.properties {
            payload.serialize_entry("properties", properties)?;
        }
        // Joining a child frame appends parentId to its existing root payload.
        let mut parent_id = self.parent_id;
        if self.role.value != "RootWebArea"
            && let Some(parent) = parent_id.take()
        {
            payload.serialize_entry("parentId", &parent)?;
        }
        if let Some(children) = &self.child_ids {
            payload.serialize_entry("childIds", children)?;
        }
        if let Some(parent) = parent_id {
            payload.serialize_entry("parentId", &parent)?;
        }
        payload.end()
    }
}

impl From<AccessibilityNode> for Value {
    fn from(node: AccessibilityNode) -> Self {
        let mut payload = serde_json::Map::new();
        payload.insert("nodeId".to_owned(), Value::String(node.node_id.to_string()));
        if let Some(id) = node.backend_node_id {
            payload.insert("backendDOMNodeId".to_owned(), id.into());
        }
        payload.insert("ignored".to_owned(), node.ignored.into());
        let mut ignored_reasons = node.ignored_reasons;
        if node.properties.is_some()
            && let Some(reasons) = ignored_reasons.take()
        {
            payload.insert("ignoredReasons".to_owned(), reasons);
        }
        payload.insert(
            "role".to_owned(),
            serde_json::json!({"type":node.role.kind, "value":node.role.value}),
        );
        if let Some(reasons) = ignored_reasons {
            payload.insert("ignoredReasons".to_owned(), reasons);
        }
        if let Some(name) = node.name {
            payload.insert(
                "name".to_owned(),
                serde_json::json!({"type":name.kind, "value":name.value}),
            );
        }
        if let Some(value) = node.value {
            payload.insert("value".to_owned(), value);
        }
        if let Some(properties) = node.properties {
            payload.insert("properties".to_owned(), Value::Array(properties));
        }
        let mut parent_id = node.parent_id;
        if node.role.value != "RootWebArea"
            && let Some(parent) = parent_id.take()
        {
            payload.insert("parentId".to_owned(), Value::String(parent.to_string()));
        }
        if let Some(children) = node.child_ids {
            payload.insert(
                "childIds".to_owned(),
                Value::Array(
                    children
                        .into_iter()
                        .map(|id| Value::String(id.to_string()))
                        .collect(),
                ),
            );
        }
        if let Some(parent) = parent_id {
            payload.insert("parentId".to_owned(), Value::String(parent.to_string()));
        }
        Self::Object(payload)
    }
}
