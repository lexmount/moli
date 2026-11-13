# Legacy HTTP/3 session interoperability

This is the published `web-transport-quinn` 0.13.5 archive, SHA-256
`49dd1992a7738bdabf673590e6b5bee753422504d08c9eec82c691a40abe51ad`.
Moli's source changes are confined to `src/connect.rs`, `src/session.rs`, and
`src/error.rs`.

The client accepts all successful CONNECT statuses and ignores a selected
optional application protocol that was not offered. Server-side protocol
selection validation is preserved. Together with the response decoder patch
in `web-transport-proto` 0.6.3, this matches the existing legacy `webtransport`
wire profile and Chromium 152; it does not enable the newer `webtransport-h3`
profile or its draft-16 negotiation requirements.

CONNECT capsule reading keeps clean EOF separate from parsing/read errors.
A clean FIN without a close capsule records remote code 0 and an empty reason;
malformed or interrupted capsule input retains a typed capsule failure and
closes this dedicated connection with H3_MESSAGE_ERROR. The first recorded
local or remote close cause continues to own teardown.
The cached capsule failure owns immutable diagnostic text so the SDK Session
retains its existing UnwindSafe and RefUnwindSafe properties.

References are HTTP/3 WebTransport draft-14
[session establishment and negotiation](https://www.ietf.org/archive/id/draft-ietf-webtrans-http3-14.html#section-3),
[session termination](https://www.ietf.org/archive/id/draft-ietf-webtrans-http3-14.html#section-6),
and the corresponding draft-16 rules for
[the newer profile](https://datatracker.ietf.org/doc/html/draft-ietf-webtrans-http3-16#section-3).
Real QUIC regression coverage is in `moli-webtransport/src/tests/http3.rs`.
