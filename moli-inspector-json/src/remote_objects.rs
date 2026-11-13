use super::*;

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

    fn field(self, key: Field) -> Self {
        match (self, key) {
            (Self::Envelope, Field::Result) => Self::Response,
            (Self::Response, Field::Result) => Self::Result,
            (Self::Response, Field::Object | Field::Objects) => Self::RemoteObject,
            (Self::Response, Field::InternalProperties | Field::PrivateProperties) => {
                Self::Descriptors
            }
            (Self::Response, Field::ExceptionDetails) => Self::Exception,
            (Self::Exception, Field::Exception) => Self::RemoteObject,
            (Self::Descriptor, Field::Value | Field::Get | Field::Set | Field::Symbol) => {
                Self::RemoteObject
            }
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

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Result,
    Object,
    Objects,
    InternalProperties,
    PrivateProperties,
    ExceptionDetails,
    Exception,
    Value,
    Get,
    Set,
    Symbol,
    Type,
    Handle,
    Other,
}

impl Field {
    fn from_bytes(value: &[u8]) -> Self {
        match value {
            b"result" => Self::Result,
            b"object" => Self::Object,
            b"objects" => Self::Objects,
            b"internalProperties" => Self::InternalProperties,
            b"privateProperties" => Self::PrivateProperties,
            b"exceptionDetails" => Self::ExceptionDetails,
            b"exception" => Self::Exception,
            b"value" => Self::Value,
            b"get" => Self::Get,
            b"set" => Self::Set,
            b"symbol" => Self::Symbol,
            b"type" => Self::Type,
            b"objectId" | b"promiseObjectId" | b"errorObjectId" => Self::Handle,
            _ => Self::Other,
        }
    }
}

fn remote_type(value: &[u8]) -> bool {
    matches!(
        value,
        b"object"
            | b"function"
            | b"undefined"
            | b"string"
            | b"number"
            | b"boolean"
            | b"symbol"
            | b"bigint"
    )
}

pub(super) fn collect(
    message: &InspectorJson,
    max_depth: usize,
) -> serde_json::Result<Vec<String>> {
    match message {
        InspectorJson::Parsed(value) => Ok(collect_value(value, Role::Envelope, max_depth)),
        InspectorJson::Raw(raw) => {
            let mut walker = RawWalker::default();
            if let Some(depth) = max_depth.checked_sub(1) {
                for (key, value) in &raw.source.fields {
                    let field = Field::from_bytes(key.as_bytes());
                    if field == Field::Handle && value.get().starts_with('"') {
                        walker.handle(value.get())?;
                    }
                    walker.scan(value.get(), Role::Envelope.field(field), depth)?;
                }
            }
            Ok(finish(walker.ids))
        }
    }
}

pub(super) fn collect_envelope(value: &Value, max_depth: usize) -> Vec<String> {
    collect_value(value, Role::Envelope, max_depth)
}

pub(super) fn collect_result(value: &Value, max_depth: usize) -> Vec<String> {
    // The omitted envelope consumes one level in the common depth budget.
    collect_value(value, Role::Response, max_depth.saturating_sub(1))
}

fn collect_value(value: &Value, role: Role, max_depth: usize) -> Vec<String> {
    let mut ids = Vec::new();
    let mut stack = vec![(value, role, max_depth)];
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
                        .is_some_and(|value| remote_type(value.as_bytes()));
                for (key, value) in fields {
                    let field = Field::from_bytes(key.as_bytes());
                    if field == Field::Handle
                        && let Some(id) = value.as_str()
                    {
                        ids.push(id.to_owned());
                    }
                    if !(opaque_value && field == Field::Value) {
                        stack.push((value, role.field(field), next_depth));
                    }
                }
            }
            Value::Array(values) => {
                stack.extend(values.iter().map(|value| (value, role.item(), next_depth)))
            }
            _ => {}
        }
    }
    finish(ids)
}

fn finish(mut ids: Vec<String>) -> Vec<String> {
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[derive(Default)]
struct RawWalker {
    ids: Vec<String>,
    #[cfg(test)]
    walk_input_bytes: usize,
}

impl RawWalker {
    fn handle(&mut self, string: &str) -> serde_json::Result<()> {
        if let Value::String(id) = decode_lossy_json(string.as_bytes())? {
            self.ids.push(id);
        }
        Ok(())
    }

    fn scan(&mut self, source: &str, role: Role, depth: usize) -> serde_json::Result<()> {
        let mut work = vec![(source, role, depth)];
        let mut frames: Vec<Frame<'_>> = Vec::new();
        while let Some((source, root_role, root_depth)) = work.pop() {
            // Sum work-item input lengths, including deferred untyped values.
            // This excludes reads within string searches and serde value skips.
            #[cfg(test)]
            {
                self.walk_input_bytes += source.len();
            }
            let mut cursor = Cursor {
                source,
                position: 0,
            };
            while let Some(byte) = cursor.byte() {
                match byte {
                    b' ' | b'\t' | b'\r' | b'\n' | b',' | b':' => cursor.position += 1,
                    b'}' | b']' => {
                        let frame = frames.pop().expect("validated container");
                        if let Container::Object {
                            opaque_value: false,
                            deferred_value: Some(value),
                            ..
                        } = frame.container
                        {
                            work.push((value, Role::Other, frame.depth - 1));
                        }
                        cursor.position += 1;
                    }
                    _ => {
                        if let Some(Frame {
                            container: Container::Object { field, .. },
                            ..
                        }) = frames.last_mut()
                            && field.is_none()
                        {
                            *field = Some(Keyword::parse(cursor.string())?.field);
                            continue;
                        }
                        let (role, depth, field, remote_parent) = match frames.last_mut() {
                            Some(frame) => match &mut frame.container {
                                Container::Object { field, .. } => {
                                    let field = field.take().expect("validated object value");
                                    (
                                        frame.role.field(field),
                                        frame.depth - 1,
                                        field,
                                        frame.role.remote_object(),
                                    )
                                }
                                Container::Array => {
                                    (frame.role.item(), frame.depth - 1, Field::Other, false)
                                }
                            },
                            None => (root_role, root_depth, Field::Other, false),
                        };
                        if remote_parent {
                            let Some(Frame {
                                container:
                                    Container::Object {
                                        opaque_value,
                                        deferred_value,
                                        ..
                                    },
                                ..
                            }) = frames.last_mut()
                            else {
                                unreachable!("remote object parent");
                            };
                            if field == Field::Value {
                                // Type may follow value. Skip its bytes only once;
                                // untyped extension values get one later Other pass.
                                let value = cursor.skip_value()?;
                                if depth > 0 {
                                    *deferred_value = Some(value);
                                }
                                continue;
                            }
                            if field == Field::Type {
                                *opaque_value =
                                    byte == b'"' && Keyword::parse(cursor.string())?.remote_type;
                                if byte == b'"' {
                                    continue;
                                }
                            }
                        }
                        if field == Field::Handle && byte == b'"' {
                            self.handle(cursor.string())?;
                        } else if depth == 0 {
                            cursor.skip_value()?;
                        } else {
                            match byte {
                                b'{' | b'[' => {
                                    frames.push(Frame {
                                        role,
                                        depth,
                                        container: if byte == b'{' {
                                            Container::Object {
                                                field: None,
                                                opaque_value: false,
                                                deferred_value: None,
                                            }
                                        } else {
                                            Container::Array
                                        },
                                    });
                                    cursor.position += 1;
                                }
                                b'"' => {
                                    cursor.string();
                                }
                                _ => cursor.atom(),
                            }
                        }
                    }
                }
            }
            debug_assert!(frames.is_empty());
        }
        Ok(())
    }
}

struct Frame<'a> {
    role: Role,
    depth: usize,
    container: Container<'a>,
}

enum Container<'a> {
    Object {
        field: Option<Field>,
        opaque_value: bool,
        deferred_value: Option<&'a str>,
    },
    Array,
}

// Only walks immutable RawValue text validated by InspectorJson::from_slice.
// This is a structural cursor, not another JSON validator: serde still owns
// validation and string decoding. Containers use an explicit stack so even
// unknown fields and standard deepSerializedValue are linear, without recursion.
struct Cursor<'a> {
    source: &'a str,
    position: usize,
}

impl<'a> Cursor<'a> {
    fn byte(&self) -> Option<u8> {
        self.source.as_bytes().get(self.position).copied()
    }

    fn string(&mut self) -> &'a str {
        let start = self.position;
        debug_assert_eq!(self.byte(), Some(b'"'));
        self.position += 1;
        loop {
            self.position += self.source[self.position..]
                .find('"')
                .expect("validated string")
                + 1;
            let escapes = self.source.as_bytes()[..self.position - 1]
                .iter()
                .rev()
                .take_while(|&&byte| byte == b'\\')
                .count();
            if escapes % 2 == 0 {
                break;
            }
        }
        &self.source[start..self.position]
    }

    fn atom(&mut self) {
        self.position += self.source[self.position..]
            .find([',', '}', ']', ' ', '\t', '\r', '\n'])
            .unwrap_or(self.source.len() - self.position);
    }

    fn skip_value(&mut self) -> serde_json::Result<&'a str> {
        // Cutting out an opaque value once is linear. Do not do this at every
        // level of traversed protocol structures: that would rescan descendants.
        let mut values = serde_json::Deserializer::from_str(&self.source[self.position..])
            .into_iter::<&RawValue>();
        let value = values.next().expect("validated value")?;
        self.position += values.byte_offset();
        Ok(value.get())
    }
}

struct Keyword {
    field: Field,
    remote_type: bool,
}

impl Keyword {
    fn from_bytes(value: &[u8]) -> Self {
        Self {
            field: Field::from_bytes(value),
            remote_type: remote_type(value),
        }
    }

    fn parse(string: &str) -> serde_json::Result<Self> {
        if !string.as_bytes().contains(&b'\\') {
            return Ok(Self::from_bytes(&string.as_bytes()[1..string.len() - 1]));
        }
        // deserialize_bytes also accepts escaped lone surrogates as WTF-8;
        // unknown field names must not force a lossy materialized Value tree.
        struct Name;
        impl Visitor<'_> for Name {
            type Value = Keyword;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a protocol keyword")
            }
            fn visit_bytes<E: serde::de::Error>(self, value: &[u8]) -> Result<Self::Value, E> {
                Ok(Keyword::from_bytes(value))
            }
        }
        serde_json::Deserializer::from_str(string).deserialize_bytes(Name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_serialized_and_unknown_values_do_not_add_deferred_walks() {
        for depth in [0, 10, 20, 40, 80, 499, 500, 1000, 2000] {
            let leaf = "x".repeat(1024 * 1024);
            let deep = format!(
                "{}{{\"objectId\":\"deep\",\"text\":\"{leaf}\"}}{}",
                "{\"type\":\"array\",\"value\":[".repeat(depth),
                "]}".repeat(depth)
            );
            for field in ["deepSerializedValue", "extension"] {
                let source = format!(r#"{{"type":"object","objectId":"root","{field}":{deep}}}"#);
                let raw: Box<RawValue> = serde_json::from_str(&source).unwrap();
                let mut walker = RawWalker::default();
                walker.scan(raw.get(), Role::RemoteObject, 1000).unwrap();
                assert_eq!(
                    walker.walk_input_bytes,
                    source.len(),
                    "{field}, depth {depth}"
                );
                let expected = if depth <= 499 {
                    vec!["deep", "root"]
                } else {
                    vec!["root"]
                };
                assert_eq!(finish(walker.ids), expected);
            }
        }
    }
}
