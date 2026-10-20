# Moli libcurl transport

Based on curl-sys 0.4.91 and libcurl 8.23 from
[lexmount/curl-rust 0392e12](https://github.com/lexmount/curl-rust/tree/0392e12475d8d1160d4cae07c172cac02797cc25/curl-sys).
The Rust curl wrapper remains at that same revision.

This copy preserves Moli's browser TLS EOF behavior across the upgrade:
accept actual transport EOF after the TLS handshake, while rejecting incomplete
HTTPS headers. TLS alerts, socket errors, certificate verification and HTTP/1
and HTTP/2 body framing remain enforced. Regression coverage lives in
`moli-curl/tests/tls_eof.rs`.

Only the native library and public headers are included. The build script does
not initialize Git submodules. On upgrades, port the three documented changes
in `curl/lib/http.c` and `curl/lib/vtls/openssl.c` and run the transport tests.

Keep the pinned main browser-response-headers feature and HTTP/1 parser source.
The remaining native source matches that revision except the three TLS EOF
changes described above.

Moli also retains the selected physical connection's timing in
`curl/moli_timing.h`. This private extension is available with the bundled
static library. It accepts an optional process-lifetime monotonic clock, used
at socket creation, successful TCP connection and TLS handshake boundaries;
ordinary libcurl callers do not install that clock and obtain no observations.
The winning socket keeps the originating shared-DNS timestamps. Connection
reuse preserves those original observations, including for concurrent HTTP/2
streams. Each request copies its selected connection before dispatch so closing
or evicting the connection cannot erase the result.

The clock callback must not unwind or retain per-request state. Moli's callback
only reads a process-lifetime Rust `Instant` origin; no cross-clock calibration,
curl duration arithmetic or completion-delivery timestamp is involved. The
extension changes neither transport decisions nor standard CURLINFO results.
On upgrades, port `moli_timing.c`, the connection/transfer carriers and the
observation sites in `cf-socket.c`, `vtls.c`, `multi.c`, `progress.c` and
`getinfo.c`. Native regression coverage includes keep-alive reuse and close,
DNS provenance, concurrent HTTP/2 streams and proxy CONNECT/TLS ordering.
