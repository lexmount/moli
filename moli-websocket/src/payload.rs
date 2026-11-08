use moli_file_api::BlobData;
use tokio::sync::oneshot;

/// A source retains its message's existing FIFO position while I/O runs on
/// the Blob backend. Preparing it does not block the socket owner.
pub(crate) enum Payload {
    Bytes(Vec<u8>),
    Blob(BlobData),
    Reading {
        length: usize,
        result: oneshot::Receiver<std::io::Result<Vec<u8>>>,
    },
}

impl Payload {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Bytes(bytes) => bytes.len(),
            Self::Blob(blob) => blob.len(),
            Self::Reading { length, .. } => *length,
        }
    }

    pub(crate) fn start_read(&mut self) {
        if !matches!(self, Self::Blob(_)) {
            return;
        }
        let Self::Blob(blob) = std::mem::replace(self, Self::Bytes(Vec::new())) else {
            unreachable!();
        };
        let (sender, result) = oneshot::channel();
        *self = Self::Reading {
            length: blob.len(),
            result,
        };
        blob.read_all(Box::new(move |bytes| {
            let _ = sender.send(bytes);
        }));
    }

    pub(crate) fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }

    pub(crate) fn is_reading(&self) -> bool {
        matches!(self, Self::Reading { .. })
    }

    pub(crate) async fn finish_read(&mut self) -> Result<(), String> {
        let Self::Reading { length, result } = self else {
            unreachable!("only a pending Blob read is awaited");
        };
        let bytes = result
            .await
            .map_err(|_| "WebSocket Blob read stopped".to_owned())?
            .map_err(|error| format!("WebSocket Blob read failed: {error}"))?;
        if bytes.len() != *length {
            return Err("WebSocket Blob read length changed".to_owned());
        }
        *self = Self::Bytes(bytes);
        Ok(())
    }
}
