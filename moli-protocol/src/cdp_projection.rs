use crate::automation::DevToolsRemoteValue;
use moli_protocol_cdp::remote_object_from_json_value;
use serde_json::{Value, json};

pub(crate) fn remote_object_from_devtools(value: DevToolsRemoteValue) -> Value {
    let DevToolsRemoteValue {
        value,
        handle,
        shared_id: _,
        node_id: _,
        backend_node_id: _,
        window_context: _,
        realm: _,
        remote_type,
        remote_subtype,
        unserializable_value,
        description,
        class_name,
        deep_serialized_value,
        node_value: _,
    } = value;
    let Some(remote_type) = remote_type else {
        return remote_object_from_json_value(value, handle.map(|handle| handle.into_string()));
    };

    let mut remote = json!({
        "type": remote_type.clone(),
    });
    if let Some(map) = remote.as_object_mut() {
        if let Some(remote_subtype) = remote_subtype {
            map.insert("subtype".to_owned(), json!(remote_subtype));
        }
        if let Some(unserializable_value) = unserializable_value {
            map.insert(
                "unserializableValue".to_owned(),
                json!(unserializable_value),
            );
        } else if !matches!(remote_type.as_str(), "undefined") {
            map.insert("value".to_owned(), value);
        }
        if let Some(description) = description {
            map.insert("description".to_owned(), json!(description));
        }
        if let Some(class_name) = class_name {
            map.insert("className".to_owned(), json!(class_name));
        }
        if let Some(deep_serialized_value) = deep_serialized_value {
            map.insert("deepSerializedValue".to_owned(), deep_serialized_value);
        }
        if let Some(handle) = handle {
            map.insert("objectId".to_owned(), json!(handle.into_string()));
        }
    }
    remote
}
