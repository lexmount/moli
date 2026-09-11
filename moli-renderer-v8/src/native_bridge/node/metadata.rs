use crate::dom::native::LiveDomNodeMetadata;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct NodeMetadataDeclaration<'scope> {
    kind: v8::Local<'scope, v8::Value>,
    node_type: u8,
    node_name: v8::Local<'scope, v8::Value>,
    local_name: v8::Local<'scope, v8::Value>,
    namespace: v8::Local<'scope, v8::Value>,
    connected: bool,
    data: v8::Local<'scope, v8::Value>,
    target: v8::Local<'scope, v8::Value>,
}

pub(super) fn build_node_metadata_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    metadata: &LiveDomNodeMetadata,
) -> Option<v8::Local<'s, v8::Object>> {
    NodeMetadataDeclaration::new(
        string_value(scope, metadata.kind)?,
        metadata.node_type,
        string_value(scope, &metadata.node_name)?,
        nullable_string_value(scope, metadata.local_name.as_deref())?,
        nullable_string_value(scope, metadata.namespace.as_deref())?,
        metadata.connected,
        nullable_string_value(scope, metadata.data.as_deref())?,
        nullable_string_value(scope, metadata.target.as_deref())?,
    )
    .bind(scope)
    .ok()
}

fn nullable_string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<&str>,
) -> Option<v8::Local<'s, v8::Value>> {
    match value {
        Some(value) => string_value(scope, value),
        None => Some(v8::null(scope).into()),
    }
}

fn string_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    v8::String::new(scope, value).map(Into::into)
}

#[cfg(test)]
mod tests {
    use std::pin::pin;

    use super::*;
    use crate::ensure_v8_for_test as ensure_v8;

    #[test]
    fn build_node_metadata_object_serializes_expected_shape() {
        ensure_v8();

        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);

        let metadata = LiveDomNodeMetadata {
            kind: "element",
            node_type: 1,
            node_name: "DIV".to_owned(),
            local_name: Some("div".to_owned()),
            namespace: Some("http://www.w3.org/1999/xhtml".to_owned()),
            connected: true,
            data: None,
            target: None,
        };

        let object = build_node_metadata_object(scope, &metadata).expect("metadata object");
        let actual: serde_json::Value =
            serde_v8::from_v8(scope, object.into()).expect("metadata JSON");

        assert_eq!(
            actual,
            serde_json::json!({
                "kind": "element",
                "nodeType": 1,
                "nodeName": "DIV",
                "localName": "div",
                "namespace": "http://www.w3.org/1999/xhtml",
                "connected": true,
                "data": null,
                "target": null,
            })
        );
    }
}
