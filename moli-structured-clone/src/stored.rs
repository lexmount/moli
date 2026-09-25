use std::{any::Any, fmt, sync::Arc};

/// An immutable serialized value, with optional trusted native host-object
/// attachments (Blob data and storage capabilities, for example).
///
/// Attachments are separate from script-controlled wire bytes. `Send + Sync`
/// rules out realm-local JS handles. The embedding's codec is responsible for
/// their concrete type and validates capabilities when deserializing them.
#[derive(Clone)]
pub struct SerializedScriptValue(Arc<StoredValue>);

struct StoredValue {
    bytes: Vec<u8>,
    attachments: Box<dyn Any + Send + Sync>,
}

impl SerializedScriptValue {
    pub fn new(bytes: Vec<u8>, attachments: impl Any + Send + Sync) -> Self {
        Self(Arc::new(StoredValue {
            bytes,
            attachments: Box::new(attachments),
        }))
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0.bytes
    }

    pub fn attachments<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.0.attachments.downcast_ref()
    }
}

// Serialized state has snapshot identity. Cloning a history seed preserves
// that identity without copying bytes or comparing native capabilities.
impl PartialEq for SerializedScriptValue {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for SerializedScriptValue {}

impl fmt::Debug for SerializedScriptValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SerializedScriptValue")
            .field("byte_length", &self.0.bytes.len())
            .finish_non_exhaustive()
    }
}
