use std::ffi::CString;

use anyhow::{Context, Result};

// curl::easy::List only accepts UTF-8 strings. Own a libcurl list here so the
// transport can send ByteString header values without re-encoding them.
#[derive(Default)]
pub(crate) struct RequestHeaderList {
    raw: *mut curl_sys::curl_slist,
}

// SAFETY: the list has a single owner and no thread-affine state. Once attached,
// it moves with its Easy2 handler and is never mutated during a transfer.
unsafe impl Send for RequestHeaderList {}

impl RequestHeaderList {
    pub(crate) fn append(&mut self, line: &str) -> Result<()> {
        let line = if line.is_ascii() {
            CString::new(line.as_bytes())
        } else {
            let bytes = line
                .chars()
                .map(|ch| u8::try_from(u32::from(ch)))
                .collect::<std::result::Result<Vec<_>, _>>()
                .context("HTTP request header contains a non-byte character")?;
            CString::new(bytes)
        }
        .context("HTTP request header contains NUL")?;
        // SAFETY: self.raw is null or a live list owned by self. libcurl copies
        // the NUL-terminated string; failure leaves the existing list intact.
        let raw = unsafe { curl_sys::curl_slist_append(self.raw, line.as_ptr()) };
        if raw.is_null() {
            return Err(curl::Error::new(curl_sys::CURLE_OUT_OF_MEMORY).into());
        }
        self.raw = raw;
        Ok(())
    }

    pub(crate) fn as_ptr(&self) -> *mut curl_sys::curl_slist {
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
    fn rejects_non_byte_characters_and_nul() {
        let mut headers = RequestHeaderList::default();
        headers.append("X-Valid: \u{ff}").unwrap();
        assert!(headers.append("X-Invalid: \u{100}").is_err());
        assert!(headers.append("X-Invalid: before\0after").is_err());
        headers.append("X-Valid: after").unwrap();
    }
}
