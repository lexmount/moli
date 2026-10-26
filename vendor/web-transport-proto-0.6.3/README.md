[![crates.io](https://img.shields.io/crates/v/web-transport-proto)](https://crates.io/crates/web-transport-proto)
[![discord](https://img.shields.io/discord/1124083992740761730)](https://discord.gg/FCYF3p99mr)

# web-transport-proto
The gritty WebTransport protocol implementation.
Not meant to be used directly, but as a dependency for the [Quinn](../web-transport-quinn), [Noq](../web-transport-noq), [Quiche](../web-transport-quiche), and [Iroh](../web-transport-iroh) backends.

CONNECT requests and responses accept a HEADERS payload and decoded field section
of at most 16 KiB. Decoded size counts every field line as its name and value
lengths plus 32 bytes, including repeated names. Literal and Huffman expansion
stop at the remaining budget, and an oversized declared HEADERS length is
refused before its payload is buffered. Encoding a request or response whose
field section exceeds the limit is refused before anything is written. All of
these paths report `ConnectError::FieldSectionTooLarge`.

`Settings::enable_webtransport` advertises this bound through
`MAX_FIELD_SECTION_SIZE`. SETTINGS, capsule, and GREASE frames retain their
64 KiB payload bound.
