use std::fmt;
use std::num::NonZeroU32;

use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::Value;

/// Keep references numeric until a protocol or dump actually serializes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccessibilityNodeId {
    Dom(NonZeroU32),
    ListMarker(NonZeroU32),
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
    pub backend_node_id: Option<NonZeroU32>,
    pub ignored: bool,
    pub role: &'static str,
    pub name: Option<String>,
    pub parent_id: Option<AccessibilityNodeId>,
    pub child_ids: Option<Vec<AccessibilityNodeId>>,
    properties_present: bool,
    rare_data: Option<Box<AccessibilityNodeRareData>>,
}

/// Common nodes have neither a value nor an ignored reason, and most have no
/// properties. Keep their empty JSON containers out of the common record.
#[derive(Debug, Default)]
struct AccessibilityNodeRareData {
    ignored_reasons: Option<Value>,
    value: Option<Value>,
    properties: Vec<Value>,
}

impl AccessibilityNode {
    /// An ordinary AX node includes an empty properties array even when it
    /// needs no rare allocation. Synthetic nodes can omit it via set_rare_data.
    pub fn new(node_id: AccessibilityNodeId, role: &'static str) -> Self {
        Self {
            node_id,
            backend_node_id: match node_id {
                AccessibilityNodeId::Dom(id) => Some(id),
                AccessibilityNodeId::ListMarker(_) => None,
            },
            ignored: false,
            role,
            name: None,
            parent_id: None,
            child_ids: None,
            properties_present: true,
            rare_data: None,
        }
    }

    pub fn set_rare_data(
        &mut self,
        ignored_reasons: Option<Value>,
        value: Option<Value>,
        properties: Option<Vec<Value>>,
    ) {
        self.properties_present = properties.is_some();
        let properties = properties.unwrap_or_default();
        self.rare_data = if ignored_reasons.is_none() && value.is_none() && properties.is_empty() {
            None
        } else {
            Some(Box::new(AccessibilityNodeRareData {
                ignored_reasons,
                value,
                properties,
            }))
        };
    }

    pub fn ignored_reasons(&self) -> Option<&Value> {
        self.rare_data.as_deref()?.ignored_reasons.as_ref()
    }

    pub fn value(&self) -> Option<&Value> {
        self.rare_data.as_deref()?.value.as_ref()
    }

    pub fn properties(&self) -> Option<&[Value]> {
        self.properties_present.then(|| {
            self.rare_data
                .as_deref()
                .map_or(&[][..], |data| data.properties.as_slice())
        })
    }
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
        let role = AccessibilityValue {
            kind: "role",
            value: self.role,
        };
        if !self.properties_present {
            payload.serialize_entry("role", &role)?;
        }
        if let Some(reasons) = self.ignored_reasons() {
            payload.serialize_entry("ignoredReasons", reasons)?;
        }
        if self.properties_present {
            payload.serialize_entry("role", &role)?;
        }
        if let Some(name) = &self.name {
            payload.serialize_entry(
                "name",
                &AccessibilityValue {
                    kind: "computedString",
                    value: name,
                },
            )?;
        }
        if let Some(value) = self.value() {
            payload.serialize_entry("value", value)?;
        }
        if let Some(properties) = self.properties() {
            payload.serialize_entry("properties", properties)?;
        }
        // Joining a child frame appends parentId to its existing root payload.
        let mut parent_id = self.parent_id;
        if self.role != "RootWebArea"
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
            payload.insert("backendDOMNodeId".to_owned(), id.get().into());
        }
        payload.insert("ignored".to_owned(), node.ignored.into());
        let AccessibilityNodeRareData {
            mut ignored_reasons,
            value,
            properties,
        } = node.rare_data.map(|data| *data).unwrap_or_default();
        if node.properties_present
            && let Some(reasons) = ignored_reasons.take()
        {
            payload.insert("ignoredReasons".to_owned(), reasons);
        }
        payload.insert(
            "role".to_owned(),
            serde_json::json!({"type":"role", "value":node.role}),
        );
        if let Some(reasons) = ignored_reasons {
            payload.insert("ignoredReasons".to_owned(), reasons);
        }
        if let Some(name) = node.name {
            payload.insert(
                "name".to_owned(),
                serde_json::json!({"type":"computedString", "value":name}),
            );
        }
        if let Some(value) = value {
            payload.insert("value".to_owned(), value);
        }
        if node.properties_present {
            payload.insert("properties".to_owned(), Value::Array(properties));
        }
        let mut parent_id = node.parent_id;
        if node.role != "RootWebArea"
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).expect("nonzero fixture ID")
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn common_ax_node_layout_stays_compact() {
        assert_eq!(size_of::<Option<NonZeroU32>>(), size_of::<u32>());
        assert_eq!(size_of::<AccessibilityNodeId>(), 8);
        assert_eq!(size_of::<Option<AccessibilityNodeId>>(), 8);
        assert_eq!(size_of::<Option<Box<AccessibilityNodeRareData>>>(), 8);
        assert!(
            size_of::<AccessibilityNode>() <= 96,
            "AccessibilityNode grew to {} bytes",
            size_of::<AccessibilityNode>()
        );
    }

    #[test]
    fn empty_properties_do_not_allocate_rare_data() {
        let mut node = AccessibilityNode::new(AccessibilityNodeId::Dom(id(7)), "generic");
        node.set_rare_data(None, None, Some(Vec::new()));
        assert!(node.rare_data.is_none());
        assert_eq!(node.properties(), Some(&[][..]));
        assert_eq!(
            serde_json::to_string(&node).expect("AX JSON"),
            r#"{"nodeId":"AX-7","backendDOMNodeId":7,"ignored":false,"role":{"type":"role","value":"generic"},"properties":[]}"#
        );

        node.set_rare_data(None, Some(json!({"type":"string", "value":"text"})), None);
        assert!(node.rare_data.is_some());
        assert!(node.properties().is_none());
        node.set_rare_data(None, None, None);
        assert!(node.rare_data.is_none());
        assert!(node.properties().is_none());
    }

    #[test]
    fn cold_fields_preserve_protocol_values_and_field_order() {
        for properties in [
            None,
            Some(Vec::new()),
            Some(vec![
                json!({"name":"focusable", "value":{"type":"boolean", "value":true}}),
            ]),
        ] {
            let mut node = AccessibilityNode::new(AccessibilityNodeId::Dom(id(7)), "textbox");
            node.ignored = true;
            node.name = Some("A name".to_owned());
            node.parent_id = Some(AccessibilityNodeId::Dom(id(1)));
            node.child_ids = Some(Vec::new());
            let reasons = json!([{"name":"notRendered", "value":{"type":"boolean", "value":true}}]);
            let value = json!({"type":"string", "value":"A value"});
            node.set_rare_data(
                Some(reasons.clone()),
                Some(value.clone()),
                properties.clone(),
            );

            let serialized = serde_json::to_string(&node).expect("AX JSON");
            let mut expected = json!({
                "nodeId":"AX-7", "backendDOMNodeId":7, "ignored":true,
                "ignoredReasons":reasons, "role":{"type":"role", "value":"textbox"},
                "name":{"type":"computedString", "value":"A name"}, "value":value,
                "parentId":"AX-1", "childIds":[],
            });
            if let Some(properties) = &properties {
                expected["properties"] = json!(properties);
            }
            assert_eq!(
                serde_json::from_str::<Value>(&serialized).expect("AX JSON"),
                expected
            );
            assert_eq!(Value::from(node), expected);

            let role_position = serialized.find("\"role\"").expect("role");
            let reason_position = serialized.find("\"ignoredReasons\"").expect("reasons");
            assert_eq!(role_position < reason_position, properties.is_none());
            assert!(serialized.find("\"parentId\"") < serialized.find("\"childIds\""));
        }
    }

    #[test]
    fn list_markers_and_child_frame_roots_keep_their_distinct_payloads() {
        let mut marker =
            AccessibilityNode::new(AccessibilityNodeId::ListMarker(id(7)), "ListMarker");
        marker.name = Some("• ".to_owned());
        marker.parent_id = Some(AccessibilityNodeId::Dom(id(7)));
        marker.child_ids = Some(Vec::new());
        let expected = json!({
            "nodeId":"AX-LM-7", "ignored":false,
            "role":{"type":"role", "value":"ListMarker"},
            "name":{"type":"computedString", "value":"• "},
            "properties":[], "parentId":"AX-7", "childIds":[],
        });
        assert!(marker.rare_data.is_none());
        assert_eq!(
            serde_json::to_value(&marker).expect("marker JSON"),
            expected
        );
        assert_eq!(Value::from(marker), expected);

        let mut root = AccessibilityNode::new(AccessibilityNodeId::Dom(id(8)), "RootWebArea");
        root.parent_id = Some(AccessibilityNodeId::Dom(id(7)));
        root.child_ids = Some(vec![AccessibilityNodeId::Dom(id(9))]);
        let serialized = serde_json::to_string(&root).expect("root JSON");
        assert!(serialized.find("\"childIds\"") < serialized.find("\"parentId\""));
        assert_eq!(
            serde_json::from_str::<Value>(&serialized).expect("root JSON"),
            Value::from(root)
        );
    }

    #[test]
    fn ids_preserve_the_full_nonzero_u32_range_and_marker_namespace() {
        for value in [1, 2_000_000_000, u32::MAX] {
            let dom = AccessibilityNodeId::Dom(id(value));
            let marker = AccessibilityNodeId::ListMarker(id(value));
            assert_ne!(dom, marker);
            assert_eq!(dom.to_string(), format!("AX-{value}"));
            assert_eq!(marker.to_string(), format!("AX-LM-{value}"));
        }
    }
}
