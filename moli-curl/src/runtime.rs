mod config;
mod identity;
mod owner;
mod residence;

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Instant,
};

use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, Sender};
use curl::{
    easy::{Easy2, Handler},
    multi::MultiWaker,
};
use parking_lot::Mutex;

use crate::dns_adapter::CurlDnsResolution;

pub use config::CurlMultiRuntimeConfig;
pub use identity::CurlTransferId;

use identity::next_transfer_id;
use owner::{CurlRuntimeCommand, CurlRuntimeOwner};

/// Origin key used by the curl scheduler for per-origin active transfer caps.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CurlOriginKey {
    pub scheme: String,
    pub host: String,
    pub port: Option<u16>,
}

/// A configured curl transfer plus scheduler metadata.
pub struct CurlMultiJob<H: Handler, C> {
    pub easy: Easy2<H>,
    pub context: C,
    pub origin: Option<CurlOriginKey>,
    /// Absolute deadline for the whole scheduler-owned transfer attempt.
    ///
    /// libcurl cannot account for time spent in Moli's priority queue or in
    /// the shared DNS residence because both happen before the easy handle is
    /// added to the multi handle. The owner enforces this deadline in those
    /// residences and gives libcurl only the remaining duration.
    pub deadline: Option<Instant>,
    /// DNS ownership chosen by the caller before this transfer enters curl.
    ///
    /// A curl-managed policy preserves libcurl's resolver behavior. A shared
    /// origin policy parks the transfer outside the curl multi handle set until
    /// the bounded system resolver publishes an answer.
    pub dns_resolution: CurlDnsResolution,
    /// Higher values start before lower values when jobs are queued.
    pub priority: u8,
    pub label: String,
}

impl<H: Handler, C> fmt::Debug for CurlMultiJob<H, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CurlMultiJob")
            .field("origin", &self.origin)
            .field("deadline", &self.deadline)
            .field("dns_resolution", &self.dns_resolution)
            .field("priority", &self.priority)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// Completion emitted by `CurlMultiRuntime`.
pub struct CurlMultiCompletion<H: Handler, C> {
    pub transfer_id: CurlTransferId,
    pub easy: Option<Easy2<H>>,
    pub context: C,
    pub result: Result<()>,
}

impl<H: Handler, C> fmt::Debug for CurlMultiCompletion<H, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CurlMultiCompletion")
            .field("transfer_id", &self.transfer_id)
            .field("has_easy", &self.easy.is_some())
            .field("result", &self.result.as_ref().map(|_| ()))
            .finish_non_exhaustive()
    }
}

/// Error returned when a job cannot be submitted and is returned to the caller.
pub struct CurlSubmitError<H: Handler, C> {
    pub job: CurlMultiJob<H, C>,
    pub error: anyhow::Error,
}

impl<H: Handler, C> fmt::Debug for CurlSubmitError<H, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CurlSubmitError")
            .field("job", &self.job)
            .field("error", &self.error)
            .finish()
    }
}

/// Cloneable handle for a single libcurl multi owner thread.
#[derive(Debug)]
pub struct CurlMultiRuntime<H: Handler + Send + 'static, C: Send + 'static> {
    inner: Arc<CurlMultiRuntimeInner<H, C>>,
}

impl<H: Handler + Send + 'static, C: Send + 'static> Clone for CurlMultiRuntime<H, C> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

#[derive(Debug)]
struct CurlMultiRuntimeInner<H: Handler + Send + 'static, C: Send + 'static> {
    command_tx: Sender<CurlRuntimeCommand<H, C>>,
    owner_waker: MultiWaker,
    shutdown_requested: Arc<AtomicBool>,
    #[cfg(test)]
    owner_started: Arc<AtomicBool>,
    owner_handle: Mutex<Option<thread::JoinHandle<()>>>,
}

impl<H: Handler + Send + 'static, C: Send + 'static> CurlMultiRuntime<H, C> {
    pub fn new(
        config: CurlMultiRuntimeConfig,
    ) -> Result<(Self, Receiver<CurlMultiCompletion<H, C>>)> {
        config.validate()?;
        let (command_tx, command_rx) = crossbeam_channel::unbounded();
        let (completion_tx, completion_rx) = crossbeam_channel::unbounded();
        let (waker_tx, waker_rx) = crossbeam_channel::bounded(1);
        let shutdown_requested = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let owner_started = Arc::new(AtomicBool::new(false));
        let owner = CurlRuntimeOwner::new(
            config,
            command_rx,
            completion_tx,
            waker_tx,
            Arc::clone(&shutdown_requested),
            #[cfg(test)]
            Arc::clone(&owner_started),
        );
        let thread_name = owner.thread_name().to_owned();
        let owner_handle = thread::Builder::new()
            .name(thread_name)
            .spawn(move || owner.run())
            .context("failed to spawn curl multi runtime owner thread")?;
        let owner_waker = waker_rx
            .recv()
            .context("curl multi runtime owner did not publish a waker")?;
        let runtime = Self {
            inner: Arc::new(CurlMultiRuntimeInner {
                command_tx,
                owner_waker,
                shutdown_requested,
                #[cfg(test)]
                owner_started,
                owner_handle: Mutex::new(Some(owner_handle)),
            }),
        };
        Ok((runtime, completion_rx))
    }

    pub fn submit(
        &self,
        job: CurlMultiJob<H, C>,
    ) -> std::result::Result<CurlTransferId, CurlSubmitError<H, C>> {
        if self.inner.shutdown_requested.load(Ordering::SeqCst) {
            return Err(CurlSubmitError {
                job,
                error: anyhow!("curl multi runtime is shutting down"),
            });
        }
        let transfer_id = match next_transfer_id() {
            Ok(transfer_id) => transfer_id,
            Err(error) => return Err(CurlSubmitError { job, error }),
        };
        match self
            .inner
            .command_tx
            .send(CurlRuntimeCommand::Request { transfer_id, job })
        {
            Ok(()) => {
                let _ = self.inner.owner_waker.wakeup();
                Ok(transfer_id)
            }
            Err(error) => {
                let CurlRuntimeCommand::Request { job, .. } = error.into_inner() else {
                    unreachable!("submit only sends request commands");
                };
                Err(CurlSubmitError {
                    job,
                    error: anyhow!("curl multi runtime is shutting down"),
                })
            }
        }
    }

    pub fn shutdown(&self) {
        self.inner.shutdown();
    }

    #[cfg(test)]
    pub fn owner_count_for_testing(&self) -> usize {
        usize::from(self.inner.owner_started.load(Ordering::SeqCst))
    }
}

impl<H: Handler + Send + 'static, C: Send + 'static> Drop for CurlMultiRuntimeInner<H, C> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl<H: Handler + Send + 'static, C: Send + 'static> CurlMultiRuntimeInner<H, C> {
    fn shutdown(&self) {
        if self.shutdown_requested.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.command_tx.send(CurlRuntimeCommand::Shutdown);
        let _ = self.owner_waker.wakeup();
        let Some(owner_handle) = self.owner_handle.lock().take() else {
            return;
        };
        let _ = owner_handle.join();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::Duration,
    };

    use super::*;

    #[derive(Debug)]
    struct TestHandler;

    impl Handler for TestHandler {}

    #[test]
    fn submitted_identity_reaches_the_matching_runtime_completion() {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .expect("test HTTP listener should bind to a local port");
        let address = listener
            .local_addr()
            .expect("test HTTP listener should have an address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("curl should connect to the test HTTP listener");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("test HTTP connection should accept a read timeout");
            let mut request = [0; 4096];
            let _ = stream
                .read(&mut request)
                .expect("test HTTP request should be readable");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .expect("test HTTP response should be writable");
        });

        let (runtime, completion_rx) = CurlMultiRuntime::new(CurlMultiRuntimeConfig {
            poll_interval: Duration::from_millis(5),
            ..CurlMultiRuntimeConfig::default()
        })
        .expect("test curl runtime should start");
        let mut easy = Easy2::new(TestHandler);
        easy.url(&format!("http://{address}/identity"))
            .expect("test curl URL should be valid");
        let transfer_id = runtime
            .submit(CurlMultiJob {
                easy,
                context: "matching-context".to_owned(),
                origin: None,
                deadline: None,
                dns_resolution: CurlDnsResolution::curl_managed(),
                priority: 1,
                label: "identity-test".to_owned(),
            })
            .expect("test curl transfer should be accepted");

        let completion = completion_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("test curl transfer should reach terminal completion");
        assert_eq!(completion.transfer_id, transfer_id);
        assert_eq!(completion.context, "matching-context");
        assert!(completion.easy.is_some());
        completion
            .result
            .expect("test curl transfer should complete successfully");

        runtime.shutdown();
        server.join().expect("test HTTP server should finish");
    }
}
