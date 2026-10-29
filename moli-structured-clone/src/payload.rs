/// Serialized structured clone payload plus transferred host-object metadata.
///
/// `PortId` is generic so the payload crate does not need to own MessagePort id
/// allocation or depend on the MessagePort registry crate. Embedders may use
/// ids or richer transfer descriptors.
#[derive(Clone, Debug)]
pub struct StructuredCloneBytes<PortId = u64> {
    /// Wire bytes produced by the embedding engine's structured clone encoder.
    pub wire_bytes: Vec<u8>,
    /// ArrayBuffer transfers captured before detaching the sender-side buffers.
    pub transferred_array_buffers: Vec<TransferredArrayBuffer>,
    /// MessagePort transfer descriptors captured by the embedding engine.
    pub transferred_message_ports: Vec<PortId>,
}

impl<PortId> Default for StructuredCloneBytes<PortId> {
    fn default() -> Self {
        Self {
            wire_bytes: Vec::new(),
            transferred_array_buffers: Vec::new(),
            transferred_message_ports: Vec::new(),
        }
    }
}

/// Sender-side bytes for a transferred ArrayBuffer.
#[derive(Clone, Debug)]
pub struct TransferredArrayBuffer {
    /// Transfer id used by the embedding engine's structured clone wire format.
    pub transfer_id: u32,
    /// Captured buffer bytes.
    pub bytes: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_payload_does_not_require_default_transfer_descriptors() {
        enum Descriptor {}
        let payload = StructuredCloneBytes::<Descriptor>::default();
        assert!(payload.wire_bytes.is_empty());
        assert!(payload.transferred_array_buffers.is_empty());
        assert!(payload.transferred_message_ports.is_empty());
    }
}
