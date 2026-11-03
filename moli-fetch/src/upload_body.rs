//! A single-use request body with bounded transport backpressure. Producers
//! never run on curl's owner; curl pauses while waiting for the next chunk.

use std::{
    fmt,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use bytes::{Buf, Bytes};
use curl::easy::ReadError;
use parking_lot::Mutex;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

type ReadWake = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy)]
enum Transmission {
    Pending,
    Sent,
    Failed,
}

struct Shared {
    cancelled: CancellationToken,
    reader_active: AtomicBool,
    read_wake: Mutex<Option<ReadWake>>,
    transmission: watch::Sender<Transmission>,
}

struct Body {
    receiver: Mutex<Option<mpsc::Receiver<Bytes>>>,
    shared: Arc<Shared>,
}

impl Shared {
    fn complete_transmission(&self, success: bool) {
        self.transmission.send_if_modified(|state| {
            if !matches!(state, Transmission::Pending) {
                return false;
            }
            *state = if success {
                Transmission::Sent
            } else {
                Transmission::Failed
            };
            true
        });
    }

    fn wake_reader(&self) {
        let wake = self.read_wake.lock().clone();
        if let Some(wake) = wake {
            wake();
        }
    }
}

impl Drop for Body {
    fn drop(&mut self) {
        if self.receiver.get_mut().take().is_some() {
            self.shared.complete_transmission(false);
        }
    }
}

/// Clones retain the same non-replayable body. Only one transfer may claim it.
#[derive(Clone)]
pub struct UploadBody(Arc<Body>);

impl fmt::Debug for UploadBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadBody").finish_non_exhaustive()
    }
}

/// The producer owns the body lifetime. Dropping it without `finish()` aborts
/// the transfer, including when its script owner is retired.
pub struct UploadBodySender {
    sender: Option<mpsc::Sender<Bytes>>,
    shared: Arc<Shared>,
}

impl fmt::Debug for UploadBodySender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadBodySender").finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadBodyError {
    Closed,
    AlreadyConsumed,
}

impl fmt::Display for UploadBodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Closed => "request upload stream is closed",
            Self::AlreadyConsumed => "request upload stream has already been consumed",
        })
    }
}

impl std::error::Error for UploadBodyError {}

impl UploadBody {
    /// Wait until the source reached EOF and curl transmitted its bytes.
    pub fn transmission(
        &self,
    ) -> impl Future<Output = Result<(), UploadBodyError>> + Send + 'static + use<> {
        transmission(&self.0.shared)
    }

    /// Queue at most one chunk ahead of curl's current chunk. The producer
    /// awaits `send()` before pulling another chunk from its source.
    pub fn channel() -> (UploadBodySender, Self) {
        let (sender, receiver) = mpsc::channel(1);
        let shared = Arc::new(Shared {
            cancelled: CancellationToken::new(),
            reader_active: AtomicBool::new(false),
            read_wake: Mutex::new(None),
            transmission: watch::channel(Transmission::Pending).0,
        });
        (
            UploadBodySender {
                sender: Some(sender),
                shared: shared.clone(),
            },
            Self(Arc::new(Body {
                receiver: Mutex::new(Some(receiver)),
                shared,
            })),
        )
    }

    pub(crate) fn claim_reader(&self) -> Result<UploadBodyReader, UploadBodyError> {
        let receiver = self
            .0
            .receiver
            .lock()
            .take()
            .ok_or(UploadBodyError::AlreadyConsumed)?;
        self.0.shared.reader_active.store(true, Ordering::Release);
        Ok(UploadBodyReader {
            receiver,
            current: Bytes::new(),
            shared: self.0.shared.clone(),
            read_bytes: 0,
            sent_bytes: 0,
            eof: false,
        })
    }

    pub(crate) fn bind_read_wake(&self, wake: impl Fn() + Send + Sync + 'static) {
        {
            let mut slot = self.0.shared.read_wake.lock();
            if !self.0.shared.reader_active.load(Ordering::Acquire) {
                return;
            }
            *slot = Some(Arc::new(wake));
        }
        // Data or EOF can arrive between submission and binding this wakeup.
        self.0.shared.wake_reader();
    }
}

// The last producer capability can live in a send future. Its cancellation
// must notify curl after dropping the channel sender, just like finish().
struct PendingSend {
    sender: Option<mpsc::Sender<Bytes>>,
    shared: Arc<Shared>,
}

impl Drop for PendingSend {
    fn drop(&mut self) {
        drop(self.sender.take());
        self.shared.wake_reader();
    }
}

impl UploadBodySender {
    /// Wait for EOF to reach the transport and for every body byte to be sent.
    /// A headers-first response must not settle half-duplex fetch before this
    /// completes. The response body may still be arriving afterwards.
    pub fn transmission(
        &self,
    ) -> impl Future<Output = Result<(), UploadBodyError>> + Send + 'static + use<> {
        transmission(&self.shared)
    }

    pub fn send(
        &self,
        bytes: Bytes,
    ) -> impl Future<Output = Result<(), UploadBodyError>> + Send + 'static + use<> {
        let pending = PendingSend {
            sender: self.sender.clone(),
            shared: self.shared.clone(),
        };
        async move {
            let sender = pending.sender.as_ref().ok_or(UploadBodyError::Closed)?;
            if pending.shared.cancelled.is_cancelled() || sender.is_closed() {
                return Err(UploadBodyError::Closed);
            }
            // An empty Uint8Array is a chunk, not transport EOF.
            if !bytes.is_empty() {
                pending
                    .shared
                    .cancelled
                    .run_until_cancelled(sender.send(bytes))
                    .await
                    .ok_or(UploadBodyError::Closed)?
                    .map_err(|_| UploadBodyError::Closed)?;
            }
            Ok(())
        }
    }

    /// Finish after outstanding send futures have completed. Queued bytes
    /// remain readable before curl observes EOF.
    pub fn finish(mut self) {
        drop(self.sender.take());
        self.shared.wake_reader();
    }
}

fn transmission(
    shared: &Shared,
) -> impl Future<Output = Result<(), UploadBodyError>> + Send + 'static + use<> {
    let mut receiver = shared.transmission.subscribe();
    async move {
        loop {
            match *receiver.borrow_and_update() {
                Transmission::Sent => return Ok(()),
                Transmission::Failed => return Err(UploadBodyError::Closed),
                Transmission::Pending => {}
            }
            receiver
                .changed()
                .await
                .map_err(|_| UploadBodyError::Closed)?;
        }
    }
}

impl Drop for UploadBodySender {
    fn drop(&mut self) {
        if self.sender.take().is_some() {
            self.shared.cancelled.cancel();
            self.shared.complete_transmission(false);
            self.shared.wake_reader();
        }
    }
}

pub(crate) struct UploadBodyReader {
    receiver: mpsc::Receiver<Bytes>,
    current: Bytes,
    shared: Arc<Shared>,
    read_bytes: u64,
    sent_bytes: u64,
    eof: bool,
}

impl UploadBodyReader {
    pub(crate) fn read(&mut self, output: &mut [u8]) -> Result<usize, ReadError> {
        if self.shared.cancelled.is_cancelled() {
            return Err(ReadError::Abort);
        }
        while self.current.is_empty() {
            match self.receiver.try_recv() {
                Ok(bytes) => self.current = bytes,
                Err(mpsc::error::TryRecvError::Empty) => return Err(ReadError::Pause),
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.eof = true;
                    self.complete_if_sent();
                    return Ok(0);
                }
            }
        }
        let count = output.len().min(self.current.len());
        output[..count].copy_from_slice(&self.current[..count]);
        self.current.advance(count);
        self.read_bytes = self.read_bytes.saturating_add(count as u64);
        Ok(count)
    }

    pub(crate) fn observe_progress(&mut self, sent_bytes: u64) {
        self.sent_bytes = sent_bytes;
        self.complete_if_sent();
    }

    fn complete_if_sent(&self) {
        if self.eof && self.sent_bytes >= self.read_bytes && !self.shared.cancelled.is_cancelled() {
            self.shared.complete_transmission(true);
        }
    }

    pub(crate) fn complete_transport(&self, success: bool) {
        self.shared
            .complete_transmission(success && self.eof && !self.shared.cancelled.is_cancelled());
    }
}

impl Drop for UploadBodyReader {
    fn drop(&mut self) {
        self.shared.reader_active.store(false, Ordering::Release);
        self.shared.complete_transmission(false);
        drop(self.shared.read_wake.lock().take());
        self.receiver.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        pin::pin,
        sync::atomic::AtomicUsize,
        task::{Context, Poll, Waker},
    };

    fn pending<F: Future>(future: std::pin::Pin<&mut F>) {
        assert!(matches!(
            future.poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
    }

    #[tokio::test]
    async fn upload_body_backpressure_follows_transport_reads() {
        let (sender, body) = UploadBody::channel();
        let mut reader = body.claim_reader().unwrap();
        sender.send(Bytes::from_static(b"abcdefgh")).await.unwrap();
        let mut output = [0; 8];
        assert_eq!(reader.read(&mut output[..3]).unwrap(), 3);
        assert_eq!(&output[..3], b"abc");
        sender.send(Bytes::from_static(b"ijk")).await.unwrap();
        let mut third = pin!(sender.send(Bytes::from_static(b"lmno")));
        pending(third.as_mut());
        assert_eq!(reader.read(&mut output).unwrap(), 5);
        assert_eq!(&output[..5], b"defgh");
        pending(third.as_mut());
        assert_eq!(reader.read(&mut output).unwrap(), 3);
        assert_eq!(&output[..3], b"ijk");
        third.await.unwrap();
        sender.finish();
        assert_eq!(reader.read(&mut output).unwrap(), 4);
        assert_eq!(&output[..4], b"lmno");
        assert_eq!(reader.read(&mut output).unwrap(), 0);
    }

    #[tokio::test]
    async fn upload_body_abort_releases_waiting_producer() {
        let (sender, body) = UploadBody::channel();
        let mut reader = body.claim_reader().unwrap();
        sender.send(Bytes::from_static(b"first")).await.unwrap();
        let mut blocked = pin!(sender.send(Bytes::from_static(b"second")));
        pending(blocked.as_mut());
        drop(sender);
        assert_eq!(blocked.await, Err(UploadBodyError::Closed));
        assert!(matches!(reader.read(&mut [0; 8]), Err(ReadError::Abort)));
    }

    #[tokio::test]
    async fn upload_body_retired_receiver_releases_waiting_producer() {
        let (sender, body) = UploadBody::channel();
        let reader = body.claim_reader().unwrap();
        sender.send(Bytes::from_static(b"first")).await.unwrap();
        let mut blocked = pin!(sender.send(Bytes::from_static(b"second")));
        pending(blocked.as_mut());
        drop(reader);
        assert_eq!(blocked.await, Err(UploadBodyError::Closed));
    }

    #[tokio::test]
    async fn upload_body_unclaimed_request_drop_closes_producer() {
        let (sender, body) = UploadBody::channel();
        sender.send(Bytes::from_static(b"first")).await.unwrap();
        let mut blocked = pin!(sender.send(Bytes::from_static(b"second")));
        pending(blocked.as_mut());
        let transmitted = sender.transmission();
        drop(body);
        assert_eq!(blocked.await, Err(UploadBodyError::Closed));
        assert_eq!(transmitted.await, Err(UploadBodyError::Closed));
    }

    #[test]
    fn upload_body_pending_send_drop_notifies_eof() {
        let (sender, body) = UploadBody::channel();
        let mut reader = body.claim_reader().unwrap();
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = wakes.clone();
        body.bind_read_wake(move || {
            count.fetch_add(1, Ordering::SeqCst);
        });
        let unsent = sender.send(Bytes::from_static(b"never polled"));
        sender.finish();
        assert!(matches!(reader.read(&mut [0; 8]), Err(ReadError::Pause)));
        let before = wakes.load(Ordering::SeqCst);
        drop(unsent);
        assert!(wakes.load(Ordering::SeqCst) > before);
        assert_eq!(reader.read(&mut [0; 8]).unwrap(), 0);
    }

    #[tokio::test]
    async fn upload_body_empty_chunks_are_not_eof_and_clones_cannot_replay() {
        let (sender, body) = UploadBody::channel();
        let copy = body.clone();
        let mut reader = body.claim_reader().unwrap();
        assert!(matches!(
            copy.claim_reader(),
            Err(UploadBodyError::AlreadyConsumed)
        ));
        sender.send(Bytes::new()).await.unwrap();
        assert!(matches!(reader.read(&mut [0; 8]), Err(ReadError::Pause)));
        sender.send(Bytes::from_static(b"value")).await.unwrap();
        sender.finish();
        let mut output = [0; 8];
        assert_eq!(reader.read(&mut output).unwrap(), 5);
        assert_eq!(&output[..5], b"value");
        assert_eq!(reader.read(&mut output).unwrap(), 0);
    }

    #[tokio::test]
    async fn upload_body_transmission_waits_for_eof_and_sent_bytes() {
        let (sender, body) = UploadBody::channel();
        let mut reader = body.claim_reader().unwrap();
        let mut transmitted = pin!(sender.transmission());
        sender.send(Bytes::from_static(b"body")).await.unwrap();
        sender.finish();
        assert_eq!(reader.read(&mut [0; 4]).unwrap(), 4);
        reader.observe_progress(3);
        pending(transmitted.as_mut());
        assert_eq!(reader.read(&mut [0; 4]).unwrap(), 0);
        pending(transmitted.as_mut());
        reader.observe_progress(4);
        transmitted.await.unwrap();
    }

    #[tokio::test]
    async fn upload_body_retired_transport_releases_transmission_waiters() {
        let (sender, body) = UploadBody::channel();
        let reader = body.claim_reader().unwrap();
        let transmitted = sender.transmission();
        drop(reader);
        assert_eq!(transmitted.await, Err(UploadBodyError::Closed));
    }
}
