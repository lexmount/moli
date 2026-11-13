These are the complete browser and JavaScript CDP schemas from
[ChromeDevTools/devtools-protocol, revision 3bd8f01934b5cb08d4f2dd765a295c2b1a303f30](https://github.com/ChromeDevTools/devtools-protocol/tree/3bd8f01934b5cb08d4f2dd765a295c2b1a303f30/json)
(Chromium revision 1663043, Chrome 152.0.7954.0).

The JS schema matches `vendor/v8-152.2.0/v8/include/js_protocol.pdl`
(V8 15.2.124.1), converted with upstream `pdl.py` and
`map_binary_to_string=True`. The vendored `js_protocol-1.3.json` is the older
stable snapshot and does not describe all current Runtime commands and types.

Keep these files unchanged from upstream. When upgrading V8, update both from a
matching pinned protocol revision and recheck the JS schema against the vendored
PDL. The build merges `protocol_extensions.json`, selects commands and events
using `moli-protocol-cdp-defs`, and validates definitions, type references, and
domain dependencies. The dispatcher uses the same action enum types as this
catalog. Browser events and V8 forwarding rules are explicit static lists;
adding an emitted event still requires updating the catalog.

All types remain available for cross-domain references. The final advertised
schema is compacted and compressed with the existing `brotlic` dependency at
quality 11. Only the `.json.br` bytes are embedded, without a network dependency.
The server decodes this final response once through `BufReader` and caches it
with `OnceLock`; it does not parse, merge, or filter schema JSON at runtime. The
uncompressed build artifact is used only by the byte-for-byte round-trip test.

Upstream licensing is preserved in `LICENSE`.
