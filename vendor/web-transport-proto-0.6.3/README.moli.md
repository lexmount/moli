# Legacy HTTP/3 response metadata

This is the published `web-transport-proto` 0.6.3 archive, SHA-256
`d2baf87f7a4701d964fd23529b440db3da535b1d7539289f7a463c3481dafcff`.
The source change in `src/connect.rs` ignores an invalid optional `WT-Protocol`
Structured Field while retaining status, QPACK, framing and size validation.

Moli currently uses the legacy `webtransport` wire profile, as does the pinned
Chromium 152/WPT comparison. This follows
[HTTP/3 WebTransport draft-14 section 3.3](https://www.ietf.org/archive/id/draft-ietf-webtrans-http3-14.html#section-3.3).
Draft-16 introduces the `webtransport-h3` token and a different rule for
unoffered protocols; this patch does not negotiate that newer wire profile.

Real QUIC regression coverage is in `moli-webtransport/src/tests/http3.rs`.
Keep this dependency patch synchronized with the client-side patch described
in `vendor/web-transport-quinn-0.13.5/README.moli.md`.
