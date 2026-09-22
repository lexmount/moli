use crate::headers::{decode_http_header_bytes, parse_http_response_header_line};

const MAX_PROXY_CONNECT_HEADER_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProxyConnectResponse {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, Vec<u8>)>,
}

#[derive(Debug, Default)]
pub(crate) struct ProxyConnectResponseRecorder {
    enabled: bool,
    collecting: bool,
    pending_status: Option<u16>,
    pending_headers: Vec<(String, Vec<u8>)>,
    observed_bytes: usize,
    completed: Option<ProxyConnectResponse>,
}

impl ProxyConnectResponseRecorder {
    pub(crate) fn begin_transfer(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.collecting = false;
        self.pending_status = None;
        self.pending_headers.clear();
        self.observed_bytes = 0;
        self.completed = None;
    }

    /// Returns whether this block is a proxy CONNECT request and therefore must
    /// not be recorded as the target resource request.
    pub(crate) fn record_outgoing_header_block(&mut self, data: &[u8]) -> bool {
        if !is_connect_request_header_block(data) {
            return false;
        }
        self.collecting = self.enabled;
        self.pending_status = None;
        self.pending_headers.clear();
        self.observed_bytes = 0;
        self.completed = None;
        true
    }

    pub(crate) fn record_incoming_header_line(&mut self, data: &[u8]) {
        if !self.collecting {
            return;
        }
        self.observed_bytes = self.observed_bytes.saturating_add(data.len());
        if self.observed_bytes > MAX_PROXY_CONNECT_HEADER_BYTES {
            self.collecting = false;
            self.pending_status = None;
            self.pending_headers.clear();
            return;
        }

        let line = decode_http_header_bytes(data);
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(status) = parse_status_line(line) {
            self.pending_status = Some(status);
            self.pending_headers.clear();
            return;
        }
        if line.is_empty() {
            self.collecting = false;
            self.completed = self
                .pending_status
                .take()
                .map(|status| ProxyConnectResponse {
                    status,
                    headers: std::mem::take(&mut self.pending_headers),
                });
            return;
        }
        if let Some(header) = parse_http_response_header_line(data) {
            self.pending_headers.push(header);
        }
    }

    pub(crate) fn take_failed_response(
        &mut self,
        connect_status: u32,
    ) -> Option<ProxyConnectResponse> {
        let response = self.completed.take()?;
        (response.status == connect_status as u16 && !(200..300).contains(&response.status))
            .then_some(response)
    }
}

fn is_connect_request_header_block(data: &[u8]) -> bool {
    let first_line = data.split(|byte| *byte == b'\n').next().unwrap_or_default();
    let first_line = first_line.strip_suffix(b"\r").unwrap_or(first_line);
    first_line
        .split(|byte| byte.is_ascii_whitespace())
        .next()
        .is_some_and(|method| method.eq_ignore_ascii_case(b"CONNECT"))
}

fn parse_status_line(line: &str) -> Option<u16> {
    line.strip_prefix("HTTP/")?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_connect_failure_without_treating_target_headers_as_connect_headers() {
        let mut recorder = ProxyConnectResponseRecorder::default();
        recorder.begin_transfer(true);
        assert!(recorder.record_outgoing_header_block(
            b"CONNECT example.test:443 HTTP/1.1\r\nHost: example.test:443\r\n\r\n"
        ));
        recorder.record_incoming_header_line(b"HTTP/1.1 407 Proxy Authentication Required\r\n");
        recorder.record_incoming_header_line(b"Proxy-Authenticate: Basic realm=\"\xff\"\r\n");
        recorder.record_incoming_header_line(b"\r\n");

        assert!(
            !recorder.record_outgoing_header_block(
                b"GET /resource HTTP/1.1\r\nHost: example.test\r\n\r\n"
            )
        );
        recorder.record_incoming_header_line(b"HTTP/1.1 200 OK\r\n");
        recorder.record_incoming_header_line(b"X-Target: response\r\n");
        recorder.record_incoming_header_line(b"\r\n");

        assert_eq!(
            recorder.take_failed_response(407),
            Some(ProxyConnectResponse {
                status: 407,
                headers: vec![(
                    "Proxy-Authenticate".to_owned(),
                    b"Basic realm=\"\xff\"".to_vec(),
                )],
            })
        );
    }

    #[test]
    fn disabled_capture_still_identifies_connect_request_blocks() {
        let mut recorder = ProxyConnectResponseRecorder::default();
        recorder.begin_transfer(false);
        assert!(
            recorder.record_outgoing_header_block(b"CONNECT example.test:443 HTTP/1.1\r\n\r\n")
        );
        recorder.record_incoming_header_line(b"HTTP/1.1 407 Proxy Authentication Required\r\n");
        recorder.record_incoming_header_line(b"\r\n");
        assert_eq!(recorder.take_failed_response(407), None);
    }
}
