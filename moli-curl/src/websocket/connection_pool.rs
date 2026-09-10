//! A separate connection cache for persistent WebSockets on the shared Multi.
//! HTTP pool limits continue to apply to HTTP connections. WebSocket admission
//! bounds this cache instead. Everything using it stays on the owner thread.
use std::{ptr::NonNull, rc::Rc};

use anyhow::{Context, Result, bail};
use curl::easy::Easy2;

use super::request::Handshake;

pub(super) struct ConnectionPool(NonNull<curl_sys::CURLSH>);

impl ConnectionPool {
    pub(super) fn new() -> Result<Rc<Self>> {
        // Multi has already initialized libcurl. The pointer is owned here and
        // is never sent across threads; each bound handler retains an Rc.
        let pool = Self(
            NonNull::new(unsafe { curl_sys::curl_share_init() })
                .context("failed to create WebSocket connection cache")?,
        );
        let result = unsafe {
            curl_sys::curl_share_setopt(
                pool.0.as_ptr(),
                curl_sys::CURLSHOPT_SHARE,
                curl_sys::CURL_LOCK_DATA_CONNECT,
            )
        };
        if result != curl_sys::CURLSHE_OK {
            bail!("failed to configure WebSocket connection cache: {result}");
        }
        Ok(Rc::new(pool))
    }

    pub(super) fn bind(self: &Rc<Self>, easy: &mut Easy2<Handshake>) -> Result<()> {
        // CURLOPT_SHARE borrows the pointer. Easy2 runs curl_easy_cleanup
        // before dropping its Handler, whose Rc therefore outlives that borrow.
        let result = unsafe {
            curl_sys::curl_easy_setopt(easy.raw(), curl_sys::CURLOPT_SHARE, self.0.as_ptr())
        };
        if result != curl_sys::CURLE_OK {
            return Err(curl::Error::new(result).into());
        }
        easy.get_mut().connection_pool = Some(self.clone());
        Ok(())
    }
}

impl Drop for ConnectionPool {
    fn drop(&mut self) {
        // All easy handles borrowing the share have already been cleaned up.
        let result = unsafe { curl_sys::curl_share_cleanup(self.0.as_ptr()) };
        debug_assert_eq!(result, curl_sys::CURLSHE_OK);
    }
}
