use super::*;
use serde::{Deserialize, de::SeqAccess};

#[derive(Clone, Copy)]
enum Role {
    Envelope,
    Response,
    // Evaluation returns a RemoteObject; getProperties returns descriptors.
    Result,
    RemoteObject,
    Descriptors,
    Descriptor,
    Exception,
    Other,
}

impl Role {
    fn remote_object(self) -> bool {
        matches!(self, Self::RemoteObject | Self::Result)
    }

    fn field(self, key: &[u8]) -> Self {
        match (self, key) {
            (Self::Envelope, b"result") => Self::Response,
            (Self::Response, b"result") => Self::Result,
            (Self::Response, b"object" | b"objects") => Self::RemoteObject,
            (Self::Response, b"internalProperties" | b"privateProperties") => Self::Descriptors,
            (Self::Response, b"exceptionDetails") => Self::Exception,
            (Self::Exception, b"exception") => Self::RemoteObject,
            (Self::Descriptor, b"value" | b"get" | b"set" | b"symbol") => Self::RemoteObject,
            _ => Self::Other,
        }
    }

    fn item(self) -> Self {
        match self {
            Self::Result | Self::Descriptors => Self::Descriptor,
            _ => Self::Other,
        }
    }
}

fn remote_type(value: &str) -> bool {
    matches!(
        value,
        "object" | "function" | "undefined" | "string" | "number" | "boolean" | "symbol" | "bigint"
    )
}

fn handle_key(key: &[u8]) -> bool {
    matches!(key, b"objectId" | b"promiseObjectId" | b"errorObjectId")
}

pub(super) fn collect(
    message: &InspectorJson,
    max_depth: usize,
) -> serde_json::Result<Vec<String>> {
    let mut ids = Vec::new();
    match message {
        InspectorJson::Parsed(value) => {
            let mut stack = vec![(value, Role::Envelope, max_depth)];
            while let Some((value, role, depth)) = stack.pop() {
                let Some(next_depth) = depth.checked_sub(1) else {
                    continue;
                };
                match value {
                    Value::Object(fields) => {
                        let opaque_value = role.remote_object()
                            && fields
                                .get("type")
                                .and_then(Value::as_str)
                                .is_some_and(remote_type);
                        for (key, value) in fields {
                            if handle_key(key.as_bytes())
                                && let Some(id) = value.as_str()
                            {
                                ids.push(id.to_owned());
                            }
                            if !(opaque_value && key == "value") {
                                stack.push((value, role.field(key.as_bytes()), next_depth));
                            }
                        }
                    }
                    Value::Array(values) => {
                        stack.extend(values.iter().map(|value| (value, role.item(), next_depth)))
                    }
                    _ => {}
                }
            }
        }
        InspectorJson::Raw(raw) => {
            if let Some(depth) = max_depth.checked_sub(1) {
                let mut stack = Vec::new();
                for (key, value) in &raw.source.fields {
                    push(
                        &mut stack,
                        value,
                        Role::Envelope.field(key.as_bytes()),
                        depth,
                    );
                }
                while let Some((value, role, depth)) = stack.pop() {
                    let mut parser = serde_json::Deserializer::from_str(value.get());
                    let visitor = Fields {
                        role,
                        depth,
                        stack: &mut stack,
                        ids: &mut ids,
                    };
                    visitor.deserialize(&mut parser)?;
                    parser.end()?;
                }
            }
        }
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

type Work<'a> = Vec<(&'a RawValue, Role, usize)>;

fn push<'a>(stack: &mut Work<'a>, value: &'a RawValue, role: Role, depth: usize) {
    if depth > 0
        && matches!(
            value.get().trim_start().as_bytes().first(),
            Some(b'{' | b'[')
        )
    {
        stack.push((value, role, depth));
    }
}

struct Fields<'a, 'de> {
    role: Role,
    depth: usize,
    stack: &'a mut Work<'de>,
    ids: &'a mut Vec<String>,
}

impl<'de> DeserializeSeed<'de> for Fields<'_, 'de> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, parser: D) -> Result<(), D::Error> {
        if self.depth == 0 {
            IgnoredAny::deserialize(parser).map(|_| ())
        } else {
            parser.deserialize_any(self)
        }
    }
}

impl<'de> Visitor<'de> for Fields<'_, 'de> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("Inspector protocol fields")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut fields: M) -> Result<(), M::Error> {
        let mut opaque_value = false;
        let mut deferred_value = None;
        while let Some(key) = fields.next_key_seed(FieldName)? {
            let role = self.role.field(&key);
            if !matches!(role, Role::Other) {
                // Stream the bounded protocol schema once. Unknown structures
                // use the work stack so their nesting cannot recurse in Rust.
                fields.next_value_seed(Fields {
                    role,
                    depth: self.depth - 1,
                    stack: &mut *self.stack,
                    ids: &mut *self.ids,
                })?;
                continue;
            }
            let value: &'de RawValue = fields.next_value()?;
            if key == b"type" && self.role.remote_object() {
                opaque_value = serde_json::from_str::<String>(value.get())
                    .ok()
                    .is_some_and(|value| remote_type(&value));
            }
            if handle_key(&key) && value.get().trim_start().starts_with('"') {
                let id =
                    decode_lossy_json(value.get().as_bytes()).map_err(serde::de::Error::custom)?;
                if let Some(id) = id.as_str() {
                    self.ids.push(id.to_owned());
                }
            }
            if key == b"value" && self.role.remote_object() {
                // Type and value may occur in either order in the envelope.
                deferred_value = Some(value);
            } else {
                push(self.stack, value, role, self.depth - 1);
            }
        }
        if !opaque_value && let Some(value) = deferred_value {
            push(self.stack, value, Role::Other, self.depth - 1);
        }
        Ok(())
    }

    fn visit_seq<S: SeqAccess<'de>>(self, mut values: S) -> Result<(), S::Error> {
        let role = self.role.item();
        if matches!(role, Role::Other) {
            while let Some(value) = values.next_element::<&'de RawValue>()? {
                push(self.stack, value, role, self.depth - 1);
            }
        } else {
            while values
                .next_element_seed(Fields {
                    role,
                    depth: self.depth - 1,
                    stack: &mut *self.stack,
                    ids: &mut *self.ids,
                })?
                .is_some()
            {}
        }
        Ok(())
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
}

struct FieldName;

impl<'de> DeserializeSeed<'de> for FieldName {
    type Value = Vec<u8>;

    fn deserialize<D: Deserializer<'de>>(self, parser: D) -> Result<Self::Value, D::Error> {
        parser.deserialize_bytes(self)
    }
}

impl Visitor<'_> for FieldName {
    type Value = Vec<u8>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("an Inspector field name")
    }

    fn visit_bytes<E: serde::de::Error>(self, value: &[u8]) -> Result<Self::Value, E> {
        Ok(value.to_vec())
    }
}
