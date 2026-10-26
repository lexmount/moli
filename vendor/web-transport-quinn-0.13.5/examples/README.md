# Example

## Simple Echo
A simple [server](echo-server.rs) and [client](echo-client.rs).

There's also advanced examples [server](echo-server-advanced.rs) and [client](echo-client-advanced.rs) that construct the QUIC connection manually.

QUIC requires TLS, which makes the initial setup a bit more involved.

-   Generate a certificate: `../dev/setup`
-   Run the Rust server: `cargo run --example echo-server -- --tls-cert ../dev/localhost.crt --tls-key ../dev/localhost.key`
-   Run the Rust client: `cargo run --example echo-client -- --tls-cert ../dev/localhost.crt`
-   Run a Web client: `cd ../web-demo; npm install; npx parcel serve client.html --open`

If you get a certificate error with the web client, try deleting `.parcel-cache`.
