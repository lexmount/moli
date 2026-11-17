# IndexedDB value types

This crate contains the IndexedDB key ordering, depth checks, key paths, ranges, errors and existing version 1 binary codecs. The default build has no SQLite or service dependency. The SQL backend re-exports the same public types and continues using the same wire representation.

The backend enables `sqlite-errors` to preserve its existing `From<rusqlite::Error>` implementation. That implementation must live beside the error type under Rust's trait ownership rules. This optional conversion uses the existing `moli-sqlite` classifier and introduces no transaction, connection or service state. Web API bindings do not enable it.
