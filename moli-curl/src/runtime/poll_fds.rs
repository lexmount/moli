//! Snapshot a second Multi's interests for the owner's single native poll.

use anyhow::Result;
use curl::multi::{Multi, WaitFd};

#[derive(Default)]
pub(super) struct MultiPollFds {
    fds: Vec<curl_sys::curl_waitfd>,
}

impl MultiPollFds {
    pub(super) fn refresh(&mut self, multi: &Multi) -> Result<()> {
        self.fds.clear();
        let mut required = 0;
        // Both queries run on the Multi's owner, without a perform or handle
        // mutation between them. The vendored libcurl supplies this portable
        // API without fd_set's descriptor-number limit.
        let result = unsafe {
            curl_sys::curl_multi_waitfds(multi.raw(), std::ptr::null_mut(), 0, &mut required)
        };
        if result != curl_sys::CURLM_OK {
            return Err(curl::MultiError::new(result).into());
        }
        if required == 0 {
            return Ok(());
        }
        self.fds
            .resize_with(required as usize, || curl_sys::curl_waitfd {
                fd: curl_sys::CURL_SOCKET_BAD,
                events: 0,
                revents: 0,
            });
        let result = unsafe {
            curl_sys::curl_multi_waitfds(
                multi.raw(),
                self.fds.as_mut_ptr(),
                required,
                &mut required,
            )
        };
        if result != curl_sys::CURLM_OK {
            self.fds.clear();
            return Err(curl::MultiError::new(result).into());
        }
        self.fds.truncate(required as usize);
        Ok(())
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = WaitFd> + '_ {
        self.fds.iter().map(|native| {
            let mut fd = WaitFd::new();
            fd.set_fd(native.fd);
            fd.poll_on_read(native.events & curl_sys::CURL_WAIT_POLLIN != 0)
                .poll_on_priority_read(native.events & curl_sys::CURL_WAIT_POLLPRI != 0)
                .poll_on_write(native.events & curl_sys::CURL_WAIT_POLLOUT != 0);
            fd
        })
    }
}
