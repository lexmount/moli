These are the complete browser and JavaScript CDP schemas from
[ChromeDevTools/devtools-protocol, revision 3bd8f01934b5cb08d4f2dd765a295c2b1a303f30](https://github.com/ChromeDevTools/devtools-protocol/tree/3bd8f01934b5cb08d4f2dd765a295c2b1a303f30/json)
(Chromium revision 1663043, Chrome 152.0.7954.0).

The JS schema matches `vendor/v8-152.2.0/v8/include/js_protocol.pdl`
(V8 15.2.124.1), converted with upstream `pdl.py` and
`map_binary_to_string=True`. The vendored `js_protocol-1.3.json` is the older
stable snapshot and does not describe all current Runtime commands and types.

Keep these files unchanged from upstream. When upgrading V8, update both from a
matching pinned protocol revision and recheck the JS schema against the vendored
PDL. Builds compact and embed them without a network dependency. The server
filters commands and events to its implementation, retaining all types for
cross-domain references. Moli-only definitions live in
`moli-protocol-server/src/protocol_server/protocol_extensions.json`.

Upstream licensing is preserved in `LICENSE`.
