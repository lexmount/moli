use std::sync::Arc;

use anyhow::{Result, anyhow, ensure};
use moli_fetch::{FetchCancelHandle, StreamingRawResponse};
use tokio::sync::Mutex;

use crate::types::{SubresourceResponseBody, SubresourceResponseBodyWriter};

/// A pending response body, independent of any Page, V8 context or decision
/// port. Reads start on demand and share the existing captured-body disk pool.
/// A fail/fulfill decision cancels the transfer without waiting for a reader.
#[derive(Clone, Debug)]
pub struct ResourceInterceptionBody {
    state: Arc<Mutex<BodyState>>,
    cancel: FetchCancelHandle,
}

impl PartialEq for ResourceInterceptionBody {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

impl Eq for ResourceInterceptionBody {}

#[derive(Debug)]
enum BodyState {
    Receiving {
        response: Box<StreamingRawResponse>,
        captured: SubresourceResponseBodyWriter,
    },
    Complete(SubresourceResponseBody),
    Failed(String),
}

impl ResourceInterceptionBody {
    pub(crate) fn new(
        response: StreamingRawResponse,
        disk_pool: Option<moli_disk_pool::DiskPool>,
    ) -> Self {
        Self {
            cancel: response.cancellation_handle(),
            state: Arc::new(Mutex::new(BodyState::Receiving {
                response: Box::new(response),
                captured: SubresourceResponseBodyWriter::with_disk_pool(disk_pool),
            })),
        }
    }

    pub(crate) fn from_complete(body: SubresourceResponseBody) -> Self {
        Self {
            cancel: FetchCancelHandle::new(),
            state: Arc::new(Mutex::new(BodyState::Complete(body))),
        }
    }

    /// Retire this transfer, including a read currently waiting for body data.
    /// This does not consume or resume the interception decision port.
    pub fn cancel(&self) {
        self.cancel.cancel();
        if let Ok(mut state) = self.state.try_lock()
            && matches!(*state, BodyState::Receiving { .. })
        {
            // Retained protocol handles must not keep a retired transport or
            // its runtime lease alive. An active reader takes this same path
            // through receive() when cancellation wins its wait.
            *state = BodyState::Failed("net::ERR_ABORTED".into());
        }
    }

    pub async fn capture(&self, limit: usize) -> Result<SubresourceResponseBody> {
        let mut state = tokio::select! {
            biased;
            () = self.cancel.cancelled() => return Err(anyhow!("net::ERR_ABORTED")),
            state = self.state.lock() => state,
        };
        loop {
            match &*state {
                BodyState::Complete(body) => {
                    ensure!(
                        body.len() <= limit,
                        "response body exceeds materialization limit"
                    );
                    return Ok(body.clone());
                }
                BodyState::Failed(error) => return Err(anyhow!(error.clone())),
                BodyState::Receiving { captured, .. } => {
                    ensure!(
                        captured.len() <= limit,
                        "response body exceeds materialization limit"
                    );
                }
            }
            let _ = self.receive(&mut state).await?;
        }
    }

    /// Returns available bytes without waiting for EOF. Only a read beyond the
    /// currently captured prefix waits for the next body chunk or terminal.
    pub async fn read_chunk(&self, offset: usize, size: usize) -> Result<(Vec<u8>, bool)> {
        let mut state = tokio::select! {
            biased;
            () = self.cancel.cancelled() => return Err(anyhow!("net::ERR_ABORTED")),
            state = self.state.lock() => state,
        };
        loop {
            match &*state {
                BodyState::Complete(body) => {
                    let bytes = body.read_chunk(offset, size)?;
                    let eof = offset.saturating_add(bytes.len()) >= body.len();
                    return Ok((bytes, eof));
                }
                BodyState::Failed(error) => return Err(anyhow!(error.clone())),
                BodyState::Receiving { captured, .. } if offset < captured.len() || size == 0 => {
                    return Ok((captured.read_chunk(offset, size)?, false));
                }
                BodyState::Receiving { captured, .. } => {
                    let start = captured.len();
                    if let Some(bytes) = self.receive(&mut state).await?
                        && offset == start
                        && bytes.len() <= size
                    {
                        // The ordinary async XHR consumer can forward the
                        // original chunk instead of reading it back from disk.
                        return Ok((bytes, false));
                    }
                }
            }
        }
    }

    async fn receive(&self, state: &mut BodyState) -> Result<Option<Vec<u8>>> {
        let BodyState::Receiving { response, captured } = state else {
            return Ok(None);
        };
        let next = tokio::select! {
            biased;
            () = self.cancel.cancelled() => Err(anyhow!("net::ERR_ABORTED")),
            next = async {
                if let Some(bytes) = response.next_chunk().await {
                    return Ok(Some(bytes));
                }
                response.finish().await?;
                Ok(None)
            } => next,
        };
        match next {
            Ok(Some(bytes)) => {
                captured.append(&bytes);
                return Ok(Some(bytes));
            }
            Ok(None) => {
                let captured = std::mem::take(captured).finish();
                *state = BodyState::Complete(captured);
            }
            Err(error) => {
                *state = BodyState::Failed(format!("{error:#}"));
                return Err(error);
            }
        }
        Ok(None)
    }
}
