//! Native connection observations. These are not Web-exposed timestamps.

use std::time::{Duration, Instant};

use curl::easy::{Easy2, Handler};
use moli_curl::CurlDnsTiming;

/// Connection setup observations owned by one transport attempt.
///
/// The shared resolver runs before libcurl. Its clock must not be inferred from
/// libcurl's name-lookup duration or from when the owner receives a completion.
/// Callback observations can be available before the terminal curl statistics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FetchConnectionTiming {
    /// The actual selected physical connection, including shared HTTP/2 setup.
    pub connection: Option<moli_curl::CurlConnectionTiming>,
    pub shared_dns: Option<CurlDnsTiming>,
    /// Last TLS context setup callback, when supported by the TLS backend.
    /// Absence does not establish that the request used plaintext transport.
    pub tls_context_started_at: Option<Instant>,
    /// Last prereq callback. Authentication can prepare several exchanges on
    /// one easy handle; the fetch's first request boundary is kept separately.
    pub request_prepared_at: Option<Instant>,
    pub curl: Option<FetchCurlTiming>,
}

/// Unrounded libcurl statistics, relative to libcurl's own transfer start.
///
/// These durations exclude Moli's preceding shared DNS lookup. A native zero is
/// retained as zero; an unavailable statistic is `None`. They are not durations
/// of individual phases, and must not be added to the logical fetch start.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FetchCurlTiming {
    pub name_lookup: Option<Duration>,
    pub connect: Option<Duration>,
    pub app_connect: Option<Duration>,
    pub pretransfer: Option<Duration>,
    pub total: Option<Duration>,
    /// New connections made by this transfer. Zero can also mean that setup
    /// failed before establishing a connection, rather than a reused socket.
    pub new_connections: Option<u64>,
}

impl FetchCurlTiming {
    pub(crate) fn from_easy<H: Handler>(easy: &Easy2<H>) -> Self {
        Self {
            name_lookup: easy.namelookup_time().ok(),
            connect: easy.connect_time().ok(),
            app_connect: easy.appconnect_time().ok(),
            pretransfer: easy.pretransfer_time().ok(),
            total: easy.total_time().ok(),
            new_connections: easy.num_connects().ok(),
        }
    }
}
