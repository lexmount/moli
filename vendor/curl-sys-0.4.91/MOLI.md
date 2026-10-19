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
