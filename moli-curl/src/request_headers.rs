use std::ffi::CString;

use anyhow::{Context, Result};

/// An owned libcurl header list that accepts encoded bytes.
///
/// `curl::easy::List` only accepts UTF-8 strings. Retain this list in the Easy2
/// handler so it outlives transfers that borrow its native pointer.
#[derive(Default)]
pub struct RequestHeaderList {
    raw: *mut curl_sys::curl_slist,
}

// SAFETY: the list has a single owner and no thread-affine state. Once attached,
// it moves with its Easy2 handler and is never mutated during a transfer.
unsafe impl Send for RequestHeaderList {}

impl RequestHeaderList {
    pub fn append(&mut self, line: &[u8]) -> Result<()> {
        let line = CString::new(line).context("HTTP request header contains NUL")?;
        // SAFETY: self.raw is null or a live list owned by self. libcurl copies
        // the NUL-terminated string; failure leaves the existing list intact.
        let raw = unsafe { curl_sys::curl_slist_append(self.raw, line.as_ptr()) };
        if raw.is_null() {
            return Err(curl::Error::new(curl_sys::CURLE_OUT_OF_MEMORY).into());
        }
        self.raw = raw;
        Ok(())
    }

    pub fn as_ptr(&self) -> *mut curl_sys::curl_slist {
        self.raw
    }
}

impl Drop for RequestHeaderList {
    fn drop(&mut self) {
        // SAFETY: this is the sole owner, and Easy2 cleans up the curl handle
        // before dropping its handler and this list.
        unsafe { curl_sys::curl_slist_free_all(self.raw) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_nul_without_discarding_the_list() {
        let mut headers = RequestHeaderList::default();
        headers.append(b"X-Valid: \xff").unwrap();
        assert!(headers.append(b"X-Invalid: before\0after").is_err());
        headers.append(b"X-Valid: after").unwrap();
    }
}
