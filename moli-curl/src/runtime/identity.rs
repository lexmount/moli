use std::{
    fmt,
    num::NonZeroUsize,
    sync::atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context, Result, anyhow};

static NEXT_CURL_TRANSFER_ID: AtomicUsize = AtomicUsize::new(1);

/// Opaque process-wide identity of one curl transfer.
///
/// The non-zero value is installed as libcurl's private token once the
/// transfer becomes active, so the same identity follows the request through
/// every residence and back in its terminal completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CurlTransferId(NonZeroUsize);

impl CurlTransferId {
    fn new(value: NonZeroUsize) -> Self {
        Self(value)
    }

    pub(crate) fn from_token(token: usize) -> Option<Self> {
        Some(Self::new(NonZeroUsize::new(token)?))
    }

    pub(crate) fn token(self) -> usize {
        self.0.get()
    }
}

impl fmt::Display for CurlTransferId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

pub(crate) fn next_transfer_id() -> Result<CurlTransferId> {
    let value = next_nonzero_usize(&NEXT_CURL_TRANSFER_ID)
        .context("curl transfer identity space exhausted")?;
    Ok(CurlTransferId::new(value))
}

fn next_nonzero_usize(counter: &AtomicUsize) -> Result<NonZeroUsize> {
    let sequence = counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| anyhow!("identity space exhausted"))?;
    NonZeroUsize::new(sequence).ok_or_else(|| anyhow!("identity must be non-zero"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_identity_uses_the_same_nonzero_value_as_its_token() {
        let transfer_id = CurlTransferId::from_token(7).expect("test token is non-zero");

        assert_eq!(transfer_id.token(), 7);
        assert_eq!(CurlTransferId::from_token(7), Some(transfer_id));
        assert_eq!(CurlTransferId::from_token(0), None);
    }

    #[test]
    fn transfer_sequence_never_wraps_or_reuses_zero() {
        let next = AtomicUsize::new(1);
        assert_eq!(next_nonzero_usize(&next).unwrap().get(), 1);
        assert_eq!(next_nonzero_usize(&next).unwrap().get(), 2);

        let exhausted = AtomicUsize::new(usize::MAX);
        assert!(next_nonzero_usize(&exhausted).is_err());
        assert_eq!(exhausted.load(Ordering::Relaxed), usize::MAX);
    }
}
