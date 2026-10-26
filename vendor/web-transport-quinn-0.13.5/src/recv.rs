use crate::error::CloseReason;
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{ready, Context, Poll},
};

use bytes::Bytes;

use crate::{
    error::{decode_stream_code, encode_stream_code},
    ReadError, ReadExactError, ReadToEndError, SessionError,
};

/// A stream that can be used to recieve bytes. See [`quinn::RecvStream`].
#[derive(Debug)]
pub struct RecvStream {
    inner: quinn::RecvStream,
    error: Arc<CloseReason>,
    // Raw QUIC carries stream codes as is; HTTP/3 maps them into its own code space.
    raw: bool,
}

impl RecvStream {
    pub(crate) fn new(stream: quinn::RecvStream, error: Arc<CloseReason>, raw: bool) -> Self {
        Self {
            inner: stream,
            error,
            raw,
        }
    }

    /// Decode the peer's reset code, and replace connection-level errors with the stored
    /// session error if available.
    fn map_error(&self, e: quinn::ReadError) -> ReadError {
        let e = match e {
            quinn::ReadError::Reset(code) => match decode_stream_code(code, self.raw) {
                Some(code) => ReadError::Reset(code),
                None => ReadError::InvalidReset(code),
            },
            e => e.into(),
        };
        if let Some(err) = self.error.get() {
            if matches!(&e, ReadError::SessionError(_) | ReadError::InvalidReset(_)) {
                return ReadError::SessionError(err.clone());
            }
        }
        match e {
            ReadError::SessionError(err) => ReadError::SessionError(self.error.map(err)),
            err => err,
        }
    }

    /// Tell the other end to stop sending data with the given error code. See [`quinn::RecvStream::stop`].
    /// This is a u32 with WebTransport since it shares the error space with HTTP/3.
    /// A raw QUIC session sends the code as is.
    pub fn stop(&mut self, code: u32) -> Result<(), quinn::ClosedStream> {
        let code = encode_stream_code(code, self.raw);
        self.inner.stop(code)
    }

    // Unfortunately, we have to wrap ReadError for a bunch of functions.

    /// Read some data into the buffer and return the amount read. See [`quinn::RecvStream::read`].
    pub async fn read(&mut self, buf: &mut [u8]) -> Result<Option<usize>, ReadError> {
        self.inner.read(buf).await.map_err(|e| self.map_error(e))
    }

    /// Fill the entire buffer with data. See [`quinn::RecvStream::read_exact`].
    pub async fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), ReadExactError> {
        self.inner.read_exact(buf).await.map_err(|e| match e {
            quinn::ReadExactError::ReadError(e) => self.map_error(e).into(),
            e => e.into(),
        })
    }

    /// Read a chunk of data from the stream. See [`quinn::RecvStream::read_chunk`].
    pub async fn read_chunk(
        &mut self,
        max_length: usize,
        ordered: bool,
    ) -> Result<Option<quinn::Chunk>, ReadError> {
        self.inner
            .read_chunk(max_length, ordered)
            .await
            .map_err(|e| self.map_error(e))
    }

    /// Read chunks of data from the stream. See [`quinn::RecvStream::read_chunks`].
    pub async fn read_chunks(&mut self, bufs: &mut [Bytes]) -> Result<Option<usize>, ReadError> {
        self.inner
            .read_chunks(bufs)
            .await
            .map_err(|e| self.map_error(e))
    }

    /// Read until the end of the stream or the limit is hit. See [`quinn::RecvStream::read_to_end`].
    pub async fn read_to_end(&mut self, size_limit: usize) -> Result<Vec<u8>, ReadToEndError> {
        self.inner
            .read_to_end(size_limit)
            .await
            .map_err(|e| match e {
                quinn::ReadToEndError::Read(e) => self.map_error(e).into(),
                e => e.into(),
            })
    }

    /// Block until the stream has been reset and return the error code. See [`quinn::RecvStream::received_reset`].
    ///
    /// Unlike Quinn, this returns a SessionError, not a ResetError, because 0-RTT is not supported.
    pub async fn received_reset(&mut self) -> Result<Option<u32>, SessionError> {
        match self.inner.received_reset().await {
            Ok(None) => Ok(None),
            Ok(Some(code)) => Ok(decode_stream_code(code, self.raw)),
            Err(quinn::ResetError::ConnectionLost(conn_err)) => {
                Err(self.error.map(conn_err.into()))
            }
            Err(quinn::ResetError::ZeroRttRejected) => unreachable!("0-RTT not supported"),
        }
    }

    /// Return the underlying QUIC stream ID.
    ///
    /// > **Warning**
    /// >
    /// > WebTransport sessions share the QUIC connection with HTTP/3 and potentially other sessions.
    /// > The [quinn::StreamId::index] might not increment by 1 like expected when using [quinn].
    /// > This is why the Javascript WebTransport API does not expose the Stream ID.
    pub fn quic_id(&self) -> quinn::StreamId {
        self.inner.id()
    }

    // We purposely don't expose the 0RTT because it's not valid with WebTransport
}

impl tokio::io::AsyncRead for RecvStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl web_transport_trait::RecvStream for RecvStream {
    type Error = ReadError;

    fn stop(&mut self, code: u32) {
        Self::stop(self, code).ok();
    }

    async fn read(&mut self, dst: &mut [u8]) -> Result<Option<usize>, Self::Error> {
        self.read(dst).await
    }

    async fn read_chunk(&mut self, max: usize) -> Result<Option<Bytes>, Self::Error> {
        self.read_chunk(max, true)
            .await
            .map(|r| r.map(|chunk| chunk.bytes))
    }

    async fn closed(&mut self) -> Result<(), Self::Error> {
        self.received_reset().await?;
        Ok(())
    }
}

impl web_transport_trait::poll::RecvStream for RecvStream {
    type Error = ReadError;

    fn poll_read(
        &mut self,
        cx: &mut Context<'_>,
        dst: &mut [u8],
    ) -> Poll<Result<Option<usize>, Self::Error>> {
        if dst.is_empty() {
            // Asking for no bytes is not end of stream.
            return Poll::Ready(Ok(Some(0)));
        }

        let size = ready!(quinn::RecvStream::poll_read(&mut self.inner, cx, dst))
            .map_err(|e| self.map_error(e))?;

        // Quinn reports a finished stream as zero bytes into a non-empty buffer.
        Poll::Ready(Ok((size != 0).then_some(size)))
    }

    fn stop(&mut self, code: u32) {
        Self::stop(self, code).ok();
    }

    fn poll_closed(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Nothing to retain: Quinn parks this waker in the connection's
        // `blocked_readers` map keyed by stream ID, not inside the future, so the
        // registration outlives the future we build here and drop again.
        let mut reset = std::pin::pin!(self.received_reset());
        ready!(reset.as_mut().poll(cx))?;
        Poll::Ready(Ok(()))
    }
}
