//! Observations on the physical connection selected by libcurl.

use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

use curl::easy::{Easy2, Handler};

use crate::CurlDnsTiming;

static CLOCK_ORIGIN: OnceLock<Instant> = OnceLock::new();

/// Native connection events, including the provenance of its initial lookup.
/// All events use Rust's monotonic clock at the actual socket/TLS boundary;
/// libcurl durations and completion delivery time are not clock anchors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CurlConnectionTiming {
    pub domain_lookup: Option<CurlDnsTiming>,
    pub connect_started_at: Instant,
    pub connect_completed_at: Instant,
    pub secure_started_at: Option<Instant>,
}

// The callback has process lifetime, does not allocate and cannot unwind. Zero
// is reserved for absent observations in native callers which did not opt in.
extern "C" fn monotonic_now() -> curl_sys::curl_off_t {
    CLOCK_ORIGIN.get().map_or(0, |origin| {
        origin.elapsed().as_nanos().min((i64::MAX - 1) as u128) as i64 + 1
    })
}

fn encode(origin: Instant, value: Instant) -> Option<i64> {
    let offset = if value >= origin {
        i64::try_from(value.duration_since(origin).as_nanos()).ok()?
    } else {
        -i64::try_from(origin.duration_since(value).as_nanos()).ok()?
    };
    offset.checked_add(1)
}

fn decode(origin: Instant, value: i64) -> Option<Instant> {
    let offset = value.checked_sub(1)?;
    if offset >= 0 {
        origin.checked_add(Duration::from_nanos(offset as u64))
    } else {
        origin.checked_sub(Duration::from_nanos(offset.unsigned_abs()))
    }
}

pub(crate) fn initialize<H: Handler>(easy: &mut Easy2<H>, dns: Option<CurlDnsTiming>) {
    let origin = *CLOCK_ORIGIN.get_or_init(Instant::now);
    let dns = dns.and_then(|dns| {
        Some((
            encode(origin, dns.started_at)?,
            encode(origin, dns.completed_at)?,
        ))
    });
    let (start, end) = dns.unwrap_or_default();
    // SAFETY: the easy handle is exclusively owned outside the Multi. The
    // static callback has no userdata, and DNS values are copied, not borrowed.
    let result = unsafe {
        curl_sys::curl_easy_moli_timing_init(
            easy.raw(),
            Some(monotonic_now),
            start,
            end,
            i32::from(dns.is_some()),
        )
    };
    assert_eq!(
        result,
        curl_sys::CURLE_OK,
        "valid easy handle accepts native timing initialization"
    );
}

impl CurlConnectionTiming {
    /// Read the connection snapshot retained before request dispatch. This
    /// remains available after the connection closed or left the pool.
    pub fn from_easy<H: Handler>(easy: &Easy2<H>) -> Option<Self> {
        // SAFETY: the caller owns the live easy handle.
        unsafe { Self::from_raw_handle(easy.raw()) }
    }

    /// Read the same snapshot inside a native pre-request callback.
    ///
    /// # Safety
    /// `easy` must be live and owned by this thread. During a callback it must
    /// be the exact handle invoking that callback. This function only copies
    /// the native record; it does not run libcurl or invoke any callbacks.
    pub unsafe fn from_raw_handle(easy: *mut curl_sys::CURL) -> Option<Self> {
        let origin = *CLOCK_ORIGIN.get()?;
        let mut raw = curl_sys::curl_moli_connection_timing::default();
        // SAFETY: getinfo-style read on the caller-owned handle; the native
        // function only copies a fixed-size value into this writable record.
        let result = unsafe { curl_sys::curl_easy_moli_connection_timing(easy, &mut raw) };
        if result != curl_sys::CURLE_OK {
            return None;
        }
        let connect_started_at = decode(origin, raw.connect_start)?;
        let connect_completed_at = decode(origin, raw.connect_end)?;
        if connect_completed_at < connect_started_at {
            return None;
        }
        Some(Self {
            domain_lookup: if raw.has_dns != 0 {
                Some(CurlDnsTiming {
                    started_at: decode(origin, raw.dns_start)?,
                    completed_at: decode(origin, raw.dns_end)?,
                })
            } else {
                None
            },
            connect_started_at,
            connect_completed_at,
            secure_started_at: if raw.secure_start != 0 {
                Some(decode(origin, raw.secure_start)?)
            } else {
                None
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[derive(Default)]
    struct Sink;
    impl Handler for Sink {
        fn write(&mut self, data: &[u8]) -> Result<usize, curl::easy::WriteError> {
            Ok(data.len())
        }
    }

    #[test]
    fn selected_connection_keeps_its_clock_and_dns_through_reuse_and_close() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            for close in [false, true] {
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.extend_from_slice(&byte);
                }
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: {}\r\n\r\n",
                    if close { "close" } else { "keep-alive" }
                )
                .unwrap();
            }
        });
        let mut easy = Easy2::new(Sink);
        easy.proxy("").unwrap();
        easy.url(&url).unwrap();
        let before = Instant::now();
        let dns = CurlDnsTiming {
            started_at: before - Duration::from_millis(1),
            completed_at: before,
        };
        initialize(&mut easy, Some(dns));
        easy.perform().unwrap();
        let first = CurlConnectionTiming::from_easy(&easy).unwrap();
        assert_eq!(first.domain_lookup, Some(dns));
        assert!(before <= first.connect_started_at);
        assert!(first.connect_started_at <= first.connect_completed_at);
        assert!(first.connect_completed_at <= Instant::now());
        assert!(first.secure_started_at.is_none());
        let later = Instant::now();
        initialize(
            &mut easy,
            Some(CurlDnsTiming {
                started_at: later,
                completed_at: later,
            }),
        );
        assert!(CurlConnectionTiming::from_easy(&easy).is_none());
        easy.perform().unwrap();
        assert_eq!(easy.num_connects().unwrap(), 0);
        assert_eq!(CurlConnectionTiming::from_easy(&easy), Some(first));
        server.join().unwrap();
        initialize(&mut easy, None);
        assert!(CurlConnectionTiming::from_easy(&easy).is_none());
    }
}
