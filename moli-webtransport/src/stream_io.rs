//! A single owner polls each QUIC endpoint, including its peer-close notifier.
//! Receives stay demand-driven. Cancellation also interrupts blocked I/O and
//! FIN acknowledgement, rather than waiting behind the operation it cancels.

use std::{fmt, future::Future};

use futures_util::future::poll_fn;
use tokio::sync::{mpsc, oneshot, watch};

use crate::{ReadError, RecvStream, SendStream, WriteError};

#[derive(Clone, Debug)]
pub enum StreamIoError<E> {
    Cancelled,
    Closed,
    Transport(E),
}

impl<E: fmt::Display> fmt::Display for StreamIoError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("stream operation was cancelled"),
            Self::Closed => f.write_str("stream I/O driver is closed"),
            Self::Transport(error) => error.fmt(f),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for StreamIoError<E> {}

type End<E> = Result<(), StreamIoError<E>>;
type ReadResult = Result<Option<Vec<u8>>, StreamIoError<ReadError>>;

async fn wait_closed<E: Clone>(mut receiver: watch::Receiver<Option<End<E>>>) -> End<E> {
    loop {
        let result = receiver.borrow().clone();
        if let Some(result) = result {
            return result;
        }
        if receiver.changed().await.is_err() {
            return Err(StreamIoError::Closed);
        }
    }
}

fn cancel(sender: &watch::Sender<Option<u32>>, code: u32) {
    sender.send_if_modified(|value| {
        if value.is_some() {
            return false;
        }
        *value = Some(code);
        true
    });
}

struct Write {
    bytes: Option<Vec<u8>>,
    reply: oneshot::Sender<End<WriteError>>,
}

#[derive(Clone)]
pub struct SendStreamIo {
    requests: mpsc::Sender<Write>,
    cancel: watch::Sender<Option<u32>>,
    end: watch::Receiver<Option<End<WriteError>>>,
}

impl SendStreamIo {
    pub async fn write_all(&self, bytes: Vec<u8>) -> End<WriteError> {
        self.request(Some(bytes)).await
    }
    pub async fn finish(&self) -> End<WriteError> {
        self.request(None).await
    }
    pub fn reset(&self, code: u32) {
        cancel(&self.cancel, code);
    }

    /// The future retains only the notification channel, not a command sender.
    /// A passive observer must not prevent an unreachable endpoint being freed.
    pub fn closed(&self) -> impl Future<Output = End<WriteError>> + Send + 'static {
        wait_closed(self.end.clone())
    }

    async fn request(&self, bytes: Option<Vec<u8>>) -> End<WriteError> {
        let (reply, result) = oneshot::channel();
        if self.requests.send(Write { bytes, reply }).await.is_ok()
            && let Ok(result) = result.await
        {
            return result;
        }
        match self.closed().await {
            Ok(()) => Err(StreamIoError::Closed),
            result => result,
        }
    }
}

impl SendStream {
    /// Poll the returned future on the execution context's captured executor.
    pub fn into_io(self) -> (SendStreamIo, impl Future<Output = ()> + Send + 'static) {
        let (requests, receiver) = mpsc::channel(1);
        let (cancel, cancelled) = watch::channel(None);
        let (end, completed) = watch::channel(None);
        (
            SendStreamIo {
                requests,
                cancel,
                end: completed,
            },
            drive_send(self, receiver, cancelled, end),
        )
    }
}

async fn send_closed(stream: &mut SendStream) -> End<WriteError> {
    poll_fn(|cx| web_transport_trait::poll::SendStream::poll_closed(&mut **stream, cx))
        .await
        .map_err(StreamIoError::Transport)
}

async fn drive_send(
    mut stream: SendStream,
    mut requests: mpsc::Receiver<Write>,
    mut cancelled: watch::Receiver<Option<u32>>,
    end: watch::Sender<Option<End<WriteError>>>,
) {
    enum Event {
        Cancelled,
        Closed(End<WriteError>),
        Write(Option<Write>),
    }
    let terminal = loop {
        let event = if cancelled.borrow().is_some() {
            Event::Cancelled
        } else {
            tokio::select! { biased;
                _ = cancelled.changed() => Event::Cancelled,
                result = send_closed(&mut stream) => Event::Closed(result),
                request = requests.recv() => Event::Write(request),
            }
        };
        let request = match event {
            Event::Closed(result) => break result,
            Event::Cancelled | Event::Write(None) => {
                let _ = stream.reset(cancelled.borrow().unwrap_or(0));
                break Err(StreamIoError::Cancelled);
            }
            Event::Write(Some(request)) => request,
        };
        let finishing = request.bytes.is_none();
        let result = tokio::select! { biased;
            _ = cancelled.changed() => {
                let _ = stream.reset(cancelled.borrow().unwrap_or(0));
                Err(StreamIoError::Cancelled)
            }
            result = async {
                if let Some(bytes) = request.bytes {
                    stream.write_all(&bytes).await.map_err(StreamIoError::Transport)
                } else {
                    stream.finish().map_err(|_| StreamIoError::Transport(WriteError::ClosedStream))?;
                    send_closed(&mut stream).await
                }
            } => result,
        };
        let _ = request.reply.send(result.clone());
        if finishing || result.is_err() {
            break result;
        }
    };
    end.send_replace(Some(terminal.clone()));
    requests.close();
    while let Some(request) = requests.recv().await {
        let result = match &terminal {
            Ok(()) => Err(StreamIoError::Closed),
            Err(error) => Err(error.clone()),
        };
        let _ = request.reply.send(result);
    }
}

#[derive(Clone)]
pub struct RecvStreamIo {
    requests: mpsc::Sender<oneshot::Sender<ReadResult>>,
    cancel: watch::Sender<Option<u32>>,
    end: watch::Receiver<Option<End<ReadError>>>,
}

impl RecvStreamIo {
    pub async fn read(&self) -> ReadResult {
        let (reply, result) = oneshot::channel();
        if self.requests.send(reply).await.is_ok()
            && let Ok(result) = result.await
        {
            return result;
        }
        self.closed().await.map(|()| None)
    }

    pub fn stop(&self, code: u32) {
        cancel(&self.cancel, code);
    }

    pub fn closed(&self) -> impl Future<Output = End<ReadError>> + Send + 'static {
        wait_closed(self.end.clone())
    }
}

impl RecvStream {
    /// The driver observes RESET without reading or buffering application bytes.
    pub fn into_io(self) -> (RecvStreamIo, impl Future<Output = ()> + Send + 'static) {
        let (requests, receiver) = mpsc::channel(1);
        let (cancel, cancelled) = watch::channel(None);
        let (end, completed) = watch::channel(None);
        (
            RecvStreamIo {
                requests,
                cancel,
                end: completed,
            },
            drive_recv(self, receiver, cancelled, end),
        )
    }
}

async fn recv_closed(stream: &mut RecvStream) -> End<ReadError> {
    match stream.received_reset().await {
        Ok(Some(code)) => Err(StreamIoError::Transport(ReadError::Reset(code))),
        Err(error) => Err(StreamIoError::Transport(ReadError::SessionError(error))),
        Ok(None) => {
            // The SDK also returns None for a reset outside the HTTP/3 code
            // range. A terminal read distinguishes it from FIN. Quinn has
            // already removed the stream state or consumed all of its data.
            match stream.read(&mut [0; 1]).await {
                Ok(None) => Ok(()),
                Err(error) => Err(StreamIoError::Transport(error)),
                Ok(Some(_)) => Err(StreamIoError::Closed),
            }
        }
    }
}

async fn drive_recv(
    mut stream: RecvStream,
    mut requests: mpsc::Receiver<oneshot::Sender<ReadResult>>,
    mut cancelled: watch::Receiver<Option<u32>>,
    end: watch::Sender<Option<End<ReadError>>>,
) {
    enum Event {
        Cancelled,
        Closed(End<ReadError>),
        Read(Option<oneshot::Sender<ReadResult>>),
    }
    let terminal = loop {
        let event = if cancelled.borrow().is_some() {
            Event::Cancelled
        } else {
            tokio::select! { biased;
                _ = cancelled.changed() => Event::Cancelled,
                result = recv_closed(&mut stream) => Event::Closed(result),
                request = requests.recv() => Event::Read(request),
            }
        };
        let reply = match event {
            Event::Closed(result) => break result,
            Event::Cancelled | Event::Read(None) => {
                let _ = stream.stop(cancelled.borrow().unwrap_or(0));
                break Err(StreamIoError::Cancelled);
            }
            Event::Read(Some(reply)) => reply,
        };
        let mut bytes = vec![0; 64 * 1024];
        let result = tokio::select! { biased;
            _ = cancelled.changed() => {
                let _ = stream.stop(cancelled.borrow().unwrap_or(0));
                Err(StreamIoError::Cancelled)
            }
            result = stream.read(&mut bytes) => result.map(|length| length.map(|length| {
                bytes.truncate(length); bytes
            })).map_err(StreamIoError::Transport),
        };
        let terminal = match &result {
            Ok(Some(_)) => None,
            Ok(None) => Some(Ok(())),
            Err(error) => Some(Err(error.clone())),
        };
        let _ = reply.send(result);
        if let Some(terminal) = terminal {
            break terminal;
        }
    };
    end.send_replace(Some(terminal.clone()));
    requests.close();
    while let Some(reply) = requests.recv().await {
        let _ = reply.send(terminal.clone().map(|()| None));
    }
}
