//! HTTP submission types and owner-local request scheduling.
//! The shared runtime drives libcurl; this module owns HTTP jobs until completion.

pub(crate) mod registry;
mod scheduling;

use crate::{
    CurlDnsResolution, CurlTransferId,
    runtime::{CurlRuntimeCommand, identity::next_transfer_id},
};
use anyhow::{Result, anyhow};
use crossbeam_channel::Sender;
use curl::{
    easy::{Easy2, Handler},
    multi::MultiWaker,
};
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

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

/// Submits HTTP work to one owner. Clones carry no shutdown or join authority.
#[derive(Debug)]
pub struct CurlHttpSender<H: Handler + Send + 'static, C: Send + 'static> {
    pub(crate) command_tx: Sender<CurlRuntimeCommand<H, C>>,
    pub(crate) owner_waker: MultiWaker,
    pub(crate) shutdown_requested: Arc<AtomicBool>,
}

impl<H: Handler + Send + 'static, C: Send + 'static> Clone for CurlHttpSender<H, C> {
    fn clone(&self) -> Self {
        Self {
            command_tx: self.command_tx.clone(),
            owner_waker: self.owner_waker.clone(),
            shutdown_requested: self.shutdown_requested.clone(),
        }
    }
}

impl<H: Handler + Send + 'static, C: Send + 'static> CurlHttpSender<H, C> {
    pub fn submit(
        &self,
        job: CurlMultiJob<H, C>,
    ) -> std::result::Result<CurlTransferId, CurlSubmitError<H, C>> {
        if self.shutdown_requested.load(Ordering::SeqCst) {
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
            .command_tx
            .send(CurlRuntimeCommand::Request { transfer_id, job })
        {
            Ok(()) => {
                let _ = self.owner_waker.wakeup();
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
}
