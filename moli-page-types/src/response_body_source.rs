use std::{fmt, future::Future, pin::Pin, sync::Arc};

use crate::SubresourceResponseBody;

/// Read access to an actual response held at the response decision boundary.
/// Implementations retain bytes once and keep transport ownership outside the
/// protocol consumer. A read may return a prefix before the response finishes.
pub trait SubresourceResponseBodyRead: Send + Sync {
    fn retained_memory_bytes(&self) -> usize;

    fn read(
        &self,
        offset: usize,
        size: usize,
    ) -> Pin<Box<dyn Future<Output = Result<(Vec<u8>, bool), String>> + Send + '_>>;
}

#[derive(Clone)]
pub enum SubresourceResponseBodySource {
    Complete(SubresourceResponseBody),
    Streaming(Arc<dyn SubresourceResponseBodyRead>),
}

impl SubresourceResponseBodySource {
    pub fn renderer_transport_retained_memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>().saturating_add(match self {
            Self::Complete(body) => body.renderer_transport_retained_memory_bytes(),
            Self::Streaming(body) => body.retained_memory_bytes(),
        })
    }

    pub async fn read(&self, offset: usize, size: usize) -> Result<(Vec<u8>, bool), String> {
        match self {
            Self::Complete(body) => {
                let bytes = body
                    .read_chunk(offset, size)
                    .map_err(|error| error.to_string())?;
                let eof = offset.saturating_add(bytes.len()) >= body.len();
                Ok((bytes, eof))
            }
            Self::Streaming(body) => body.read(offset, size).await,
        }
    }

    pub async fn materialize_bytes_limited(&self, limit: usize) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        loop {
            let (chunk, eof) = self.read(bytes.len(), 64 * 1024).await?;
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(format!(
                    "response body exceeds materialization limit of {limit} bytes"
                ));
            }
            bytes.extend(chunk);
            if eof {
                return Ok(bytes);
            }
        }
    }
}

impl From<SubresourceResponseBody> for SubresourceResponseBodySource {
    fn from(body: SubresourceResponseBody) -> Self {
        Self::Complete(body)
    }
}

impl fmt::Debug for SubresourceResponseBodySource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Complete(body) => body.fmt(formatter),
            Self::Streaming(_) => formatter.write_str("StreamingResponseBody"),
        }
    }
}

impl PartialEq for SubresourceResponseBodySource {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Complete(left), Self::Complete(right)) => left == right,
            (Self::Streaming(left), Self::Streaming(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }
}

impl Eq for SubresourceResponseBodySource {}
