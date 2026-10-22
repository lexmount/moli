# Moli changes to urlpattern 0.6.0

Upstream: https://github.com/denoland/rust-urlpattern
Published crate SHA256: df16f50ef4cc145211879a3867ba757076b25dfee812040dcb0658bd9ae7904b

Add browser entry points that accept a scoped special-scheme matcher while parsing constructor strings and compiling components. Existing Rust entry points keep their matcher and behavior.

The renderer supplies its existing bounded V8 RegExp implementation to decide whether a protocol pattern matches a special URL scheme. Final components also use V8. This avoids a second JavaScript regex engine, unbounded native backtracking, and thread-local V8 scope pointers. No constructor allocation or WebIDL timing mechanism is changed.

Constructor protocol expressions use the regex syntax declared by their backend. Three signatures make elided Token lifetimes explicit for the current Rust compiler. Other vendored Rust source retains upstream behavior; fixtures and license are unchanged. The upstream rustfmt configuration is included.
